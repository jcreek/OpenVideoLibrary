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
/// Placeholder for the text-subtitle filter's source, bound to an anchored
/// handle by [`bind_input_args`]. Residual risk: libass's `subtitles` filter
/// opens that handle without FFmpeg's input whitelists. It reads the same
/// inode as the confined, already probed input, so escaping needs library
/// write access and a content swap between probe and filter start.
const SUBTITLE_SOURCE: &str = "subtitles=filename=source:";
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientCapabilities {
    pub containers: Vec<String>,
    pub video_codecs: Vec<String>,
    pub audio_codecs: Vec<String>,
    #[serde(default)]
    pub max_audio_channels: Option<u8>,
    #[serde(default)]
    pub max_audio_sample_rate: Option<u32>,
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
    /// Server-resolved video keyframe at or before `start`, used as the input
    /// seek; transcodes trim to `start` after decoding. Never client-supplied.
    #[serde(skip)]
    pub keyframe: Option<f64>,
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
fn channel_limit(capabilities: Option<&ClientCapabilities>) -> u32 {
    u32::from(capabilities.and_then(|c| c.max_audio_channels).unwrap_or(2))
}
fn audio_limits_supported(track: &media::Track, capabilities: Option<&ClientCapabilities>) -> bool {
    track.channels > 0
        && track.channels <= channel_limit(capabilities)
        && capabilities
            .and_then(|c| c.max_audio_sample_rate)
            .is_none_or(|max| {
                track
                    .sample_rate
                    .parse::<u32>()
                    .is_ok_and(|hz| hz > 0 && hz <= max)
            })
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
        let audio_ok = audio.get(b.audio).is_none_or(|a| {
            c.audio_codecs.contains(&a.codec_name) && audio_limits_supported(a, Some(c))
        });
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
    let audio_ok = audio.get(b.audio).is_none_or(|a| {
        ["aac", "mp3"].contains(&a.codec_name.as_str()) && audio_limits_supported(a, None)
    });
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
    let (path, library, kind) = media::item_with_kind(&app, u, item_id).await?;
    let mut b: Playback = parse(&body)?;
    if b.capabilities
        .as_ref()
        .and_then(|c| c.max_audio_channels)
        .is_some_and(|n| !(1..=8).contains(&n))
    {
        return Err(Error::new(
            400,
            "max_audio_channels must be between 1 and 8",
        ));
    }
    if b.capabilities
        .as_ref()
        .and_then(|c| c.max_audio_sample_rate)
        .is_some_and(|rate| !(8_000..=384_000).contains(&rate))
    {
        return Err(Error::new(
            400,
            "max_audio_sample_rate must be between 8000 and 384000 Hz",
        ));
    }
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
    if kind == "music" {
        if b.audio != 0 || b.subtitle.is_some() {
            return Err(Error::new(400, "music playback uses the first audio track"));
        }
        if b.mode == "remux" {
            return Err(Error::new(422, "music remux is unavailable"));
        }
        info = media::inspect_media(&app, &path).await?;
        let audio = info.streams.iter().find(|t| t.codec_type == "audio");
        let Some(audio) = audio.filter(|_| info.video().is_none()) else {
            return Err(Error::new(422, "expected an audio-only music track"));
        };
        if b.mode == "auto" {
            let supported = if let Some(c) = &b.capabilities {
                matches!(
                    (info.format.format_name.as_str(), audio.codec_name.as_str()),
                    ("mp3", "mp3")
                        | ("m4a", "aac")
                        | ("flac", "flac")
                        | ("ogg", "vorbis" | "opus")
                        | ("wav", "pcm_s16le")
                ) && c.containers.contains(&info.format.format_name)
                    && c.audio_codecs.contains(&audio.codec_name)
                    && audio_limits_supported(audio, Some(c))
            } else {
                info.format.format_name == "mp3"
                    && audio.codec_name == "mp3"
                    && audio_limits_supported(audio, None)
            };
            b.mode = if supported { "direct" } else { "transcode" }.into();
        }
        if b.mode == "direct" {
            reason = "Direct music playback".into();
        } else {
            if b.capabilities.as_ref().is_none_or(|c| {
                !c.containers.iter().any(|container| container == "hls")
                    || !c.audio_codecs.iter().any(|codec| codec == "aac")
            }) {
                return Err(Error::new(
                    422,
                    "client does not support AAC HLS music playback",
                ));
            }
            reason = "AAC HLS music conversion".into();
        }
    } else if b.mode != "direct" {
        info = media::inspect_media(&app, &path).await?;
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
            if b.mode != "direct"
                && let Some(c) = &b.capabilities
            {
                let h264_supported = c.video_codecs.iter().any(|codec| codec == "h264");
                let output_format_supported = b.mode == "remux"
                    || c.video_pixel_formats
                        .get("h264")
                        .is_none_or(|formats| formats.iter().any(|format| format == "yuv420p"));
                if !h264_supported || !output_format_supported {
                    return Err(Error::new(
                        422,
                        "Client does not support the H.264 playback fallback",
                    ));
                }
                if !info.tracks("audio").is_empty()
                    && !c.audio_codecs.iter().any(|codec| codec == "aac")
                {
                    return Err(Error::new(
                        422,
                        "Client does not support the AAC playback fallback",
                    ));
                }
            }
        }
    }
    if b.subtitle.is_some() && b.mode != "transcode" {
        return Err(Error::new(400, "subtitle burn-in requires transcoding"));
    }
    if matches!(b.mode.as_str(), "remux" | "transcode")
        && kind != "music"
        && b.start > 0.0
        && unindexed_seek(&info.format.format_name)
        // Sequential bitmap burn-in decodes from the start and ignores a keyframe.
        && !sequential_bitmap(&b, &info)
    {
        // MPEG-TS has no seek index: FFmpeg's input seek can land after the
        // preceding keyframe and start at the next one, up to a GOP late. Seek the
        // input onto the preceding keyframe; transcodes trim to the requested
        // start after decoding. Other containers keep FFmpeg's accurate seek.
        match media::keyframe_at_or_before(&app, &path, &info, b.start).await {
            // Round up so the input seek cannot fall back to the previous GOP.
            Ok(Some(keyframe)) => b.keyframe = Some((keyframe * 1000.0).ceil() / 1000.0),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error=%e,"keyframe lookup failed; keeping requested start")
            }
        }
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
    let (mode, mut start, keyframe) = (b.mode.clone(), b.start, b.keyframe);
    let origin = info
        .format
        .start_time
        .parse::<f64>()
        .ok()
        .filter(|t| t.is_finite())
        .unwrap_or(0.0);
    let measured = stream.clone();
    let session_id = id.clone();
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
    // A client that disconnects while this handler waits never learns the
    // session id; cancel the session then so FFmpeg is reaped and the slot freed.
    let abandon = measured.cancel.clone().drop_guard();
    if mode == "remux" && start > 0.0 {
        // Stream copy begins on a keyframe chosen by FFmpeg's container-specific
        // seek. Remux keeps source timestamps, so report where the first published
        // segment actually begins; the client offsets its timeline by it.
        let segment = measured.dir.join("v0-0.ts");
        let ready = tokio::time::timeout(Duration::from_secs(20), async {
            while !tokio::fs::try_exists(&segment).await.unwrap_or(false) {
                if measured.finished.load(Ordering::Acquire) || measured.cancel.is_cancelled() {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            true
        })
        .await
        .unwrap_or(false);
        let actual = if ready {
            media::segment_start(&app, &segment, origin)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error=%e,"remux start probe failed");
                    None
                })
        } else {
            None
        };
        let reported = remux_start(start, keyframe, actual);
        if actual.is_none_or(|a| (a * 1000.0).floor() / 1000.0 != reported) {
            tracing::warn!(requested=start,measured=?actual,reported,"remux start not measured; using fallback");
        }
        start = reported;
        measured.touched.store(auth::now(), Ordering::Relaxed);
    }
    abandon.disarm();
    let response = json!({"id":session_id,"url":format!("/api/v1/playback/{session_id}/index.m3u8"),"mode":mode,"start":start,"reason":reason});
    Ok((StatusCode::CREATED, Json(response)))
}
/// Containers without a seek index, where FFmpeg's input seek can land after
/// the preceding keyframe and start at the next one (MPEG-TS, MPEG program
/// streams and elementary MPEG video).
pub fn unindexed_seek(format_name: &str) -> bool {
    format_name
        .split(',')
        .any(|f| matches!(f, "mpegts" | "mpeg" | "mpegvideo"))
}
/// The selected subtitle track when it is a bitmap format that must be burned in.
fn bitmap_subtitle(b: &Playback, info: &Info) -> Option<usize> {
    b.subtitle.filter(|index| {
        info.tracks("subtitle").get(*index).is_some_and(|track| {
            matches!(
                track.codec_name.as_str(),
                "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle" | "xsub"
            )
        })
    })
}
/// Bitmap burn-in from an unindexed container composes from the file start
/// with a single input and trims the output to the requested start.
fn sequential_bitmap(b: &Playback, info: &Info) -> bool {
    bitmap_subtitle(b, info).is_some() && unindexed_seek(&info.format.format_name)
}
/// Chooses the remux start reported to the client. Copied video begins on a
/// keyframe at or before the input seek, so a measured start is accepted only
/// between 0 and `requested` + 0.1 s; anything else (for example timestamps
/// shifted by MPEG-TS wrap-around or the muxer) falls back to the resolved
/// keyframe or the requested start.
pub fn remux_start(requested: f64, keyframe: Option<f64>, measured: Option<f64>) -> f64 {
    let fallback = keyframe
        .filter(|k| *k <= requested + 0.001)
        .unwrap_or(requested);
    measured
        .filter(|m| m.is_finite() && *m >= 0.0 && *m <= requested + 0.1)
        .map_or(fallback, |m| (m * 1000.0).floor() / 1000.0)
}
pub fn encoding_args(
    path: &std::path::Path,
    b: &Playback,
    info: &Info,
    variants: &[Rendition],
) -> Vec<String> {
    let video_map = format!("0:{}", info.video().map_or(0, |v| v.index));
    let bitmap = bitmap_subtitle(b, info);
    // MPEG-TS input seeks/probed origins can discard DVB state or yield no frames,
    // and a second program-stream input shows DVD cues from the stream start.
    // Their correctness path composes sequentially and trims the complete output.
    let sequential_bitmap = sequential_bitmap(b, info);
    // Input seeking starts decoding at a resolved keyframe; transcodes then trim
    // the remaining gap per output so the first output frame is `start`.
    let input_seek = if sequential_bitmap {
        0.0
    } else {
        b.keyframe
            .filter(|k| *k <= b.start + 0.001)
            .unwrap_or(b.start)
    };
    // One millisecond value, rounded up so a seek onto a keyframe can never fall
    // before it, drives the input seek and every subtitle timeline offset.
    let input_seek = (input_seek * 1000.0).ceil() / 1000.0;
    let output_seek = if sequential_bitmap {
        Some(b.start)
    } else {
        Some(b.start - input_seek).filter(|gap| b.mode == "transcode" && *gap >= 0.001)
    };
    let mut a = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
    ];
    if b.mode == "remux" {
        // Keep source timestamps so the server can report the actual start.
        a.push("-copyts".into());
    }
    // Seeking to 0 is a no-op elsewhere but fails on unindexed FLV, where the
    // demuxer then drops leading audio; FFmpeg rebases timestamps either way.
    if input_seek > 0.0 {
        a.extend(["-ss".into(), format!("{input_seek:.3}")]);
    }
    a.extend(["-i".into(), path.to_string_lossy().into_owned()]);
    if info.video().is_none() {
        a.extend(["-map".into(), "0:a:0".into(), "-vn".into()]);
        a.extend(hls_audio_args(b, info.tracks("audio").first().copied(), 0));
        return a;
    }
    if let Some(subtitle) = bitmap {
        if !sequential_bitmap {
            // Only the second input's subtitle is mapped, leaving its other streams
            // undecoded. Preserve full probing so both inputs retain the same origin.
            // Both inputs use the same permission-checked canonical path.
            a.extend(["-i".into(), path.to_string_lossy().into_owned()]);
        }
        let base = if info.hdr() {
            format!(
                "[{video_map}]zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p[base];"
            )
        } else {
            format!("[{video_map}]null[base];")
        };
        let mut graph = format!(
            "[{}:s:{subtitle}]setpts=PTS-{:.3}/TB[subs];{base}[base][subs]overlay=eof_action=pass:repeatlast=0[burn];[burn]split={}",
            if sequential_bitmap { 0 } else { 1 },
            input_seek,
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
        if let Some(gap) = output_seek {
            a.extend(["-ss".into(), format!("{gap:.3}")]);
        }
        a.extend([
            "-map".into(),
            if bitmap.is_some() {
                format!("[burn{i}]")
            } else {
                video_map.clone()
            },
            "-map".into(),
            format!("0:a:{}?", b.audio),
        ]);
        if b.mode == "transcode" {
            let mut filters = vec![];
            if info.hdr() {
                filters.push("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p".into());
            }
            if let Some(s) = b.subtitle.filter(|_| bitmap.is_none()) {
                let origin = info
                    .format
                    .start_time
                    .parse::<f64>()
                    .ok()
                    .filter(|t| t.is_finite())
                    .unwrap_or(0.0);
                let subtitle_offset = origin + input_seek;
                // Input seeking rebases video PTS, while libass reads original cue times.
                // Restore that timeline only around subtitle rendering, then preserve AV sync.
                filters.push(format!("setpts=PTS+({subtitle_offset:.6})/TB"));
                filters.push(format!("{SUBTITLE_SOURCE}si={s}"));
                filters.push(format!("setpts=PTS-({subtitle_offset:.6})/TB"));
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
            // Zero mux delay keeps segment timestamps equal to source timestamps.
            a.extend([
                "-c:v".into(),
                "copy".into(),
                "-muxdelay".into(),
                "0".into(),
                "-muxpreload".into(),
                "0".into(),
            ]);
        }
        a.extend(hls_audio_args(
            b,
            info.tracks("audio").get(b.audio).copied(),
            i,
        ));
    }
    a
}
fn hls_audio_args(b: &Playback, audio: Option<&media::Track>, i: usize) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(max) = b
        .capabilities
        .as_ref()
        .and_then(|c| c.max_audio_sample_rate)
    {
        let input = audio
            .and_then(|track| track.sample_rate.parse::<u32>().ok())
            .unwrap_or(max);
        let ceiling = max.min(input).max(8_000);
        let rate = [
            96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
            8_000,
        ]
        .into_iter()
        .find(|rate| *rate <= ceiling)
        .unwrap_or(8_000);
        args.extend(["-ar".into(), rate.to_string()]);
    }
    args.extend([
        "-c:a".into(),
        "aac".into(),
        "-ac".into(),
        channel_limit(b.capabilities.as_ref())
            .clamp(1, 2)
            .to_string(),
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
    args
}
pub fn hardware_args(mut args: Vec<String>, name: &str, variants: &[Rendition]) -> Vec<String> {
    // Hardware options precede the first input's options (its seek, if any).
    let input = args
        .iter()
        .position(|v| v == "-ss" || v == "-i")
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
        } else if v.width == 0 {
            v.kbps * 1250
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
    } else if b.mode == "transcode" && info.video().is_some() && !info.hdr() && b.subtitle.is_none()
    {
        // A DELETE during cold detection stops waiting; dropping the detection
        // future cancels its probe, whose tracked task kills and reaps it.
        let detection = tokio::select! {
            _ = st.cancel.cancelled() => return Err(Error::new(499, "playback cancelled")),
            result = crate::hardware::select(app) => result,
        };
        match detection {
            Ok(accelerator) => accelerator,
            Err(error) if app.cancel.is_cancelled() => return Err(error),
            Err(error) => {
                tracing::warn!(reason=%error.1,"hardware detection unavailable; using CPU");
                None
            }
        }
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
    let variants = if let Some(v) = info.video().filter(|_| b.mode == "transcode") {
        ladder(v.width, v.height, b.adaptive)
    } else {
        vec![Rendition {
            width: 0,
            height: 0,
            kbps: if info.video().is_none() { 128 } else { 0 },
        }]
    };
    let mut args = encoding_args(&path, &b, &info, &variants);
    if let Some(name) = accelerator {
        args = if info.hdr() {
            vaapi_hdr_args(args, &variants)
        } else {
            hardware_args(args, name, &variants)
        };
    }
    let (args, files) = bind_media_inputs(app, args, &path).await?;
    let conversion = media::process_media(
        app,
        "ffmpeg",
        args,
        Some(st.dir.clone()),
        Duration::from_secs(6 * 3600),
        1024,
        st.cancel.clone(),
        files,
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
                if let Err(error) = check_conversion_quota(app, st).await {
                    st.cancel.cancel();
                    let _ = conversion.await;
                    return Err(error);
                }
            }
        }
    }
}
/// Rewrites every library input of an encode, and the subtitle filter's read
/// of the same source, onto inherited handles with confined demuxing; returns
/// the rewritten arguments and how many handles they name. Each consumer gets
/// its own handle, so no two readers share a file offset. Fails closed: any
/// input left on a path rather than an inherited handle is an error.
pub fn bind_input_args(args: Vec<String>, path: &std::path::Path) -> Result<(Vec<String>, usize)> {
    let source = path.to_string_lossy().into_owned();
    let mut bound = Vec::with_capacity(args.len() + 8);
    let mut handles = 0;
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        if arg == "-i" {
            if args.next().as_deref() != Some(source.as_str()) {
                return Err(Error::new(500, "encode input is not the authorized media"));
            }
            bound.extend(media::confined_input(media::MEDIA_DEMUXERS));
            bound.push(arg);
            bound.push(media::inherited(handles));
            handles += 1;
        } else if arg.contains(SUBTITLE_SOURCE) {
            let mut rewritten = String::with_capacity(arg.len());
            let mut rest = arg.as_str();
            while let Some(at) = rest.find(SUBTITLE_SOURCE) {
                rewritten.push_str(&rest[..at]);
                rewritten.push_str(&format!(
                    "subtitles=filename={}:",
                    media::inherited(handles)
                ));
                handles += 1;
                rest = &rest[at + SUBTITLE_SOURCE.len()..];
            }
            rewritten.push_str(rest);
            bound.push(rewritten);
        } else {
            bound.push(arg);
        }
    }
    if handles > media::MAX_INHERITED {
        return Err(Error::new(500, "too many media handles"));
    }
    Ok((bound, handles))
}
/// Opens one anchored handle per input named by [`bind_input_args`].
async fn bind_media_inputs(
    app: &App,
    args: Vec<String>,
    path: &std::path::Path,
) -> Result<(Vec<String>, Vec<std::fs::File>)> {
    let (bound, handles) = bind_input_args(args, path)?;
    let mut files = Vec::with_capacity(handles);
    for _ in 0..handles {
        files.push(media::open_media(app, path).await?.0);
    }
    Ok((bound, files))
}
async fn check_conversion_quota(app: &App, stream: &Stream) -> Result<()> {
    let mut size = 0u64;
    let mut entries = tokio::fs::read_dir(&stream.dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        match entry.metadata().await {
            Ok(metadata) if metadata.is_file() => size = size.saturating_add(metadata.len()),
            Ok(_) => {}
            // FFmpeg publishes segments by renaming temporary files.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let free = fs2::available_space(&app.config.data)?;
    let idle = auth::now() - stream.touched.load(Ordering::Relaxed);
    if size > app.config.transcode_bytes
        || free < app.config.min_free_bytes
        || idle > app.config.stream_idle.as_secs() as i64
    {
        return Err(Error::new(
            507,
            "transcoding stopped: storage or idle limit reached",
        ));
    }
    Ok(())
}
/// Whether a session playlist is complete: the master once every variant it
/// lists has `#EXT-X-ENDLIST`, a variant once it has the tag itself.
async fn playlist_complete(dir: &std::path::Path, name: &str) -> bool {
    async fn ended(path: std::path::PathBuf) -> bool {
        tokio::fs::read_to_string(path)
            .await
            .is_ok_and(|text| text.lines().any(|line| line.trim() == "#EXT-X-ENDLIST"))
    }
    if name != "index.m3u8" {
        return ended(dir.join(name)).await;
    }
    let Ok(master) = tokio::fs::read_to_string(dir.join(name)).await else {
        return false;
    };
    let variants: Vec<String> = master
        .lines()
        .filter(|line| line.starts_with('v') && line.ends_with(".m3u8"))
        .map(str::to_owned)
        .collect();
    if variants.is_empty() {
        return false;
    }
    for variant in variants {
        if !ended(dir.join(variant)).await {
            return false;
        }
    }
    true
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
    let path = st.dir.join(&name);
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
        // Observe completion before checking the path. The encoder can publish
        // between a failed lookup and the completion load.
        let finished = st.finished.load(Ordering::Acquire);
        // A failed encode can leave an unfinished EVENT playlist. Report the
        // failure on its reloads so players stop waiting for segments that
        // will never appear. A playlist the encoder completed (ENDLIST) stays
        // servable, as do published segments.
        if finished
            && name.ends_with(".m3u8")
            && let Some(error) = st.error.lock().await.clone()
            && !playlist_complete(&st.dir, &name).await
        {
            return Err(Error::new(502, error));
        }
        if tokio::fs::metadata(&path).await.is_ok() {
            return media::serve(&app, path, req).await;
        }
        if finished {
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

#[cfg(test)]
mod playlist_tests {
    use super::playlist_complete;

    #[tokio::test]
    async fn only_unfinished_playlists_count_as_incomplete() {
        let dir = tempfile::TempDir::new().unwrap();
        let dir = dir.path();
        assert!(!playlist_complete(dir, "index.m3u8").await);
        std::fs::write(dir.join("index.m3u8"), "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv0.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2\nv1.m3u8\n").unwrap();
        std::fs::write(
            dir.join("v0.m3u8"),
            "#EXTM3U\n#EXTINF:4,\nv0-0.ts\n#EXT-X-ENDLIST\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("v1.m3u8"),
            "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXTINF:4,\nv1-0.ts\n",
        )
        .unwrap();
        assert!(playlist_complete(dir, "v0.m3u8").await);
        assert!(!playlist_complete(dir, "v1.m3u8").await);
        assert!(
            !playlist_complete(dir, "index.m3u8").await,
            "one variant unfinished"
        );
        std::fs::write(
            dir.join("v1.m3u8"),
            "#EXTM3U\n#EXTINF:4,\nv1-0.ts\n#EXT-X-ENDLIST\n",
        )
        .unwrap();
        assert!(playlist_complete(dir, "index.m3u8").await);
        assert!(!playlist_complete(dir, "v2.m3u8").await, "missing playlist");
    }
}
