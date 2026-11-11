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

pub(crate) async fn inspect(app: &Arc<App>, path: &Path) -> Option<Tags> {
    let format = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mp3" => "mp3",
        "flac" => "flac",
        "ogg" | "opus" => "ogg",
        "wav" => "wav",
        "m4a" => "mov",
        _ => return None,
    };
    let slot = app.inspection.clone().try_acquire_owned().ok()?;
    let path = path.to_owned();
    let work = app.tasks.spawn_blocking(move || {
        let file = media::open_registered_file(&path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("not a regular track"));
        }
        Ok((file, slot))
    });
    let (file, slot) = tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .ok()?
        .ok()?
        .ok()?;
    let data = match media::probe_music_tags(app, file, slot, format).await {
        Ok(data) => data,
        Err(e) if e.0 == axum::http::StatusCode::UNPROCESSABLE_ENTITY => {
            return Some(Tags::default());
        }
        Err(_) => return None,
    };
    let info: serde_json::Value = serde_json::from_slice(&data).ok()?;
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
            AND (instr(lower(t.album),lower(?))>0 OR instr(lower(t.artist),lower(?))>0)
            GROUP BY f.library_id,t.artist,t.album ORDER BY t.artist,t.album,f.library_id LIMIT 100 OFFSET ?");
        let mut stmt=c.prepare(&sql)?;
        let rows=stmt.query_map(params![u.admin,u.id,q.library,q.library,q.artist,q.artist,q.q,q.q,q.offset],|r|
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
            AND instr(lower(t.artist),lower(?))>0
            GROUP BY f.library_id,t.artist ORDER BY t.artist,f.library_id LIMIT 100 OFFSET ?");
        let mut stmt=c.prepare(&sql)?;
        let rows=stmt.query_map(params![u.admin,u.id,q.library,q.library,q.q,q.offset],|r|
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
        || q.album
            .as_ref()
            .is_some_and(|a| a.is_empty() || a.len() > 800)
        || (q.album.is_none() && q.artist.is_empty())
        || !(0..=100_000).contains(&q.offset)
    {
        return Err(Error::new(400, "invalid music track query"));
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
