use crate::{
    App, auth,
    error::{Error, Result},
};
use rusqlite::{OptionalExtension, params};
use std::io::Read;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};
use tokio::sync::mpsc;
use walkdir::WalkDir;
struct Found {
    path: String,
    title: String,
    series: String,
    season: i64,
    episode: i64,
    overview: String,
    mtime: i64,
    size: i64,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Nfo {
    title: String,
    plot: String,
}
/// Reads a sidecar NFO through an anchored open that refuses every symlink, so
/// the handle stays inside the canonical library root even if the tree changes
/// after discovery. Only a regular file within the size limit is read.
fn read_nfo(path: &Path) -> std::io::Result<String> {
    let file = crate::media::open_registered_file(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 1 << 20 {
        return Err(std::io::Error::other("NFO is not a bounded regular file"));
    }
    let mut out = String::new();
    file.take((1 << 20) + 1).read_to_string(&mut out)?;
    if out.len() > 1 << 20 {
        return Err(std::io::Error::other("NFO exceeds limit"));
    }
    Ok(out)
}
fn discover(
    root: &Path,
    music: bool,
    cancel: &tokio_util::sync::CancellationToken,
    tx: mpsc::Sender<Found>,
) -> Result<usize> {
    if std::fs::canonicalize(root)? != root {
        return Err(Error::new(
            503,
            "registered library root is no longer canonical",
        ));
    }
    let episode = regex::Regex::new(r"(?i)^(.*?)s(\d{1,3})e(\d{1,4})").expect("episode expression");
    let started = Instant::now();
    let mut visited = 0;
    let mut seen = 0;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .follow_root_links(false)
        .max_open(16)
    {
        if cancel.is_cancelled() {
            return Err(Error::new(499, "scan cancelled"));
        }
        visited += 1;
        if visited > 1_000_000 || started.elapsed() > Duration::from_secs(1800) {
            return Err(Error::new(503, "scan work budget exceeded"));
        }
        let entry = entry.map_err(|_| Error::new(503, "library directory unavailable"))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let extensions: &[&str] = if music {
            &["mp3", "m4a", "flac", "ogg", "opus", "wav"]
        } else {
            &["mkv", "mp4", "m4v", "avi", "mov", "ts", "webm"]
        };
        if !extensions.contains(&ext.as_str()) {
            continue;
        }
        let real = std::fs::canonicalize(p)?;
        if !real.starts_with(root) {
            continue;
        }
        let m = entry
            .metadata()
            .map_err(|_| Error::new(503, "media unavailable"))?;
        let stem = p.file_stem().unwrap_or_default().to_string_lossy();
        let mut title = stem.replace('.', " ");
        let mut series = String::new();
        let mut season = 0;
        let mut episode_num = 0;
        if !music && let Some(c) = episode.captures(&stem) {
            series = c[1]
                .replace('.', " ")
                .trim_matches([' ', '-', '_'])
                .to_owned();
            season = c[2].parse().unwrap_or(0);
            episode_num = c[3].parse().unwrap_or(0);
        }
        let nfo = p.with_extension("nfo");
        let mut overview = String::new();
        if let Ok(data) = read_nfo(&nfo)
            && let Ok(n) = quick_xml::de::from_str::<Nfo>(&data)
        {
            if !n.title.is_empty() {
                title = n.title;
            }
            overview = n.plot;
        }
        let mtime = m
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .min(i64::MAX as u128) as i64;
        if tx
            .blocking_send(Found {
                path: p.to_string_lossy().into_owned(),
                title,
                series,
                season,
                episode: episode_num,
                overview,
                mtime,
                size: m.len().min(i64::MAX as u64) as i64,
            })
            .is_err()
        {
            return Err(Error::new(499, "scan cancelled"));
        }
        seen += 1;
    }
    Ok(seen)
}
async fn library(app: &Arc<App>, id: i64, root: PathBuf, music: bool) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(16);
    let cancel = app.cancel.child_token();
    let producer_cancel = cancel.clone();
    let _guard = cancel.clone().drop_guard();
    let producer =
        tokio::task::spawn_blocking(move || discover(&root, music, &producer_cancel, tx));
    let result:Result<()>=async{
        let mut probes = 0;
        let metadata_deadline = Instant::now() + Duration::from_secs(60);
        loop{let found=tokio::select!{_=cancel.cancelled()=>return Err(Error::new(499,"scan cancelled")),next=rx.recv()=>next};
            let Some(f)=found else{break;};
            let probe = if music && probes < 256 && Instant::now() < metadata_deadline {
                let (path, mtime, size) = (f.path.clone(), f.mtime, f.size);
                let now = auth::now();
                // Skip current tags and recent failures of this file version.
                let indexed = app.db.call(move |c| Ok(c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM media_files f JOIN ovl_music_tags t ON t.file_id=f.id
                      WHERE f.path=?1 AND t.mtime=?2 AND t.size=?3)
                     OR EXISTS(SELECT 1 FROM media_files f JOIN ovl_music_probe_failures p ON p.file_id=f.id
                      WHERE f.path=?1 AND p.mtime=?2 AND p.size=?3 AND p.retry_at>?4)",
                    rusqlite::params![path, mtime, size, now], |r| r.get::<_, bool>(0))?)).await?;
                if indexed { None } else {
                    let probe = crate::music::inspect(app, Path::new(&f.path)).await;
                    if !matches!(probe, crate::music::Probe::Busy) { probes += 1; }
                    // A cancelled scan's probe failure says nothing about the track.
                    if cancel.is_cancelled() { return Err(Error::new(499, "scan cancelled")); }
                    Some(probe)
                }
            } else { None };
            let now = auth::now();
            app.db.call(move|c|{
                let tx=c.transaction()?;
                let known=tx.query_row("SELECT item_id,mtime,size FROM media_files WHERE path=?",[&f.path],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).optional()?;
                if let Some((_,mtime,size))=known{if mtime!=f.mtime||size!=f.size{tx.execute("UPDATE media_files SET mtime=?,size=? WHERE path=?",params![f.mtime,f.size,f.path])?;}}
                else {tx.execute("INSERT INTO items(title,series,season,episode,overview) VALUES(?,?,?,?,?)",params![f.title,f.series,f.season,f.episode,f.overview])?;let item=tx.last_insert_rowid();tx.execute("INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,?,?,?,?)",params![item,id,f.path,f.mtime,f.size])?;}
                match probe {
                    Some(crate::music::Probe::Tags(tags)) => crate::music::save(&tx, &f.path, f.mtime, f.size, tags)?,
                    Some(crate::music::Probe::Failed(delay)) => crate::music::save_failure(&tx, &f.path, f.mtime, f.size, now.saturating_add(delay.as_secs() as i64))?,
                    Some(crate::music::Probe::Busy | crate::music::Probe::Transient) | None => {}
                }
                tx.commit()?;Ok(())
            }).await?;
        }Ok(())
    }.await;
    if result.is_err() {
        cancel.cancel();
    }
    drop(rx);
    let seen = producer
        .await
        .map_err(|_| Error::new(500, "scan worker failed"))??;
    result?;
    if seen == 0 {
        let count = app
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM media_files WHERE library_id=?",
                    [id],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await?;
        if count > 0 {
            return Err(Error::new(
                503,
                "empty library root; existing records preserved",
            ));
        }
    }
    Ok(())
}
pub async fn start(app: Arc<App>) -> Result<()> {
    let permit = app
        .scans
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(409, "scan already running"))?;
    let task_app = app.clone();
    app.tasks.spawn(async move {
        let app = task_app;
        let _permit = permit;
        let started = Instant::now();
        let libs = app
            .db
            .call(|c| {
                let mut s = c.prepare("SELECT id,path,CASE WHEN EXISTS(SELECT 1 FROM ovl_music_libraries m WHERE m.library_id=l.id) THEN 'music' ELSE kind END FROM libraries l ORDER BY id")?;
                Ok(s.query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        PathBuf::from(r.get::<_, String>(1)?),
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?)
            })
            .await;
        if let Ok(libs) = libs {
            for (id, root, kind) in libs {
                if app.cancel.is_cancelled() {
                    break;
                }
                let result = library(&app, id, root, kind == "music").await;
                let offline = result.is_err();
                if let Err(e) = result {
                    tracing::warn!(library=id,error=%e,"scan incomplete; records preserved");
                }
                let _ = app
                    .db
                    .call(move |c| {
                        c.execute(
                            "UPDATE libraries SET offline=? WHERE id=?",
                            params![offline, id],
                        )?;
                        Ok(())
                    })
                    .await;
            }
        }
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            finished_at = auth::now(),
            "scan finished"
        );
    });
    Ok(())
}
