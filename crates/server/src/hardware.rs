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
