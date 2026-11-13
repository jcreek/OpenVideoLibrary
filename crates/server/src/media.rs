use crate::{
    App,
    auth::{self, User, parse},
    error::{Error, Result},
};
use axum::{
    Extension, Json,
    body::{Body, Bytes},
    extract::{Path, Query, Request, State},
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

/// Single-file demuxers accepted for library media. Playlist, concat and
/// image-sequence demuxers, which can open further files or URLs, are excluded.
pub(crate) const MEDIA_DEMUXERS: &str =
    "mov,matroska,avi,mpegts,mpeg,mpegvideo,flv,asf,ogg,mp3,flac,wav,aac";

/// Input options that keep an FFmpeg input on local files and single-file
/// demuxers, so a crafted library entry cannot select a demuxer that opens
/// other paths or URLs. They must precede that input's `-i`.
pub(crate) fn confined_input(formats: &str) -> [String; 4] {
    [
        "-protocol_whitelist".into(),
        "file".into(),
        "-format_whitelist".into(),
        formats.into(),
    ]
}

/// Most handles one child may inherit (fds 3 to 10). An adaptive burn-in uses
/// one per input plus one per rendition's subtitle filter.
pub(crate) const MAX_INHERITED: usize = 8;

/// Path at which a child sees the `slot`-th handle passed to [`process_media`].
pub(crate) fn inherited(slot: usize) -> String {
    format!("/dev/fd/{}", 3 + slot)
}

/// Opens an already authorized canonical media path through anchored,
/// symlink-free component lookups and checks the opened handle is a file.
/// Consumers then use this handle (or `/dev/fd/N` naming it), never the path,
/// so a component swapped after authorization cannot redirect the read.
pub(crate) async fn open_media(app: &App, path: &FsPath) -> Result<(std::fs::File, u64)> {
    let path = path.to_owned();
    let work = app.tasks.spawn_blocking(move || {
        let file = open_registered_file(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(std::io::Error::other("not a regular file"));
        }
        Ok((file, metadata.len()))
    });
    tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .map_err(|_| Error::new(504, "media open timed out"))?
        .map_err(|_| Error::new(500, "media worker failed"))?
        .map_err(|_| Error::new(404, "media unavailable"))
}

/// Serves authorized library media from an anchored handle. Content type comes
/// from the authorized path; ranges and validators come from the opened file.
pub async fn serve_media(app: &App, path: PathBuf, req: Request) -> Result<Response> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let (file, _) = open_media(app, &path).await?;
        let mime = axum::http::HeaderValue::from_static(
            mime_guess::from_path(&path)
                .first_raw()
                .unwrap_or("application/octet-stream"),
        );
        let mut response = serve(
            app,
            PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd())),
            req,
        )
        .await?;
        // ServeFile has opened its own handle to the same file by now.
        drop(file);
        if response.status().is_success() {
            response.headers_mut().insert(header::CONTENT_TYPE, mime);
        }
        Ok(response)
    }
    #[cfg(not(unix))]
    {
        let _ = (app, path, req);
        Err(Error::new(503, "anchored media serving requires Unix"))
    }
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
    serve_media(&app, path, req).await
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Track {
    pub index: i32,
    pub codec_name: String,
    pub codec_type: String,
    pub disposition: Disposition,
    pub channels: u32,
    pub sample_rate: String,
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
    run_process(
        app,
        program,
        args,
        cwd,
        timeout,
        limit,
        (cancel, None),
        Vec::new(),
    )
    .await
}

/// Like [`process`], with `files` inherited by the child at fd 3 and up, where
/// [`inherited`] names them. The child reads exactly these handles.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_media(
    app: &Arc<App>,
    program: &str,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    timeout: Duration,
    limit: usize,
    cancel: tokio_util::sync::CancellationToken,
    files: Vec<std::fs::File>,
) -> Result<Vec<u8>> {
    run_process(
        app,
        program,
        args,
        cwd,
        timeout,
        limit,
        (cancel, None),
        files,
    )
    .await
}

pub(crate) async fn probe_lyrics(
    app: &Arc<App>,
    file: std::fs::File,
    slot: tokio::sync::OwnedSemaphorePermit,
) -> Result<Vec<u8>> {
    probe_music_tags(app, file, slot, "mp3,flac,ogg").await
}

pub(crate) async fn probe_music_tags(
    app: &Arc<App>,
    file: std::fs::File,
    slot: tokio::sync::OwnedSemaphorePermit,
    formats: &str,
) -> Result<Vec<u8>> {
    run_process(
        app,
        "ffprobe",
        [
            "-v",
            "error",
            "-protocol_whitelist",
            "pipe",
            "-format_whitelist",
            formats,
            "-show_entries",
            "format_tags:stream=codec_type:stream_tags:stream_disposition=attached_pic,timed_thumbnails",
            "-of",
            "json",
            "pipe:0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        None,
        Duration::from_secs(5),
        1 << 20,
        (app.cancel.child_token(), Some((file, slot))),
        Vec::new(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_process(
    app: &Arc<App>,
    program: &str,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    timeout: Duration,
    limit: usize,
    (cancel, input): (
        tokio_util::sync::CancellationToken,
        Option<(std::fs::File, tokio::sync::OwnedSemaphorePermit)>,
    ),
    files: Vec<std::fs::File>,
) -> Result<Vec<u8>> {
    const MAX_FILES: usize = MAX_INHERITED;
    if files.len() > MAX_FILES {
        return Err(Error::new(500, "too many media handles"));
    }
    let request_cancel = cancel.child_token();
    let _guard = request_cancel.clone().drop_guard();
    let global = app.cancel.clone();
    let program = program.to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.tasks.spawn(async move {
        let result=async {
            let (stdin, _slot) = match input {
                Some((file, slot)) => (Stdio::from(file), Some(slot)),
                None => (Stdio::null(), None),
            };
            let mut command=Command::new(program);command.args(args).stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
            if let Some(cwd)=cwd{command.current_dir(cwd);}
            #[cfg(unix)]
            if !files.is_empty() {
                use std::os::fd::AsRawFd;
                let count = files.len();
                let mut raw = [-1; MAX_FILES];
                for (slot, file) in raw.iter_mut().zip(&files) {
                    *slot = file.as_raw_fd();
                }
                // SAFETY: between fork and exec this runs only async-signal-safe
                // fcntl/dup2 on stack data. Sources first move above the target
                // range (close-on-exec), so no dup2 overwrites a later source.
                unsafe {
                    command.pre_exec(move || {
                        let mut high = [-1; MAX_FILES];
                        for (moved, source) in high.iter_mut().zip(raw).take(count) {
                            *moved = libc::fcntl(source, libc::F_DUPFD_CLOEXEC, 64);
                            if *moved < 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                        }
                        for (i, fd) in high.iter().enumerate().take(count) {
                            if libc::dup2(*fd, 3 + i as libc::c_int) < 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                        }
                        Ok(())
                    });
                }
            }
            let mut child=command.spawn().map_err(|_|Error::new(503,"media tool unavailable"))?;
            // The child holds its own copies; the parent's handles are no longer needed.
            drop(files);
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
/// FFmpeg's input seek on MPEG-TS (no seek index) can land after a keyframe and
/// then start at the next one, up to a GOP late. This bounded lookup finds the
/// latest video keyframe at or before `start` (seconds from the media origin)
/// so callers can seek the input onto it. It reads packet headers only (no
/// decoding): one packet, then 2 s and 8 s windows before the target, under a
/// 3 s overall deadline. It is an accuracy aid, so a busy inspection slot,
/// timeout or no visible keyframe returns `None` and callers keep FFmpeg's seek.
pub async fn keyframe_at_or_before(
    app: &Arc<App>,
    path: &FsPath,
    info: &Info,
    start: f64,
) -> Result<Option<f64>> {
    let Some(video) = info.video() else {
        return Ok(None);
    };
    if start <= 0.0 {
        return Ok(None);
    }
    let origin = info
        .format
        .start_time
        .parse::<f64>()
        .ok()
        .filter(|t| t.is_finite())
        .unwrap_or(0.0);
    let target = origin + start;
    let Ok(_slot) = app.inspection.clone().try_acquire_owned() else {
        return Ok(None);
    };
    let intervals = std::iter::once(format!("{target:.6}%+#1")).chain(
        [2.0, 8.0]
            .into_iter()
            .map(|window: f64| format!("{:.6}%{:.6}", (target - window).max(0.0), target + 0.001)),
    );
    let lookup = async {
        for interval in intervals {
            let (mut args, input, files) = probe_input(app, path, true).await?;
            args.splice(
                0..0,
                [
                    "-v".into(),
                    "error".into(),
                    "-select_streams".into(),
                    video.index.to_string(),
                    "-read_intervals".into(),
                    interval,
                    "-show_entries".into(),
                    "packet=pts_time,flags".into(),
                    "-of".into(),
                    "csv=p=0".into(),
                ],
            );
            args.push(input);
            let out = process_media(
                app,
                "ffprobe",
                args,
                None,
                Duration::from_secs(3),
                1 << 20,
                app.cancel.child_token(),
                files,
            )
            .await?;
            if let Some(keyframe) = latest_keyframe(&String::from_utf8_lossy(&out), target) {
                return Ok(Some((keyframe - origin).max(0.0)));
            }
        }
        Ok(None)
    };
    // Dropping the future on deadline kills and reaps the running probe.
    tokio::time::timeout(Duration::from_secs(3), lookup)
        .await
        .unwrap_or(Ok(None))
}

/// Source position at which a published remux segment begins. Remux runs with
/// `-copyts`, so segment timestamps are source timestamps; the earliest stream
/// start minus the source origin is where the client timeline begins.
pub async fn segment_start(app: &Arc<App>, segment: &FsPath, origin: f64) -> Result<Option<f64>> {
    let out = process(
        app,
        "ffprobe",
        vec![
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "stream=start_time".into(),
            "-of".into(),
            "csv=p=0".into(),
            segment.to_string_lossy().into_owned(),
        ],
        None,
        Duration::from_secs(5),
        64 << 10,
        app.cancel.child_token(),
    )
    .await?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse::<f64>().ok())
        .filter(|t| t.is_finite())
        .reduce(f64::min)
        .map(|first| (first - origin).max(0.0)))
}

/// Selects the latest keyframe packet time not after `target` from ffprobe
/// `packet=pts_time,flags` CSV rows.
pub fn latest_keyframe(output: &str, target: f64) -> Option<f64> {
    output
        .lines()
        .filter_map(|line| {
            let mut pts = None;
            let mut key = false;
            for field in line.split(',').map(str::trim) {
                if let Ok(value) = field.parse::<f64>() {
                    pts = Some(value);
                } else if field.starts_with('K') {
                    key = true;
                }
            }
            pts.filter(|t| key && t.is_finite() && *t <= target + 1e-6)
        })
        .reduce(f64::max)
}

fn hdr_enrichment() -> bool {
    crate::hardware::wants_vaapi_hdr(
        &std::env::var("OVL_ACCELERATOR").unwrap_or("auto".into()),
        &std::env::var("OVL_HDR_ACCELERATOR").unwrap_or_default(),
    )
}
/// Probes a trusted local file, such as generated output or a playlist the
/// server wrote. Library media must use [`inspect_media`].
pub async fn inspect(app: &Arc<App>, path: &FsPath) -> Result<Info> {
    inspect_with_hdr_metadata(app, path, hdr_enrichment()).await
}
/// Probes authorized library media through an anchored handle, confined to
/// local single-file demuxers.
pub async fn inspect_media(app: &Arc<App>, path: &FsPath) -> Result<Info> {
    inspect_source(app, path, hdr_enrichment(), true).await
}
/// Explicit enrichment is also used by physical acceptance tools; ordinary CPU
/// playback avoids the additional bounded frame decode. Trusted paths only.
pub async fn inspect_with_hdr_metadata(
    app: &Arc<App>,
    path: &FsPath,
    enrich: bool,
) -> Result<Info> {
    inspect_source(app, path, enrich, false).await
}
/// Arguments and inherited handles for one probe of `path`. Library media are
/// read only through a fresh anchored handle with confined demuxing.
async fn probe_input(
    app: &App,
    path: &FsPath,
    library: bool,
) -> Result<(Vec<String>, String, Vec<std::fs::File>)> {
    if library {
        let (file, _) = open_media(app, path).await?;
        Ok((
            confined_input(MEDIA_DEMUXERS).into(),
            inherited(0),
            vec![file],
        ))
    } else {
        Ok((Vec::new(), path.to_string_lossy().into_owned(), Vec::new()))
    }
}
async fn inspect_source(
    app: &Arc<App>,
    path: &FsPath,
    enrich: bool,
    library: bool,
) -> Result<Info> {
    let _slot = app
        .inspection
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(503, "media inspection busy; retry shortly"))?;
    let (mut args, input, files) = probe_input(app, path, library).await?;
    args.splice(
        0..0,
        [
            "-v".into(),
            "error".into(),
            "-show_streams".into(),
            "-show_format".into(),
            "-of".into(),
            "json".into(),
        ],
    );
    args.push(input);
    let out = process_media(
        app,
        "ffprobe",
        args,
        None,
        Duration::from_secs(10),
        2 << 20,
        app.cancel.child_token(),
        files,
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
        let frames = async {
            let (mut args, input, files) = probe_input(app, path, library).await?;
            args.splice(
                0..0,
                [
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
                ],
            );
            args.push(input);
            process_media(
                app,
                "ffprobe",
                args,
                None,
                Duration::from_secs(10),
                512 << 10,
                app.cancel.child_token(),
                files,
            )
            .await
        }
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
    Ok(Json(inspect_media(&app, &path).await?))
}
#[cfg(unix)]
pub(crate) fn open_registered_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };
    let mut parts = path.components().peekable();
    if parts.next() != Some(Component::RootDir) {
        return Err(std::io::Error::other("expected absolute media path"));
    }
    let mut directory = std::fs::File::open("/")?;
    while let Some(Component::Normal(name)) = parts.next() {
        let name = CString::new(name.as_bytes())?;
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        let flags = if parts.peek().is_some() {
            flags | libc::O_DIRECTORY
        } else {
            flags
        };
        // The directory handle anchors each component. No symlink is followed.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
        if parts.peek().is_none() {
            return Ok(directory);
        }
    }
    Err(std::io::Error::other("invalid media path"))
}

#[cfg(not(unix))]
pub(crate) fn open_registered_file(_: &std::path::Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::other("anchored media opens require Unix"))
}

async fn serve_artwork(app: &App, path: PathBuf, req: Request) -> Result<Response> {
    let mime = match path.extension().and_then(|v| v.to_str()) {
        Some("jpg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => return Err(Error::new(404, "no local artwork")),
    };
    let mut response = serve_media(app, path, req).await?;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(mime),
    );
    Ok(response)
}

#[derive(Deserialize)]
pub struct ArtworkOptions {
    #[serde(default)]
    embedded: bool,
}

async fn embedded_cover(app: &Arc<App>, path: PathBuf) -> Result<Response> {
    if !path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| ["mp3", "flac"].iter().any(|v| ext.eq_ignore_ascii_case(v)))
    {
        return Err(Error::new(404, "no supported embedded artwork"));
    }
    let slot = app
        .inspection
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(429, "media inspection busy"))?;
    let work = app.tasks.spawn_blocking(move || {
        let file = open_registered_file(&path).map_err(|_| Error::new(404, "media unavailable"))?;
        if !file.metadata()?.is_file() {
            return Err(Error::new(404, "media unavailable"));
        }
        Ok((file, slot))
    });
    let input = tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .map_err(|_| Error::new(504, "artwork open timed out"))?
        .map_err(|_| Error::new(500, "artwork worker failed"))??;
    let png = run_process(
        app,
        "ffmpeg",
        [
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "pipe",
            "-format_whitelist",
            "mp3,flac",
            "-threads",
            "1",
            "-max_pixels",
            "16777216",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-frames:v",
            "1",
            "-filter_threads",
            "1",
            "-vf",
            "scale=w='min(512,iw)':h='min(512,ih)':force_original_aspect_ratio=decrease",
            "-c:v",
            "png",
            "-threads",
            "1",
            "-f",
            "image2pipe",
            "pipe:1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        None,
        Duration::from_secs(5),
        2 << 20,
        (app.cancel.child_token(), Some(input)),
        Vec::new(),
    )
    .await
    .map_err(|e| {
        if e.0 == StatusCode::UNPROCESSABLE_ENTITY {
            Error::new(404, "no supported embedded artwork")
        } else {
            e
        }
    })?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(Error::new(404, "no supported embedded artwork"));
    }
    Ok(([(header::CONTENT_TYPE, "image/png")], png).into_response())
}

pub async fn artwork(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    Query(options): Query<ArtworkOptions>,
    req: Request,
) -> Result<Response> {
    let (path, _, kind) = item_with_kind(&app, u, id).await?;
    let cache = app.config.data.join("artwork").join(format!("{id}.jpg"));
    if let Ok(real) = tokio::fs::canonicalize(&cache).await
        && real == cache
        && let Ok(metadata) = tokio::fs::metadata(&real).await
        && metadata.is_file()
        && metadata.len() < 20 << 20
    {
        return serve_artwork(&app, real, req).await;
    }
    let root = path
        .parent()
        .ok_or_else(|| Error::new(404, "artwork unavailable"))?;
    let base = path.with_extension("").to_string_lossy().into_owned();
    let album_covers = [
        "cover.jpg",
        "cover.png",
        "cover.webp",
        "folder.jpg",
        "folder.png",
        "folder.webp",
    ];
    let video_posters = [
        PathBuf::from(format!("{base}-poster.jpg")),
        PathBuf::from(format!("{base}-poster.png")),
        root.join("poster.jpg"),
        root.join("folder.jpg"),
    ];
    let candidates = if kind == "music" {
        album_covers
            .iter()
            .map(|name| root.join(name))
            .collect::<Vec<_>>()
    } else {
        video_posters.into()
    };
    for p in candidates {
        if let Ok(real) = tokio::fs::canonicalize(&p).await
            && real == p
            && let Ok(m) = tokio::fs::metadata(&real).await
            && m.is_file()
            && m.len() < 20 << 20
        {
            return serve_artwork(&app, real, req).await;
        }
    }
    if kind == "music" && options.embedded {
        return embedded_cover(&app, path).await;
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
    let mut args: Vec<String> = vec!["-nostdin".into(), "-v".into(), "error".into()];
    let file = if b.external {
        path.set_extension("srt");
        let (file, len) = open_media(&app, &path)
            .await
            .map_err(|_| Error::new(404, "external SRT unavailable"))?;
        if len > 10 << 20 {
            return Err(Error::new(404, "external SRT unavailable"));
        }
        args.extend(confined_input("srt"));
        file
    } else {
        args.extend(confined_input(MEDIA_DEMUXERS));
        open_media(&app, &path).await?.0
    };
    args.extend(["-i".into(), inherited(0)]);
    if !b.external {
        args.extend(["-map".into(), format!("0:s:{}", b.index)]);
    }
    args.extend(["-f".into(), "webvtt".into(), "pipe:1".into()]);
    let out = process_media(
        &app,
        "ffmpeg",
        args,
        None,
        Duration::from_secs(15),
        10 << 20,
        app.cancel.child_token(),
        vec![file],
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
