//! Runtime smoke tests select candidates, never certify capacity or media support.
use crate::{App, error::Result, media};
use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tokio::sync::OnceCell;
#[derive(Clone, Debug, Serialize)]
pub struct Accelerator {
    pub name: String,
    pub available: bool,
}
static DETECTED: OnceCell<Vec<Accelerator>> = OnceCell::const_new();
pub async fn detect(app: &Arc<App>) -> Result<&'static Vec<Accelerator>> {
    DETECTED
        .get_or_try_init(|| async {
            let _slot = app
                .inspection
                .clone()
                .try_acquire_owned()
                .map_err(|_| crate::error::Error::new(503, "media inspection busy"))?;
            let mut out = vec![];
            for name in ["qsv", "vaapi", "nvenc"] {
                let mut args = vec!["-nostdin".into(), "-v".into(), "error".into()];
                args.extend(pre(name, false));
                args.extend([
                    "-f".into(),
                    "lavfi".into(),
                    "-i".into(),
                    "color=size=128x128:rate=1".into(),
                ]);
                if name != "nvenc" {
                    args.extend([
                        "-vf".into(),
                        "format=nv12,hwupload=extra_hw_frames=32".into(),
                    ]);
                }
                args.extend([
                    "-c:v".into(),
                    encoder(name).into(),
                    "-frames:v".into(),
                    "1".into(),
                    "-f".into(),
                    "null".into(),
                    "-".into(),
                ]);
                let result = media::process(
                    app,
                    "ffmpeg",
                    args,
                    None,
                    Duration::from_secs(8),
                    1024,
                    app.cancel.child_token(),
                )
                .await;
                if app.cancel.is_cancelled() {
                    return Err(crate::error::Error::new(503, "server stopping"));
                }
                out.push(Accelerator {
                    name: name.into(),
                    available: result.is_ok(),
                });
            }
            Ok(out)
        })
        .await
}
pub async fn select(app: &Arc<App>) -> Result<Option<String>> {
    let requested = std::env::var("OVL_ACCELERATOR").unwrap_or("auto".into());
    if requested == "cpu" {
        return Ok(None);
    }
    Ok(detect(app)
        .await?
        .iter()
        .find(|a| a.available && (requested == "auto" || requested == a.name))
        .map(|a| a.name.clone()))
}
pub fn encoder(name: &str) -> &'static str {
    match name {
        "vaapi" => "h264_vaapi",
        "qsv" => "h264_qsv",
        "nvenc" => "h264_nvenc",
        _ => "libx264",
    }
}
pub fn pre(name: &str, decode: bool) -> Vec<String> {
    let mut p: Vec<String> = match name {
        "vaapi" => vec!["-vaapi_device", "/dev/dri/renderD128"],
        "qsv" => vec!["-init_hw_device", "qsv=hw", "-filter_hw_device", "hw"],
        _ => vec![],
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    if decode {
        let kind = if name == "nvenc" { "cuda" } else { name };
        p.extend([
            "-hwaccel".into(),
            kind.into(),
            "-hwaccel_output_format".into(),
            kind.into(),
        ]);
    }
    p
}
pub fn filter(name: &str, w: u32, h: u32) -> String {
    let scaler = match name {
        "qsv" => "scale_qsv",
        "nvenc" => "scale_cuda",
        _ => "scale_vaapi",
    };
    format!("{scaler}=w={w}:h={h}:format=nv12")
}

/// HDR acceleration is deliberately opt-in and never overrides a forced CPU or
/// a different explicitly selected accelerator.
pub fn wants_vaapi_hdr(accelerator: &str, hdr: &str) -> bool {
    hdr == "vaapi" && ["auto", "vaapi"].contains(&accelerator)
}
pub async fn select_hdr(
    app: &Arc<App>,
    workspace: &std::path::Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Option<String>> {
    if !wants_vaapi_hdr(
        &std::env::var("OVL_ACCELERATOR").unwrap_or("auto".into()),
        &std::env::var("OVL_HDR_ACCELERATOR").unwrap_or_default(),
    ) {
        return Ok(None);
    }
    let detection = tokio::select! {
        _ = cancel.cancelled() => return Err(crate::error::Error::new(499,"playback cancelled")),
        result = detect(app) => result,
    };
    let detected = match detection {
        Ok(detected) => detected,
        Err(error) if app.cancel.is_cancelled() => return Err(error),
        Err(error) => {
            tracing::warn!(reason=%error.1,"HDR hardware detection unavailable; using CPU");
            return Ok(None);
        }
    };
    if !detected.iter().any(|a| a.name == "vaapi" && a.available)
        || app.config.transcode_bytes < 1 << 20
    {
        return Ok(None);
    }
    static HDR_QUALITY: OnceCell<bool> = OnceCell::const_new();
    let validation = tokio::select! {
        _ = cancel.cancelled() => return Err(crate::error::Error::new(499,"playback cancelled")),
        result = HDR_QUALITY.get_or_try_init(|| async {
            let _slot = app.inspection.clone().try_acquire_owned()
                .map_err(|_|crate::error::Error::new(503,"media inspection busy"))?;
            validate_hdr_picture(app, workspace, cancel).await
        }) => result,
    };
    match validation {
        Ok(true) => Ok(Some("vaapi".into())),
        Ok(false) => {
            tracing::warn!("HDR hardware picture probe failed; using CPU");
            Ok(None)
        }
        Err(error) if cancel.is_cancelled() || app.cancel.is_cancelled() => Err(error),
        Err(error) => {
            tracing::warn!(reason=%error.1,"HDR hardware picture probe unavailable; using CPU");
            Ok(None)
        }
    }
}

/// Reject silent all-black/corrupt HDR drivers using calibrated PQ luminance.
/// This runtime guard is deliberately smaller than physical corpus acceptance.
pub fn hdr_probe_picture_valid(rgb: &[u8]) -> bool {
    if rgb.len() != 128 * 128 * 3 {
        return false;
    }
    let samples: Vec<_> = [16, 48, 80, 112]
        .map(|x| {
            let start = (64 * 128 + x) * 3;
            &rgb[start..start + 3]
        })
        .into();
    if samples
        .iter()
        .any(|p| p.iter().max().unwrap() - p.iter().min().unwrap() > 5)
    {
        return false;
    }
    let gray: Vec<_> = samples
        .iter()
        .map(|p| p.iter().map(|v| f64::from(*v)).sum::<f64>() / 3.0)
        .collect();
    gray[0] < 8.0
        && gray[1] > gray[0] + 20.0
        && gray[2] > gray[1] + 20.0
        && (80.0..240.0).contains(&gray[2])
        && gray[3] > 180.0
        && gray[3] > gray[2] + 10.0
}
/// Run only in the admitted session's owned workspace. At most 1 MiB is
/// budgeted; all subprocesses have eight-second deadlines and session cancellation.
/// Drivers can report successful encoding while outputting black video.
pub async fn validate_hdr_picture(
    app: &Arc<App>,
    workspace: &std::path::Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<bool> {
    if fs2::available_space(&app.config.data)? < app.config.min_free_bytes.saturating_add(1 << 20) {
        return Ok(false);
    }
    match probe_hdr_picture(app, workspace, cancel).await {
        Ok(available) => Ok(available),
        Err(error) if error.0.as_u16() == 499 => Err(error),
        Err(error) => {
            tracing::warn!(reason=%error.1,"HDR hardware picture probe failed or rejected the driver; using CPU");
            Ok(false)
        }
    }
}
/// The HDR picture probe without fallback: `Ok(false)` means fixture generation
/// and the VAAPI encode ran and the decoded picture failed validation; an error
/// means a step (such as libx265 fixture generation or the hardware encode)
/// did not complete.
pub async fn probe_hdr_picture(
    app: &Arc<App>,
    workspace: &std::path::Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<bool> {
    let dir = workspace.join("hdr-probe");
    tokio::fs::create_dir(&dir).await?;
    let result: Result<bool> = async {
        // Limited-range PQ codes for 0, 10, 100 and 1000 nits, respectively.
        let mut raw = Vec::with_capacity(128 * 128 * 3);
        for _ in 0..128 {for x in 0..128 {raw.extend([64u16,327,509,723][x/32].to_le_bytes());}}
        for _ in 0..128*128/2 {raw.extend(512u16.to_le_bytes());}
        let input = dir.join("signal.yuv");
        tokio::fs::write(&input,raw).await?;
        let source = dir.join("source.mp4");
        let mut args:Vec<String>=["-nostdin","-v","error","-f","rawvideo","-pixel_format","yuv420p10le","-video_size","128x128","-framerate","1","-i"].into_iter().map(str::to_owned).collect();
        args.push(input.to_string_lossy().into_owned());
        args.extend(["-frames:v","1","-c:v","libx265","-preset","ultrafast","-threads","2","-x265-params","pools=1:frame-threads=1:log-level=error:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400","-pix_fmt","yuv420p10le","-color_primaries","bt2020","-color_trc","smpte2084","-colorspace","bt2020nc","-color_range","tv"].into_iter().map(str::to_owned));
        args.push(source.to_string_lossy().into_owned());
        media::process(app,"ffmpeg",args,Some(dir.clone()),Duration::from_secs(8),1024,cancel.clone()).await?;
        let info = media::Info {streams:vec![media::Track {codec_type:"video".into(),color_transfer:"smpte2084".into(),..Default::default()}],..Default::default()};
        let variants = [crate::playback::Rendition {width:128,height:128,kbps:500}];
        let args = crate::playback::vaapi_hdr_args(crate::playback::encoding_args(&source,&crate::playback::Playback {mode:"transcode".into(),..Default::default()},&info,&variants),&variants);
        media::process(app,"ffmpeg",args,Some(dir.clone()),Duration::from_secs(8),1024,cancel.clone()).await?;
        let playlist = dir.join("v0.m3u8").to_string_lossy().into_owned();
        let args:Vec<String>=["-nostdin","-v","error","-i",playlist.as_str(),"-frames:v","1","-pix_fmt","rgb24","-f","rawvideo","pipe:1"].into_iter().map(str::to_owned).collect();
        let rgb=media::process(app,"ffmpeg",args,Some(dir.clone()),Duration::from_secs(8),128*128*3,cancel.clone()).await?;
        Ok(hdr_probe_picture_valid(&rgb))
    }.await;
    let cleanup = tokio::fs::remove_dir_all(&dir).await;
    if cancel.is_cancelled() || app.cancel.is_cancelled() {
        return Err(crate::error::Error::new(499, "playback cancelled"));
    }
    cleanup?;
    result
}
