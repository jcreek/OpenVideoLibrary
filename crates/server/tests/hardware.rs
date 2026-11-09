//! Explicit physical GPU acceptance; never included in the CPU media suite.
#![cfg(feature = "gpu-tests")]
use ovl_server::{App, Config, media, playback};
use std::{path::PathBuf, time::Duration};
use tempfile::TempDir;

#[tokio::test]
#[ignore = "physical GPU required: OVL_ACCELERATOR=vaapi cargo test --features gpu-tests --test hardware -- --ignored --nocapture"]
async fn physical_sdr_decode_scale_encode_quality_and_audio() {
    let accelerator =
        std::env::var("OVL_ACCELERATOR").expect("choose a physical accelerator explicitly");
    assert!(["vaapi", "qsv", "nvenc"].contains(&accelerator.as_str()));
    let root = TempDir::new().unwrap();
    let app = App::open(Config {
        data: root.path().join("data"),
        listen: "127.0.0.1:0".into(),
        web: PathBuf::new(),
        secure_cookies: false,
        max_streams: 2,
        transcode_bytes: 32 << 20,
        min_free_bytes: 1 << 20,
        scan_interval: Duration::from_secs(3600),
        stream_idle: Duration::from_secs(60),
    })
    .unwrap();
    let source = root.path().join("source.mp4");
    let mut fixture: Vec<String> = [
        "-nostdin",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=24",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=880",
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-map",
        "2:a",
        "-t",
        "4",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-threads",
        "2",
        "-c:a",
        "aac",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    fixture.push(source.to_string_lossy().into_owned());
    media::process(
        &app,
        "ffmpeg",
        fixture,
        None,
        Duration::from_secs(30),
        1024,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    let info = media::inspect(&app, &source).await.unwrap();
    let options = playback::Playback {
        mode: "transcode".into(),
        audio: 1,
        start: 0.5,
        ..Default::default()
    };
    let renditions = vec![playback::Rendition {
        width: 320,
        height: 180,
        kbps: 1000,
    }];
    let args = playback::hardware_args(
        playback::encoding_args(&source, &options, &info, &renditions),
        &accelerator,
        &renditions,
    );
    // No selection or CPU fallback here: this must execute the actual HW graph.
    assert!(
        args.iter()
            .any(|v| v == ovl_server::hardware::encoder(&accelerator))
    );
    media::process(
        &app,
        "ffmpeg",
        args,
        Some(root.path().to_owned()),
        Duration::from_secs(30),
        1024,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    let playlist = root.path().join("v0.m3u8");
    let output = media::inspect(&app, &playlist).await.unwrap();
    let video = output.video().unwrap();
    assert_eq!((video.width, video.height), (320, 180));
    assert_eq!(video.codec_name, "h264");
    let audio_args = vec![
        "-v".into(),
        "error".into(),
        "-i".into(),
        playlist.to_string_lossy().into_owned(),
        "-map".into(),
        "0:a:0".into(),
        "-t".into(),
        "1".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        "48000".into(),
        "-f".into(),
        "f32le".into(),
        "pipe:1".into(),
    ];
    let pcm = media::process(
        &app,
        "ffmpeg",
        audio_args,
        None,
        Duration::from_secs(15),
        1 << 20,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    let samples: Vec<f32> = pcm
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| f32::from_le_bytes(*v))
        .collect();
    assert!(samples.len() > 40000);
    // AAC ringing around silence creates spurious zero crossings. Measure the
    // dominant spectral bin and require signal energy instead.
    let rms =
        (samples.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();
    assert!(rms > 0.01, "silent or corrupt selected audio");
    let power = |hz: u32| {
        let coefficient = 2.0 * (2.0 * std::f64::consts::PI * f64::from(hz) / 48000.0).cos();
        let (mut previous, mut earlier) = (0.0, 0.0);
        for sample in &samples {
            let current = f64::from(*sample) + coefficient * previous - earlier;
            earlier = previous;
            previous = current;
        }
        previous * previous + earlier * earlier - coefficient * previous * earlier
    };
    let hz = (400..=1000)
        .step_by(2)
        .max_by(|a, b| power(*a).total_cmp(&power(*b)))
        .unwrap();
    assert!(
        hz.abs_diff(880) < 6,
        "selected audio spectral frequency: {hz}"
    );
    let reference = root.path().join("reference.mp4");
    let args: Vec<String> = vec![
        "-v",
        "error",
        "-ss",
        "0.5",
        "-i",
        source.to_str().unwrap(),
        "-an",
        "-vf",
        "scale=320:180",
        "-c:v",
        "libx264",
        "-crf",
        "0",
        "-threads",
        "2",
        reference.to_str().unwrap(),
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    media::process(
        &app,
        "ffmpeg",
        args,
        None,
        Duration::from_secs(30),
        1024,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    let args: Vec<String> = vec![
        "-v",
        "error",
        "-i",
        reference.to_str().unwrap(),
        "-i",
        playlist.to_str().unwrap(),
        "-lavfi",
        "[0:v]setpts=PTS-STARTPTS[r];[1:v]setpts=PTS-STARTPTS[o];[r][o]ssim=stats_file=quality.log",
        "-an",
        "-f",
        "null",
        "-",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    media::process(
        &app,
        "ffmpeg",
        args,
        Some(root.path().to_owned()),
        Duration::from_secs(30),
        1024,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    let log = std::fs::read_to_string(root.path().join("quality.log")).unwrap();
    let scores: Vec<f64> = log
        .lines()
        .map(|line| {
            line.split_whitespace()
                .find_map(|v| v.strip_prefix("All:"))
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert!(scores.len() >= 70, "decoded frame count {}", scores.len());
    let ssim = scores.iter().sum::<f64>() / scores.len() as f64;
    assert!(ssim > 0.90, "mean SSIM {ssim}");
    println!(
        "accelerator={accelerator} mean_ssim={ssim:.6} frames={} selected_audio_hz={hz}",
        scores.len()
    );
    app.shutdown().await;
}
