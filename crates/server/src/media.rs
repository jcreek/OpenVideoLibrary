use crate::{
    App,
    auth::{self, User, parse},
    error::{Error, Result},
};
use axum::{
    Extension, Json,
    body::{Body, Bytes},
    extract::{Path, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path as FsPath, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};
use tower::ServiceExt;

pub async fn item(app: &App, u: User, id: i64) -> Result<(PathBuf, i64)> {
    let (path, library, _) = resolve_item(app, u, id, false).await?;
    Ok((path, library))
}

pub(crate) async fn item_with_kind(app: &App, u: User, id: i64) -> Result<(PathBuf, i64, String)> {
    resolve_item(app, u, id, true).await
}

pub(crate) async fn locate_item(
    app: &App,
    u: User,
    id: i64,
    kind: bool,
) -> Result<(String, String, i64, String)> {
    app.db.call(move|c|{
        let entry=c.query_row("SELECT f.path,l.path,l.id,CASE WHEN ? THEN CASE WHEN EXISTS(SELECT 1 FROM ovl_music_libraries m WHERE m.library_id=l.id) THEN 'music' ELSE l.kind END ELSE '' END FROM media_files f JOIN libraries l ON l.id=f.library_id WHERE f.item_id=? AND (? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=l.id AND a.user_id=?)) ORDER BY f.id LIMIT 1",params![kind,id,u.admin,u.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).optional()?;
        entry.ok_or_else(||Error::new(404,"media unavailable"))
    }).await
}

async fn resolve_item(app: &App, u: User, id: i64, kind: bool) -> Result<(PathBuf, i64, String)> {
    let (path, root, library, kind) = locate_item(app, u, id, kind).await?;
    let root = PathBuf::from(root);
    let registered = tokio::fs::canonicalize(&root)
        .await
        .map_err(|_| Error::new(404, "media unavailable"))?;
    if registered != root {
        return Err(Error::new(404, "media unavailable"));
    }
    let real = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| Error::new(404, "media unavailable"))?;
    if !real.starts_with(&root)
        || !tokio::fs::metadata(&real)
            .await
            .map_err(|_| Error::new(404, "media unavailable"))?
            .is_file()
    {
        return Err(Error::new(404, "media unavailable"));
    }
    Ok((real, library, kind))
}
pub async fn serve(app: &App, path: PathBuf, req: Request) -> Result<Response> {
    let response = tower_http::services::ServeFile::new(path)
        .oneshot(req)
        .await
        .map_err(|_| Error::new(500, "file unavailable"))?;
    let (parts, body) = response.into_parts();
    // Active direct/range/artwork/HLS responses must not hold graceful shutdown
    // open for the remaining length of a movie or a stalled client connection.
    let stream = Body::new(body)
        .into_data_stream()
        .take_until(app.cancel.clone().cancelled_owned());
    Ok(Response::from_parts(parts, Body::from_stream(stream)))
}
pub async fn file(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    req: Request,
) -> Result<Response> {
    let (path, _) = item(&app, u, id).await?;
    serve(&app, path, req).await
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Track {
    pub index: i32,
    pub codec_name: String,
    pub codec_type: String,
    pub disposition: Disposition,
    pub channels: u32,
    pub width: u32,
    pub height: u32,
    pub color_transfer: String,
    pub color_primaries: String,
    pub color_space: String,
    pub color_range: String,
    pub pix_fmt: String,
    pub codec_tag_string: String,
    pub side_data_list: Vec<SideData>,
    #[serde(skip)]
    pub hdr_frame_metadata_checked: bool,
    pub tags: HashMap<String, String>,
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Disposition {
    pub attached_pic: u8,
    pub timed_thumbnails: u8,
}
impl Track {
    fn is_playable_video(&self) -> bool {
        self.codec_type == "video"
            && self.disposition.attached_pic == 0
            && self.disposition.timed_thumbnails == 0
    }
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct SideData {
    pub side_data_type: String,
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Format {
    pub format_name: String,
    pub start_time: String,
    pub duration: String,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Info {
    pub streams: Vec<Track>,
    pub format: Format,
}
impl Info {
    pub fn video(&self) -> Option<&Track> {
        self.streams.iter().find(|t| t.is_playable_video())
    }
    pub fn tracks(&self, kind: &str) -> Vec<&Track> {
        self.streams
            .iter()
            .filter(|t| t.codec_type == kind)
            .collect()
    }
    pub fn hdr(&self) -> bool {
        self.video()
            .is_some_and(|v| ["smpte2084", "arib-std-b67"].contains(&v.color_transfer.as_str()))
    }
}
async fn bounded_read(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if data.len() + n > limit {
            return Err(Error::new(422, "process output limit exceeded"));
        }
        data.extend_from_slice(&buf[..n]);
    }
    Ok(data)
}
async fn tail_read(mut reader: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut tail = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > 8192 {
            tail.drain(..tail.len() - 8192);
        }
    }
    Ok(tail)
}
/// The tracked task owns wait/kill even if the requesting handler is dropped.
/// Its cancellation guard cancels on client disconnect; shutdown waits for reaping.
pub async fn process(
    app: &Arc<App>,
    program: &str,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    timeout: Duration,
    limit: usize,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Vec<u8>> {
    let request_cancel = cancel.child_token();
    let _guard = request_cancel.clone().drop_guard();
    let global = app.cancel.clone();
    let program = program.to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.tasks.spawn(async move {
        let result=async {
            let mut command=Command::new(program);command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
            if let Some(cwd)=cwd{command.current_dir(cwd);}
            let mut child=command.spawn().map_err(|_|Error::new(503,"media tool unavailable"))?;
            let stdout=child.stdout.take().expect("piped stdout");let stderr=child.stderr.take().expect("piped stderr");
            let outcome=tokio::select! {
                _=global.cancelled()=>Err(Error::new(503,"server stopping")),
                _=request_cancel.cancelled()=>Err(Error::new(499,"process cancelled")),
                _=tokio::time::sleep(timeout)=>Err(Error::new(504,"media process timed out")),
                result=async {tokio::try_join!(async{Ok(child.wait().await?)},bounded_read(stdout,limit),tail_read(stderr))}=>{
                    match result {Ok((status,out,_tail)) if status.success()=>Ok(out),Ok((_,_,tail))=>{tracing::warn!(stderr=%String::from_utf8_lossy(&tail),"media process failed");Err(Error::new(422,"media conversion failed"))},Err(e)=>Err(e)}
                }
            };
            if outcome.is_err(){let _=child.kill().await;let _=child.wait().await;}
            outcome
        }.await;let _=tx.send(result);
    });
    rx.await
        .map_err(|_| Error::new(503, "media worker unavailable"))?
}
pub async fn inspect(app: &Arc<App>, path: &FsPath) -> Result<Info> {
    let enrich = crate::hardware::wants_vaapi_hdr(
        &std::env::var("OVL_ACCELERATOR").unwrap_or("auto".into()),
        &std::env::var("OVL_HDR_ACCELERATOR").unwrap_or_default(),
    );
    inspect_with_hdr_metadata(app, path, enrich).await
}
/// Explicit enrichment is also used by physical acceptance tools; ordinary CPU
/// playback avoids the additional bounded frame decode.
pub async fn inspect_with_hdr_metadata(
    app: &Arc<App>,
    path: &FsPath,
    enrich: bool,
) -> Result<Info> {
    let _slot = app
        .inspection
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(503, "media inspection busy; retry shortly"))?;
    let out = process(
        app,
        "ffprobe",
        vec![
            "-v".into(),
            "error".into(),
            "-show_streams".into(),
            "-show_format".into(),
            "-of".into(),
            "json".into(),
            path.to_string_lossy().into_owned(),
        ],
        None,
        Duration::from_secs(10),
        2 << 20,
        app.cancel.child_token(),
    )
    .await?;
    let mut info: Info = serde_json::from_slice(&out)
        .map_err(|_| Error::new(503, "invalid media probe response"))?;
    // HEVC mastering/CLL (and some Dolby Vision signals) can be frame side data
    // rather than stream metadata. Decode at most eight packets, with bounded
    // output/time/threads. Failure leaves the conservative software path intact.
    if enrich
        && info.hdr()
        && let Some(position) = info.streams.iter().position(Track::is_playable_video)
    {
        let frames = process(
            app,
            "ffprobe",
            vec![
                "-v".into(),
                "error".into(),
                "-threads".into(),
                "2".into(),
                "-select_streams".into(),
                info.streams[position].index.to_string(),
                "-read_intervals".into(),
                "%+#8".into(),
                "-show_frames".into(),
                "-show_entries".into(),
                "frame=side_data_list".into(),
                "-of".into(),
                "json".into(),
                path.to_string_lossy().into_owned(),
            ],
            None,
            Duration::from_secs(10),
            512 << 10,
            app.cancel.child_token(),
        )
        .await;
        if let Ok(bytes) = frames {
            #[derive(Deserialize)]
            struct Frame {
                #[serde(default)]
                side_data_list: Vec<SideData>,
            }
            #[derive(Deserialize)]
            struct Frames {
                frames: Vec<Frame>,
            }
            if let Ok(frames) = serde_json::from_slice::<Frames>(&bytes) {
                let video = &mut info.streams[position];
                video.hdr_frame_metadata_checked = !frames.frames.is_empty();
                for side in frames.frames.into_iter().flat_map(|f| f.side_data_list) {
                    if !video
                        .side_data_list
                        .iter()
                        .any(|v| v.side_data_type == side.side_data_type)
                    {
                        video.side_data_list.push(side);
                    }
                }
            }
        }
    }
    // Publish a bounded display subset rather than arbitrary container tags.
    let tags = std::mem::take(&mut info.format.tags);
    if info.video().is_none() {
        let audio = info.streams.iter().find(|t| t.codec_type == "audio");
        for field in ["title", "artist", "album", "album_artist", "track", "disc"] {
            let value = tags
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(field))
                .or_else(|| {
                    audio.and_then(|t| {
                        t.tags
                            .iter()
                            .find(|(key, _)| key.eq_ignore_ascii_case(field))
                    })
                });
            if let Some((_, value)) = value {
                let display: String = value
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect();
                if !display.is_empty() {
                    info.format.tags.insert(field.into(), display);
                }
            }
        }
    }
    if info.video().is_none() {
        for track in &mut info.streams {
            track.tags.clear();
        }
    }
    // FFprobe reports shared demuxer aliases, not the actual browser container.
    // A Matroska input must not be advertised as WebM merely because FFmpeg
    // reports "matroska,webm". Ambiguous/unknown extensions remain conservative.
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if info.format.format_name == "matroska,webm" {
        info.format.format_name = if extension == "webm" {
            "webm"
        } else {
            "matroska"
        }
        .into();
    } else if info.format.format_name == "mov,mp4,m4a,3gp,3g2,mj2" {
        info.format.format_name = match extension.as_str() {
            "mp4" | "m4v" => "mp4",
            "m4a" => "m4a",
            "3gp" => "3gp",
            "3g2" => "3g2",
            "mj2" => "mj2",
            _ => "mov",
        }
        .into();
    }
    Ok(info)
}
pub async fn probe(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
) -> Result<Json<Info>> {
    let (path, _) = item(&app, u, id).await?;
    Ok(Json(inspect(&app, &path).await?))
}
pub async fn artwork(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    req: Request,
) -> Result<Response> {
    let (path, _) = item(&app, u, id).await?;
    let cache = app.config.data.join("artwork").join(format!("{id}.jpg"));
    if let Ok(real) = tokio::fs::canonicalize(&cache).await
        && real.starts_with(app.config.data.join("artwork"))
        && let Ok(metadata) = tokio::fs::metadata(&real).await
        && metadata.is_file()
        && metadata.len() < 20 << 20
    {
        return serve(&app, real, req).await;
    }
    let root = path
        .parent()
        .ok_or_else(|| Error::new(404, "artwork unavailable"))?;
    let base = path.with_extension("").to_string_lossy().into_owned();
    for p in [
        PathBuf::from(format!("{base}-poster.jpg")),
        PathBuf::from(format!("{base}-poster.png")),
        root.join("poster.jpg"),
        root.join("folder.jpg"),
    ] {
        if let Ok(real) = tokio::fs::canonicalize(p).await
            && real.starts_with(root)
            && let Ok(m) = tokio::fs::metadata(&real).await
            && m.is_file()
            && m.len() < 20 << 20
        {
            return serve(&app, real, req).await;
        }
    }
    Err(Error::new(404, "no local artwork"))
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Subtitle {
    index: u32,
    #[serde(default)]
    external: bool,
}
pub async fn subtitle(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Response> {
    let (mut path, _) = item(&app, u, id).await?;
    let b: Subtitle = parse(&body)?;
    let _slot = app
        .inspection
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(429, "subtitle conversion busy"))?;
    let mut args = vec!["-nostdin".into(), "-v".into(), "error".into()];
    if b.external {
        path.set_extension("srt");
        let m = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| Error::new(404, "external SRT unavailable"))?;
        if !m.is_file() || m.len() > 10 << 20 {
            return Err(Error::new(404, "external SRT unavailable"));
        }
    }
    args.extend(["-i".into(), path.to_string_lossy().into_owned()]);
    if !b.external {
        args.extend(["-map".into(), format!("0:s:{}", b.index)]);
    }
    args.extend(["-f".into(), "webvtt".into(), "pipe:1".into()]);
    let out = process(
        &app,
        "ffmpeg",
        args,
        None,
        Duration::from_secs(15),
        10 << 20,
        app.cancel.child_token(),
    )
    .await?;
    Ok(([(header::CONTENT_TYPE, "text/vtt")], out).into_response())
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Match {
    #[serde(rename = "ID")]
    id: u64,
    kind: String,
}
async fn download(client: &reqwest::Client, url: reqwest::Url, limit: usize) -> Result<Vec<u8>> {
    let mut res = client
        .get(url)
        .send()
        .await
        .map_err(|_| Error::new(502, "metadata provider unavailable"))?;
    if res.status() != StatusCode::OK {
        return Err(Error::new(502, "metadata provider rejected request"));
    }
    let mut out = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|_| Error::new(502, "metadata download failed"))?
    {
        if out.len() + chunk.len() > limit {
            return Err(Error::new(502, "metadata response too large"));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
pub async fn metadata_match(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    auth::admin(u)?;
    item(&app, u, id).await?;
    let b: Match = parse(&body)?;
    if b.id == 0 || !["movie", "tv"].contains(&b.kind.as_str()) {
        return Err(Error::new(400, "TMDB ID and movie/tv kind required"));
    }
    let key = std::env::var("OVL_TMDB_KEY")
        .map_err(|_| Error::new(503, "OVL_TMDB_KEY is not configured"))?;
    let _slot = app
        .metadata
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(429, "metadata queue full"))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Error::new(502, "metadata client unavailable"))?;
    let mut url = reqwest::Url::parse(&format!("https://api.themoviedb.org/3/{}/{}", b.kind, b.id))
        .expect("fixed provider URL");
    url.query_pairs_mut().append_pair("api_key", &key);
    let data = tokio::select! {_=app.cancel.cancelled()=>return Err(Error::new(503,"server stopping")),data=download(&client,url,2<<20)=>data?};
    let v: Value =
        serde_json::from_slice(&data).map_err(|_| Error::new(502, "invalid metadata"))?;
    let title = v
        .get("title")
        .or_else(|| v.get("name"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new(502, "missing metadata title"))?
        .to_owned();
    let overview = v
        .get("overview")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let (t, o) = (title.clone(), overview.clone());
    app.db
        .call(move |c| {
            c.execute(
                "UPDATE items SET title=?,overview=? WHERE id=?",
                params![t, o, id],
            )?;
            Ok(())
        })
        .await?;
    if let Some(poster) = v
        .get("poster_path")
        .and_then(Value::as_str)
        .filter(|s| s.starts_with('/') && !s.contains("..") && !s.contains('?') && !s.contains('#'))
        && let Ok(url) = reqwest::Url::parse(&format!("https://image.tmdb.org/t/p/w500{poster}"))
    {
        let image = tokio::select! {_=app.cancel.cancelled()=>None,data=download(&client,url,10<<20)=>data.ok()};
        if let Some(bytes) = image {
            let dir = app.config.data.join("artwork");
            tokio::fs::create_dir_all(&dir).await?;
            let tmp = dir.join(format!("poster-{}", auth::token()?));
            tokio::fs::write(&tmp, bytes).await?;
            tokio::fs::rename(tmp, dir.join(format!("{id}.jpg"))).await?;
        }
    }
    Ok(Json(json!({"title":title,"overview":overview})))
}
pub async fn capabilities(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
) -> Result<Json<Value>> {
    auth::admin(u)?;
    let accelerators = crate::hardware::detect(&app).await?;
    Ok(Json(
        json!({"accelerators":accelerators,"validated_hardware":[],"note":"Runtime encoder smoke tests are not GPU certification. HDR uses bounded software conversion by default. OVL_HDR_ACCELERATOR=vaapi opts eligible HDR10 into VAAPI with pre-publication CPU fallback; runtime smoke tests do not certify HDR quality or capacity."}),
    ))
}
