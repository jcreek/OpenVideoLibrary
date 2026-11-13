use crate::{
    App,
    api::item_row,
    auth::User,
    error::{Error, Result},
    media,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};

#[derive(Default)]
pub(crate) struct Tags {
    title: String,
    artist: String,
    album: String,
    disc: i64,
    track: i64,
}

/// Outcome of one scan-time tag probe.
pub(crate) enum Probe {
    Tags(Tags),
    /// Every inspection slot was in use; nothing was attempted.
    Busy,
    /// The tool was unavailable or the server is stopping; nothing about the
    /// track was learned, so it is not recorded.
    Transient,
    /// The track could not be probed. It is recorded and skipped until the
    /// retry delay passes or the file changes: [`PROBE_RETRY`] when the track
    /// itself is unusable (permission, symlink, not a regular file, no
    /// readable audio), [`PROBE_RETRY_SOON`] for timeouts and I/O errors that
    /// may clear.
    Failed(Duration),
}

pub(crate) async fn inspect(app: &Arc<App>, path: &Path) -> Probe {
    let Ok(slot) = app.inspection.clone().try_acquire_owned() else {
        return Probe::Busy;
    };
    read_tags(app, path, slot).await
}

fn permanent(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR))
}

fn retry_after(permanent: bool) -> Probe {
    Probe::Failed(if permanent {
        PROBE_RETRY
    } else {
        PROBE_RETRY_SOON
    })
}

async fn read_tags(app: &Arc<App>, path: &Path, slot: tokio::sync::OwnedSemaphorePermit) -> Probe {
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let format = match extension.as_str() {
        "mp3" => "mp3",
        "flac" => "flac",
        "ogg" | "opus" => "ogg",
        "wav" => "wav",
        "m4a" => "mov",
        _ => return Probe::Failed(PROBE_RETRY),
    };
    let path = path.to_owned();
    let work = app.tasks.spawn_blocking(move || {
        let file = media::open_registered_file(&path).map_err(|e| permanent(&e))?;
        if !file.metadata().map_err(|e| permanent(&e))?.is_file() {
            return Err(true);
        }
        Ok((file, slot))
    });
    let (file, slot) = match tokio::time::timeout(Duration::from_secs(5), work).await {
        Ok(Ok(Ok(opened))) => opened,
        Ok(Ok(Err(permanent))) => return retry_after(permanent),
        // Open timeout, or a blocking task that could not run.
        _ => return retry_after(false),
    };
    let data = match media::probe_music_tags(app, file, slot, format).await {
        Ok(data) => data,
        Err(e) if e.0 == axum::http::StatusCode::UNPROCESSABLE_ENTITY => {
            return Probe::Tags(Tags::default());
        }
        // Missing tool or stopping server: says nothing about the track.
        Err(e) if [503, 499].contains(&e.0.as_u16()) => return Probe::Transient,
        Err(_) => return retry_after(false),
    };
    match parse_tags(&data) {
        Some(tags) => Probe::Tags(tags),
        None => Probe::Failed(PROBE_RETRY),
    }
}

fn parse_tags(data: &[u8]) -> Option<Tags> {
    let info: serde_json::Value = serde_json::from_slice(data).ok()?;
    let streams = info["streams"].as_array()?;
    if streams.iter().any(|stream| {
        stream["codec_type"] == "video"
            && stream["disposition"]["attached_pic"] != 1
            && stream["disposition"]["timed_thumbnails"] != 1
    }) {
        return Some(Tags::default());
    }
    let stream = streams.iter().find(|s| s["codec_type"] == "audio")?;
    let field = |key: &str| {
        [&info["format"]["tags"], &stream["tags"]]
            .into_iter()
            .filter_map(serde_json::Value::as_object)
            .flat_map(|group| group.iter())
            .find_map(|(k, v)| k.eq_ignore_ascii_case(key).then(|| v.as_str()).flatten())
            .unwrap_or_default()
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>()
    };
    let number = |key| {
        field(key)
            .split('/')
            .next()
            .and_then(|n| n.parse::<i64>().ok())
            .filter(|n| (1..=9999).contains(n))
            .unwrap_or(0)
    };
    Some(Tags {
        title: field("title"),
        artist: {
            let value = field("album_artist");
            if value.is_empty() {
                field("artist")
            } else {
                value
            }
        },
        album: field("album"),
        disc: number("disc"),
        track: number("track"),
    })
}

pub(crate) fn save(
    tx: &rusqlite::Transaction,
    path: &str,
    mtime: i64,
    size: i64,
    tags: Tags,
) -> Result<()> {
    tx.execute(
        "DELETE FROM ovl_music_probe_failures WHERE file_id=(SELECT id FROM media_files WHERE path=?)",
        [path],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO ovl_music_tags(file_id,mtime,size,title,artist,album,disc,track)
                SELECT id,?,?,?,?,?,?,? FROM media_files WHERE path=?",
        params![
            mtime,
            size,
            tags.title,
            tags.artist,
            tags.album,
            tags.disc,
            tags.track,
            path
        ],
    )?;
    Ok(())
}

/// Records a failed probe so later scans spend their budget on other tracks.
/// The track is retried at `retry_at` (Unix seconds) or when the file changes.
pub(crate) fn save_failure(
    tx: &rusqlite::Transaction,
    path: &str,
    mtime: i64,
    size: i64,
    retry_at: i64,
) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO ovl_music_probe_failures(file_id,mtime,size,retry_at)
                SELECT id,?,?,? FROM media_files WHERE path=?",
        params![mtime, size, retry_at, path],
    )?;
    Ok(())
}

/// How long a track that cannot be probed is skipped.
pub(crate) const PROBE_RETRY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a track whose probe timed out or hit an I/O error is skipped.
pub(crate) const PROBE_RETRY_SOON: Duration = Duration::from_secs(60 * 60);

#[derive(Deserialize)]
pub struct AlbumQuery {
    library: Option<i64>,
    artist: Option<String>,
    #[serde(default)]
    q: String,
    #[serde(default)]
    offset: i64,
}

const CURRENT: &str = " FROM ovl_music_tags t JOIN media_files f ON f.id=t.file_id
    JOIN ovl_music_libraries m ON m.library_id=f.library_id";
const FRESH: &str = "t.mtime=f.mtime AND t.size=f.size";
const GRANT: &str =
    "(? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=f.library_id AND a.user_id=?))";

pub async fn albums(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Query(q): Query<AlbumQuery>,
) -> Result<Json<Value>> {
    if q.q.len() > 800
        || q.artist.as_ref().is_some_and(|a| a.len() > 800)
        || !(0..=100_000).contains(&q.offset)
    {
        return Err(Error::new(400, "invalid music album query"));
    }
    Ok(Json(app.db.call(move|c| {
        let sql=format!("SELECT f.library_id,t.artist,t.album,count(*) {CURRENT}
            WHERE {FRESH} AND {GRANT} AND t.album<>'' AND (? IS NULL OR f.library_id=?)
            AND (? IS NULL OR t.artist=?)
            AND (?='' OR instr(ovl_lower(t.album),ovl_lower(?))>0 OR instr(ovl_lower(t.artist),ovl_lower(?))>0)
            GROUP BY f.library_id,t.artist,t.album ORDER BY t.artist,t.album,f.library_id LIMIT 100 OFFSET ?");
        let mut stmt=c.prepare(&sql)?;
        let rows=stmt.query_map(params![u.admin,u.id,q.library,q.library,q.artist,q.artist,q.q,q.q,q.q,q.offset],|r|
            Ok(json!({"library_id":r.get::<_,i64>(0)?,"artist":r.get::<_,String>(1)?,
                "album":r.get::<_,String>(2)?,"tracks":r.get::<_,i64>(3)?})))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!(rows))
    }).await?))
}

#[derive(Deserialize)]
pub struct ArtistsQuery {
    library: Option<i64>,
    #[serde(default)]
    q: String,
    #[serde(default)]
    offset: i64,
}

pub async fn artists(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Query(q): Query<ArtistsQuery>,
) -> Result<Json<Value>> {
    if q.q.len() > 800 || !(0..=100_000).contains(&q.offset) {
        return Err(Error::new(400, "invalid music artist query"));
    }
    Ok(Json(app.db.call(move|c| {
        let sql=format!("SELECT f.library_id,t.artist,count(DISTINCT nullif(t.album,'')),count(*) {CURRENT}
            WHERE {FRESH} AND {GRANT} AND t.artist<>'' AND (? IS NULL OR f.library_id=?)
            AND (?='' OR instr(ovl_lower(t.artist),ovl_lower(?))>0)
            GROUP BY f.library_id,t.artist ORDER BY t.artist,f.library_id LIMIT 100 OFFSET ?");
        let mut stmt=c.prepare(&sql)?;
        let rows=stmt.query_map(params![u.admin,u.id,q.library,q.library,q.q,q.q,q.offset],|r|
            Ok(json!({"library_id":r.get::<_,i64>(0)?,"artist":r.get::<_,String>(1)?,
                "albums":r.get::<_,i64>(2)?,"tracks":r.get::<_,i64>(3)?})))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!(rows))
    }).await?))
}

#[derive(Deserialize)]
pub struct TracksQuery {
    library: i64,
    artist: String,
    album: Option<String>,
    #[serde(default)]
    offset: i64,
}

pub async fn tracks(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Query(q): Query<TracksQuery>,
) -> Result<Json<Value>> {
    if q.artist.len() > 800
        || q.album.as_ref().is_some_and(|a| a.len() > 800)
        || (q.album.is_none() && q.artist.is_empty())
        || !(0..=100_000).contains(&q.offset)
    {
        return Err(Error::new(400, "invalid music track query"));
    }
    if q.album.as_deref() == Some("") {
        return Ok(Json(json!([])));
    }
    Ok(Json(
        app.db
            .call(move |c| {
                let sql = format!(
                    "SELECT i.id,f.library_id,coalesce(nullif(t.title,''),i.title),
            i.series,i.season,i.episode,i.overview,coalesce(p.position,0),coalesce(p.watched,0)
            {CURRENT} JOIN items i ON i.id=f.item_id
            LEFT JOIN progress p ON p.item_id=i.id AND p.user_id=?
            WHERE {FRESH} AND {GRANT} AND f.library_id=? AND (? IS NULL OR t.album=?) AND t.artist=?
            ORDER BY t.album,t.disc,t.track,i.title,i.id,f.id LIMIT 100 OFFSET ?"
                );
                let mut stmt = c.prepare(&sql)?;
                let rows = stmt
                    .query_map(
                        params![
                            u.id, u.admin, u.id, q.library, q.album, q.album, q.artist, q.offset
                        ],
                        item_row,
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(json!(rows))
            })
            .await?,
    ))
}

#[cfg(test)]
mod probe_tests {
    #[test]
    fn only_track_faults_are_recorded_as_permanent() {
        use std::io::{Error, ErrorKind};
        assert!(super::permanent(&Error::from(ErrorKind::PermissionDenied)));
        assert!(super::permanent(&Error::from_raw_os_error(libc::ELOOP)));
        assert!(super::permanent(&Error::from_raw_os_error(libc::ENOTDIR)));
        for transient in [libc::EIO, libc::ENOENT, libc::EMFILE, libc::ENOTCONN] {
            assert!(
                !super::permanent(&Error::from_raw_os_error(transient)),
                "{transient}"
            );
        }
        assert!(
            matches!(super::retry_after(true), super::Probe::Failed(d) if d == super::PROBE_RETRY)
        );
        assert!(
            matches!(super::retry_after(false), super::Probe::Failed(d) if d == super::PROBE_RETRY_SOON)
        );
        assert!(super::parse_tags(b"not json").is_none());
        assert!(super::parse_tags(br#"{"streams":[]}"#).is_none());
    }
}
