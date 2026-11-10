use crate::{
    App,
    auth::{self, User, parse},
    error::{Error, Result},
    media::{self, Info},
};
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, Request, State},
    http::StatusCode,
    response::Response,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientCapabilities {
    pub containers: Vec<String>,
    pub video_codecs: Vec<String>,
    pub audio_codecs: Vec<String>,
    #[serde(default)]
    pub video_pixel_formats: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub hdr: bool,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default, deny_unknown_fields)]
pub struct Playback {
    pub mode: String,
    pub start: f64,
    pub audio: usize,
    pub subtitle: Option<usize>,
    pub adaptive: bool,
    #[serde(rename = "CanPlayMP4")]
    pub can_play_mp4: bool,
    pub capabilities: Option<ClientCapabilities>,
}
pub struct Stream {
    pub owner: i64,
    pub library: i64,
    pub item: i64,
    pub dir: PathBuf,
    pub cancel: CancellationToken,
    pub done: watch::Receiver<bool>,
    pub touched: AtomicI64,
    pub finished: AtomicBool,
    pub error: Mutex<Option<String>>,
    pub accelerator: Mutex<Option<String>>,
}
#[derive(Clone, Debug)]
pub struct Rendition {
    pub width: u32,
    pub height: u32,
    pub kbps: u32,
}
pub fn ladder(width: u32, height: u32, adaptive: bool) -> Vec<Rendition> {
    let (w, h) = if width < 2 || height < 2 {
        (1920, 1080)
    } else {
        (width, height)
    };
    let mut out = vec![];
    for (height, kbps) in [(1080, 5000), (720, 2800), (480, 1200)] {
        if height > h {
            continue;
        }
        out.push(Rendition {
            width: ((u64::from(w) * u64::from(height) / u64::from(h)) as u32 / 2 * 2).max(2),
            height,
            kbps,
        });
        if !adaptive {
            break;
        }
    }
    if out.is_empty() {
        out.push(Rendition {
            width: w / 2 * 2,
            height: h / 2 * 2,
            kbps: 1000,
        });
    }
    out
}
// Codec-only reports describe the conservative 8-bit 4:2:0 browser baseline.
// Native clients may explicitly report additional formats for each codec.
fn pixel_format_supported(track: &media::Track, capabilities: Option<&ClientCapabilities>) -> bool {
    if track.pix_fmt.is_empty() {
        return false;
    }
    if let Some(formats) = capabilities.and_then(|c| c.video_pixel_formats.get(&track.codec_name)) {
        return formats.contains(&track.pix_fmt);
    }
    matches!(track.codec_name.as_str(), "h264" | "hevc" | "vp9" | "av1")
        && matches!(track.pix_fmt.as_str(), "yuv420p" | "yuvj420p")
}
pub fn negotiate(info: &Info, b: &Playback) -> (String, String) {
    if b.subtitle.is_some() {
        return (
            "transcode".into(),
            "Selected subtitles require burn-in".into(),
        );
    }
    let Some(v) = info.video() else {
        return ("transcode".into(), "No supported video track".into());
    };
    let audio = info.tracks("audio");
    if let Some(c) = &b.capabilities {
        let video_ok = c.video_codecs.contains(&v.codec_name)
            && pixel_format_supported(v, Some(c))
            && (!info.hdr() || c.hdr);
        let audio_ok = audio
            .get(b.audio)
            .is_none_or(|a| c.audio_codecs.contains(&a.codec_name));
        let container_ok = info
            .format
            .format_name
            .split(',')
            .all(|f| c.containers.iter().any(|v| v == f));
        if video_ok && audio_ok && container_ok && b.audio == 0 {
            return (
                "direct".into(),
                "Client supports source video, audio and container".into(),
            );
        }
        if v.codec_name == "h264" && video_ok {
            return (
                "remux".into(),
                "Video can be copied; container or audio needs conversion".into(),
            );
        }
        return (
            "transcode".into(),
            if info.hdr() && !c.hdr {
                "Client requires HDR to SDR tone mapping"
            } else {
                "Source video codec or pixel format is unsupported by client"
            }
            .into(),
        );
    }
    let audio_ok = audio
        .get(b.audio)
        .is_none_or(|a| ["aac", "mp3"].contains(&a.codec_name.as_str()));
    if b.can_play_mp4
        && info.format.format_name.contains("mp4")
        && v.codec_name == "h264"
        && pixel_format_supported(v, None)
        && !info.hdr()
        && audio_ok
        && b.audio == 0
    {
        return (
            "direct".into(),
            "Client supports H.264 MP4 and source audio".into(),
        );
    }
    if v.codec_name == "h264" && pixel_format_supported(v, None) && !info.hdr() {
        return (
            "remux".into(),
            "Video can be copied into HLS with compatible audio".into(),
        );
    }
    (
        "transcode".into(),
        if info.hdr() {
            "HDR source requires SDR tone mapping"
        } else {
            "Video requires H.264 conversion"
        }
        .into(),
    )
}
pub async fn start(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(item_id): Path<i64>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    let (path, library) = media::item(&app, u, item_id).await?;
    let mut b: Playback = parse(&body)?;
    if !b.start.is_finite() || b.start < 0.0 {
        return Err(Error::new(400, "invalid stream options"));
    }
    if b.mode.is_empty() {
        b.mode = "auto".into();
    }
    if !["auto", "direct", "remux", "transcode"].contains(&b.mode.as_str()) {
        return Err(Error::new(400, "invalid playback mode"));
    }
    let mut info = Info::default();
    let mut reason = "Playback mode selected manually".to_owned();
    if b.mode != "direct" {
        info = media::inspect(&app, &path).await?;
        if info.video().is_none() {
            return Err(Error::new(422, "no video track"));
        }
        if b.audio >= info.tracks("audio").len() && b.audio != 0 {
            return Err(Error::new(400, "audio track unavailable"));
        }
        if b.subtitle
            .is_some_and(|i| i >= info.tracks("subtitle").len())
        {
            return Err(Error::new(400, "subtitle track unavailable"));
        }
        if b.mode == "auto" {
            (b.mode, reason) = negotiate(&info, &b);
        }
    }
    if b.subtitle.is_some() && b.mode != "transcode" {
        return Err(Error::new(400, "subtitle burn-in requires transcoding"));
    }
    if b.mode == "direct" && b.audio > 0 {
        return Err(Error::new(
            400,
            "select automatic or remux playback to change audio tracks",
        ));
    }
    if b.mode == "direct" {
        return Ok((
            StatusCode::OK,
            Json(
                json!({"url":format!("/api/v1/items/{item_id}/file"),"mode":"direct","start":b.start,"reason":reason}),
            ),
        ));
    }
    let mut streams = app.streams.lock().await;
    if streams.len() >= app.config.max_streams {
        return Err(Error::new(429, "stream limit reached"));
    }
    let free = fs2::available_space(&app.config.data)?;
    let reserved = (streams.len() as u64 + 1)
        .saturating_mul(app.config.transcode_bytes)
        .saturating_add(app.config.min_free_bytes);
    if free < reserved {
        return Err(Error::new(
            507,
            "insufficient free space for transcode reservation",
        ));
    }
    let id = auth::token()?;
    let dir = app.config.data.join("transcodes").join(&id);
    tokio::fs::create_dir_all(&dir).await?;
    let (done_tx, done) = watch::channel(false);
    let stream = Arc::new(Stream {
        owner: u.id,
        library,
        item: item_id,
        dir,
        cancel: app.cancel.child_token(),
        done,
        touched: AtomicI64::new(auth::now()),
        finished: AtomicBool::new(false),
        error: Mutex::new(None),
        accelerator: Mutex::new(None),
    });
    streams.insert(id.clone(), stream.clone());
    drop(streams);
    let response = json!({"id":id,"url":format!("/api/v1/playback/{id}/index.m3u8"),"mode":b.mode,"start":b.start,"reason":reason});
    let task_app = app.clone();
    app.tasks.spawn(async move {
        let app=task_app;
        let result=encode(&app,&stream,path,b,info).await;
        if let Err(e)=result&& !stream.cancel.is_cancelled(){*stream.error.lock().await=Some(e.1);}
        stream.finished.store(true,Ordering::Release);
        loop{tokio::select!{_=stream.cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(1))=>{if auth::now()-stream.touched.load(Ordering::Relaxed)>app.config.stream_idle.as_secs() as i64{break;}}}}
        stream.cancel.cancel();
        let _=tokio::fs::remove_dir_all(&stream.dir).await;
        app.streams.lock().await.remove(&id);
        let _=done_tx.send(true);
    });
    Ok((StatusCode::CREATED, Json(response)))
}
pub fn encoding_args(
    path: &std::path::Path,
    b: &Playback,
    info: &Info,
    variants: &[Rendition],
) -> Vec<String> {
    let mut a = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-ss".into(),
        format!("{:.3}", b.start),
        "-i".into(),
        path.to_string_lossy().into_owned(),
    ];
    let bitmap = b.subtitle.filter(|index| {
        info.tracks("subtitle").get(*index).is_some_and(|track| {
            matches!(
                track.codec_name.as_str(),
                "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle" | "xsub"
            )
        })
    });
    if let Some(subtitle) = bitmap {
        let base = if info.hdr() {
            "[0:v:0]zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p[base];"
        } else {
            "[0:v:0]null[base];"
        };
        let mut graph = format!(
            "{base}[base][0:s:{subtitle}]overlay=eof_action=pass:repeatlast=0[burn];[burn]split={}",
            variants.len()
        );
        for i in 0..variants.len() {
            graph.push_str(&format!("[input{i}]"));
        }
        for (i, v) in variants.iter().enumerate() {
            graph.push_str(&format!(
                ";[input{i}]scale={}:{}[burn{i}]",
                v.width, v.height
            ));
        }
        a.extend([
            "-filter_complex_threads".into(),
            "2".into(),
            "-filter_complex".into(),
            graph,
        ]);
    }
    for (i, v) in variants.iter().enumerate() {
        a.extend([
            "-map".into(),
            if bitmap.is_some() {
                format!("[burn{i}]")
            } else {
                "0:v:0".into()
            },
            "-map".into(),
            format!("0:a:{}?", b.audio),
        ]);
        if b.mode == "transcode" {
            let mut filters = vec![];
            if info.hdr() {
                filters.push("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p".into());
            }
            if let Some(s) = b.subtitle {
                filters.push(format!("subtitles=filename=source:si={s}"));
            }
            filters.push(format!("scale={}:{}", v.width, v.height));
            if bitmap.is_none() {
                a.extend(["-vf".into(), filters.join(",")]);
            }
            a.extend([
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "veryfast".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-threads".into(),
                "2".into(),
                "-filter_threads".into(),
                "2".into(),
                "-b:v".into(),
                format!("{}k", v.kbps),
                "-maxrate".into(),
                format!("{}k", v.kbps),
                "-bufsize".into(),
                format!("{}k", v.kbps * 2),
                "-force_key_frames".into(),
                "expr:gte(t,n_forced*4)".into(),
            ]);
            if info.hdr() {
                a.extend([
                    "-color_primaries".into(),
                    "bt709".into(),
                    "-color_trc".into(),
                    "bt709".into(),
                    "-colorspace".into(),
                    "bt709".into(),
                ]);
            }
        } else {
            a.extend(["-c:v".into(), "copy".into()]);
        }
        a.extend([
            "-c:a".into(),
            "aac".into(),
            "-ac".into(),
            "2".into(),
            "-b:a".into(),
            "128k".into(),
            "-f".into(),
            "hls".into(),
            "-hls_time".into(),
            "4".into(),
            "-hls_list_size".into(),
            "0".into(),
            "-hls_playlist_type".into(),
            "event".into(),
            "-hls_flags".into(),
            "temp_file".into(),
            "-hls_segment_filename".into(),
            format!("v{i}-%d.ts"),
            format!("v{i}.m3u8"),
        ]);
    }
    a
}
pub fn hardware_args(mut args: Vec<String>, name: &str, variants: &[Rendition]) -> Vec<String> {
    let input = args
        .iter()
        .position(|v| v == "-ss")
        .expect("input arguments");
    args.splice(input..input, crate::hardware::pre(name, true));
    let mut rendition = 0;
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "-vf" {
            let v = &variants[rendition];
            args[i + 1] = crate::hardware::filter(name, v.width, v.height);
            rendition += 1;
        }
        if args[i] == "-c:v" {
            args[i + 1] = crate::hardware::encoder(name).into();
        }
        if ["-preset", "-pix_fmt", "-threads"].contains(&args[i].as_str()) {
            args.drain(i..i + 2);
            continue;
        }
        i += 1;
    }
    args
}
/// Only this explicitly described HDR10 slice has a physical acceptance graph.
/// Unknown color metadata, HLG, Dolby Vision and subtitle burn-in stay in software.
pub fn vaapi_hdr_eligible(info: &Info, b: &Playback) -> bool {
    b.mode == "transcode"
        && b.subtitle.is_none()
        && info.video().is_some_and(|v| {
            v.codec_name == "hevc"
                && v.color_transfer == "smpte2084"
                && v.color_primaries == "bt2020"
                && v.color_space == "bt2020nc"
                && v.color_range == "tv"
                && v.hdr_frame_metadata_checked
                && v.pix_fmt == "yuv420p10le"
                && !["dvhe", "dvh1"].contains(&v.codec_tag_string.as_str())
                && !v.side_data_list.iter().any(|d| {
                    let kind = d.side_data_type.to_ascii_lowercase();
                    kind.contains("dovi") || kind.contains("dolby vision")
                })
                && v.side_data_list
                    .iter()
                    .any(|d| d.side_data_type == "Mastering display metadata")
                && v.side_data_list
                    .iter()
                    .any(|d| d.side_data_type == "Content light level metadata")
        })
}
/// Uses hardware decoding, tone mapping and scaling on VAAPI surfaces. The
/// existing per-session limits, process ownership and pre-master fallback apply.
pub fn vaapi_hdr_args(args: Vec<String>, variants: &[Rendition]) -> Vec<String> {
    let mut args = hardware_args(args, "vaapi", variants);
    for i in 0..args.len().saturating_sub(1) {
        if args[i] == "-vf" {
            args[i + 1] = format!(
                "tonemap_vaapi=format=nv12:primaries=bt709:transfer=bt709:matrix=bt709,{}",
                args[i + 1]
            );
        }
    }
    args
}
async fn publish(dir: &std::path::Path, variants: &[Rendition]) -> bool {
    if tokio::fs::metadata(dir.join("index.m3u8")).await.is_ok() {
        return true;
    }
    for i in 0..variants.len() {
        if tokio::fs::metadata(dir.join(format!("v{i}.m3u8")))
            .await
            .is_err()
        {
            return false;
        }
    }
    let mut master = String::from("#EXTM3U\n#EXT-X-VERSION:3\n");
    for (i, v) in variants.iter().enumerate() {
        let bw = if v.kbps == 0 {
            8_000_000
        } else {
            (v.kbps + 128) * 1200
        };
        master.push_str(&format!("#EXT-X-STREAM-INF:BANDWIDTH={bw}"));
        if v.width > 0 {
            master.push_str(&format!(",RESOLUTION={}x{}", v.width, v.height));
        }
        master.push_str(&format!("\nv{i}.m3u8\n"));
    }
    if tokio::fs::write(dir.join("master.tmp"), master)
        .await
        .is_err()
    {
        return false;
    }
    tokio::fs::rename(dir.join("master.tmp"), dir.join("index.m3u8"))
        .await
        .is_ok()
}
async fn encode(
    app: &Arc<App>,
    st: &Arc<Stream>,
    path: PathBuf,
    b: Playback,
    info: Info,
) -> Result<()> {
    let accelerator = if vaapi_hdr_eligible(&info, &b) {
        crate::hardware::select_hdr(app, &st.dir, &st.cancel).await?
    } else if b.mode == "transcode" && !info.hdr() && b.subtitle.is_none() {
        crate::hardware::select(app).await?
    } else {
        None
    };
    *st.accelerator.lock().await = accelerator.clone();
    tracing::info!(accelerator=?accelerator,hdr=info.hdr(),"playback conversion path");
    let result = encode_attempt(
        app,
        st,
        path.clone(),
        b.clone(),
        info.clone(),
        accelerator.as_deref(),
    )
    .await;
    if result.is_err()
        && accelerator.is_some()
        && !st.cancel.is_cancelled()
        && tokio::fs::metadata(st.dir.join("index.m3u8"))
            .await
            .is_err()
    {
        let mut entries = tokio::fs::read_dir(&st.dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file() {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }
        *st.accelerator.lock().await = None;
        tracing::warn!(accelerator=?accelerator,"hardware conversion failed before publication; retrying CPU");
        return encode_attempt(app, st, path, b, info, None).await;
    }
    result
}
async fn encode_attempt(
    app: &Arc<App>,
    st: &Arc<Stream>,
    path: PathBuf,
    b: Playback,
    info: Info,
    accelerator: Option<&str>,
) -> Result<()> {
    let v = info.video().expect("video checked");
    let variants = if b.mode == "transcode" {
        ladder(v.width, v.height, b.adaptive)
    } else {
        vec![Rendition {
            width: 0,
            height: 0,
            kbps: 0,
        }]
    };
    if b.subtitle.is_some() {
        #[cfg(unix)]
        tokio::fs::symlink(&path, st.dir.join("source")).await?;
        #[cfg(not(unix))]
        return Err(Error::new(
            422,
            "subtitle burn-in requires Unix in this preview",
        ));
    }
    let mut args = encoding_args(&path, &b, &info, &variants);
    if let Some(name) = accelerator {
        args = if info.hdr() {
            vaapi_hdr_args(args, &variants)
        } else {
            hardware_args(args, name, &variants)
        };
    }
    let conversion = media::process(
        app,
        "ffmpeg",
        args,
        Some(st.dir.clone()),
        Duration::from_secs(6 * 3600),
        1024,
        st.cancel.clone(),
    );
    tokio::pin!(conversion);
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut quota = tokio::time::interval(Duration::from_secs(1));
    let started = Instant::now();
    let mut published = false;
    loop {
        tokio::select! {
            result=&mut conversion=>{result?;if !publish(&st.dir,&variants).await{return Err(Error::new(502,"FFmpeg completed without a playlist"));}return Ok(());},
            _=tick.tick()=>{if !published&&publish(&st.dir,&variants).await{published=true;tracing::info!(prepare_ms=started.elapsed().as_millis() as u64,"playback playlist ready");}},
            _=quota.tick()=>{
                let mut size=0u64;let mut entries=tokio::fs::read_dir(&st.dir).await?;
                while let Some(e)=entries.next_entry().await?{if e.file_type().await?.is_file(){size=size.saturating_add(e.metadata().await?.len());}}
                let free=fs2::available_space(&app.config.data)?;
                if size>app.config.transcode_bytes||free<app.config.min_free_bytes||auth::now()-st.touched.load(Ordering::Relaxed)>app.config.stream_idle.as_secs() as i64 {
                    st.cancel.cancel();let _=conversion.await;return Err(Error::new(507,"transcoding stopped: storage or idle limit reached"));
                }
            }
        }
    }
}
async fn owner(app: &Arc<App>, u: User, id: &str) -> Result<Arc<Stream>> {
    let st = app
        .streams
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| Error::new(404, "stream not found"))?;
    if st.owner != u.id {
        return Err(Error::new(404, "stream not found"));
    }
    Ok(st)
}
async fn owned(app: &Arc<App>, u: User, id: &str) -> Result<Arc<Stream>> {
    let st = owner(app, u, id).await?;
    let (_, library) = media::item(app, u, st.item).await?;
    if library != st.library {
        return Err(Error::new(404, "stream not found"));
    }
    st.touched.store(auth::now(), Ordering::Relaxed);
    Ok(st)
}
pub async fn segment(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path((id, name)): Path<(String, String)>,
    req: Request,
) -> Result<Response> {
    let valid = regex::Regex::new(r"^(index\.m3u8|v[0-2]\.m3u8|v[0-2]-[0-9]+\.ts)$")
        .expect("segment expression");
    if !valid.is_match(&name) {
        return Err(Error::new(404, "stream not found"));
    }
    let st = owned(&app, u, &id).await?;
    let path = st.dir.join(name);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        // Recheck grants while waiting for a segment, not just at admission.
        let (_, library) = media::item(&app, u, st.item).await?;
        if library != st.library {
            st.cancel.cancel();
            return Err(Error::new(404, "stream not found"));
        }
        // An authorized request actively waiting for output is not idle.
        st.touched.store(auth::now(), Ordering::Relaxed);
        if tokio::fs::metadata(&path).await.is_ok() {
            return media::serve(&app, path, req).await;
        }
        if st.finished.load(Ordering::Acquire) {
            return Err(Error::new(
                502,
                st.error
                    .lock()
                    .await
                    .clone()
                    .unwrap_or("stream file unavailable".into()),
            ));
        }
        if Instant::now() >= deadline {
            return Err(Error::new(504, "stream preparation timed out"));
        }
        tokio::select! {_=st.cancel.cancelled()=>return Err(Error::new(404,"stream stopped")),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
    }
}
pub async fn stop(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    // Always cancel/reap an owned stream, even if its grants or source were
    // lost. Authorization still controls the response; no media is returned.
    let st = owner(&app, u, &id).await?;
    let mut done = st.done.clone();
    st.cancel.cancel();
    if !*done.borrow() {
        let _ = done.wait_for(|v| *v).await;
    }
    let (_, library) = media::item(&app, u, st.item).await?;
    if library != st.library {
        return Err(Error::new(404, "stream not found"));
    }
    Ok(Json(json!({"ok":true})))
}
