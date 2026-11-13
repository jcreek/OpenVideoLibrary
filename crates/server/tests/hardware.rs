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
    let info = media::inspect_with_hdr_metadata(&app, &source, true)
        .await
        .unwrap();
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

/// ST 2084 absolute luminance -> nonlinear PQ code, normalized to 10,000 nits.
fn pq(nits: f64) -> f64 {
    let power = (nits / 10_000.0).powf(2610.0 / 16384.0);
    ((3424.0 / 4096.0 + (2413.0 / 128.0) * power) / (1.0 + (2392.0 / 128.0) * power))
        .powf(2523.0 / 32.0)
}
fn hdr_patches() -> Vec<[f64; 3]> {
    let mut patches: Vec<_> = [0.0, 0.1, 1.0, 10.0, 50.0, 100.0, 203.0, 400.0, 1000.0]
        .map(|n| [n; 3])
        .into();
    patches.extend([[100.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 100.0]]);
    patches
}
/// A true PQ/BT.2020 signal, not an SDR pattern with HDR tags. First second is
/// black; the remaining two seconds contain 4x3 calibrated luminance/color tiles.
fn write_hdr_signal(path: &std::path::Path) {
    use std::io::Write;
    let patches = hdr_patches();
    let mut pattern = Vec::with_capacity(320 * 180 * 3);
    for plane in 0..3 {
        let (width, height) = if plane == 0 { (320, 180) } else { (160, 90) };
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = patches[(y / (height / 3)) * 4 + x / (width / 4)].map(pq);
                let luma = 0.2627 * r + 0.6780 * g + 0.0593 * b;
                let code = match plane {
                    0 => 64.0 + 876.0 * luma,
                    1 => 512.0 + 896.0 * (b - luma) / 1.8814,
                    _ => 512.0 + 896.0 * (r - luma) / 1.4746,
                }
                .round() as u16;
                pattern.extend(code.to_le_bytes());
            }
        }
    }
    let black: Vec<u8> = (0..320 * 180 * 3 / 2)
        .flat_map(|i| (if i < 320 * 180 { 64u16 } else { 512u16 }).to_le_bytes())
        .collect();
    let mut file = std::fs::File::create(path).unwrap();
    for frame in 0..36 {
        file.write_all(if frame < 12 { &black } else { &pattern })
            .unwrap();
    }
}
fn verify_hdr_picture(rgb: &[u8], width: usize, height: usize) -> Vec<[f64; 3]> {
    let frame_size = width * height * 3;
    assert_eq!(rgb.len() % frame_size, 0);
    assert!(
        rgb.len() / frame_size >= 18,
        "missing decoded seek output frames"
    );
    let mut means = vec![];
    for tile in 0..12 {
        let cx = (tile % 4 * 2 + 1) * width / 8;
        let cy = (tile / 4 * 2 + 1) * height / 6;
        let mut mean = [0.0; 3];
        for y in cy - 4..cy + 4 {
            for x in cx - 4..cx + 4 {
                for channel in 0..3 {
                    mean[channel] += f64::from(rgb[(y * width + x) * 3 + channel]) / 64.0;
                }
            }
        }
        means.push(mean);
    }
    for tile in &means[..9] {
        assert!(
            tile.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - tile.iter().copied().fold(f64::INFINITY, f64::min)
                < 5.0,
            "neutral HDR tile became colored: {tile:?}"
        );
    }
    let gray: Vec<_> = means[..9]
        .iter()
        .map(|v| v.iter().sum::<f64>() / 3.0)
        .collect();
    assert!(gray[0] < 8.0, "black lifted: {gray:?}");
    for pair in gray.windows(2) {
        assert!(
            pair[1] + 2.0 >= pair[0],
            "non-monotonic luminance: {gray:?}"
        );
    }
    assert!(
        gray[3] > gray[0] + 20.0 && gray[5] > gray[3] + 20.0 && gray[8] > gray[5] + 10.0,
        "missing shadow/midtone/highlight separation: {gray:?}"
    );
    assert!(
        gray[8] > 180.0 && (80.0..240.0).contains(&gray[5]),
        "implausible HDR to SDR range: {gray:?}"
    );
    for channel in 0..3 {
        for other in 0..3 {
            if channel != other {
                assert!(
                    means[9 + channel][channel] > means[9 + channel][other] + 30.0,
                    "color channel/hue corruption: {:?}",
                    means[9 + channel]
                );
            }
        }
    }
    for frame in [rgb.len() / frame_size / 2, rgb.len() / frame_size - 1] {
        for (tile, expected) in means.iter().enumerate() {
            let cx = (tile % 4 * 2 + 1) * width / 8;
            let cy = (tile / 4 * 2 + 1) * height / 6;
            let mut actual = [0.0; 3];
            for y in cy - 4..cy + 4 {
                for x in cx - 4..cx + 4 {
                    for channel in 0..3 {
                        actual[channel] +=
                            f64::from(rgb[frame * frame_size + (y * width + x) * 3 + channel])
                                / 64.0;
                    }
                }
            }
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[channel]).abs() < 5.0,
                    "later HDR corruption: frame={frame} tile={tile} expected={expected:?} actual={actual:?}"
                );
            }
        }
    }
    means
}

#[tokio::test]
#[ignore = "physical Intel VAAPI required; calibrated synthetic HDR10 slice, not full HDR visual acceptance"]
async fn physical_vaapi_hdr_tone_mapping_picture_seek_audio_and_adaptive() {
    assert_eq!(std::env::var("OVL_ACCELERATOR").unwrap(), "vaapi");
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
    let raw = root.path().join("signal.yuv");
    write_hdr_signal(&raw);
    let media_dir = root.path().join("media");
    std::fs::create_dir(&media_dir).unwrap();
    let source = media_dir.join("hdr10.mp4");
    let mut args: Vec<String> = [
        "-nostdin",
        "-v",
        "error",
        "-f",
        "rawvideo",
        "-pixel_format",
        "yuv420p10le",
        "-video_size",
        "320x180",
        "-framerate",
        "12",
        "-i",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(raw.to_string_lossy().into_owned());
    args.extend(["-f","lavfi","-i","sine=frequency=440","-f","lavfi","-i","sine=frequency=880","-map","0:v","-map","1:a","-map","2:a","-t","3","-c:v","libx265","-preset","ultrafast","-threads","2","-x265-params","pools=1:frame-threads=1:log-level=error:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400","-pix_fmt","yuv420p10le","-color_primaries","bt2020","-color_trc","smpte2084","-colorspace","bt2020nc","-c:a","aac"].into_iter().map(str::to_owned));
    args.push(source.to_string_lossy().into_owned());
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
    let info = media::inspect_with_hdr_metadata(&app, &source, true)
        .await
        .unwrap();
    assert!(
        ovl_server::hardware::validate_hdr_picture(&app, root.path(), &app.cancel.child_token())
            .await
            .unwrap(),
        "runtime HDR picture probe rejected this driver"
    );
    let options = playback::Playback {
        mode: "transcode".into(),
        start: 1.25,
        audio: 1,
        adaptive: true,
        ..Default::default()
    };
    assert!(
        playback::vaapi_hdr_eligible(&info, &options),
        "fixture metadata: {info:?}"
    );
    let variants = vec![
        playback::Rendition {
            width: 320,
            height: 180,
            kbps: 1000,
        },
        playback::Rendition {
            width: 160,
            height: 90,
            kbps: 500,
        },
    ];
    for hardware in [false, true] {
        let dir = root
            .path()
            .join(if hardware { "vaapi" } else { "software" });
        std::fs::create_dir(&dir).unwrap();
        let args = playback::encoding_args(&source, &options, &info, &variants);
        let args = if hardware {
            playback::vaapi_hdr_args(args, &variants)
        } else {
            args
        };
        if hardware {
            assert!(args.iter().any(|v| v == "h264_vaapi"));
            assert!(args.iter().any(|v| v.contains("tonemap_vaapi")));
        }
        let started = std::time::Instant::now();
        media::process(
            &app,
            "ffmpeg",
            args,
            Some(dir.clone()),
            Duration::from_secs(30),
            1024,
            app.cancel.child_token(),
        )
        .await
        .unwrap();
        let encode_ms = started.elapsed().as_millis();
        for (index, variant) in variants.iter().enumerate() {
            let playlist = dir.join(format!("v{index}.m3u8"));
            let output = media::inspect(&app, &playlist).await.unwrap();
            let video = output.video().unwrap();
            assert_eq!((video.width, video.height), (variant.width, variant.height));
            assert_eq!(video.codec_name, "h264");
            assert_eq!(video.color_transfer, "bt709");
            assert_eq!(video.color_primaries, "bt709");
            assert_eq!(video.color_space, "bt709");
            let args: Vec<String> = [
                "-v",
                "error",
                "-i",
                playlist.to_str().unwrap(),
                "-an",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "pipe:1",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            let rgb = media::process(
                &app,
                "ffmpeg",
                args,
                None,
                Duration::from_secs(15),
                8 << 20,
                app.cancel.child_token(),
            )
            .await
            .unwrap();
            let frame_bytes = variant.width as usize * variant.height as usize * 3;
            println!(
                "hdr_hardware={hardware} variant={index} first/middle/last RGB ranges={:?}",
                [0, rgb.len() / frame_bytes / 2, rgb.len() / frame_bytes - 1].map(|f| {
                    let pixels = &rgb[f * frame_bytes..(f + 1) * frame_bytes];
                    (*pixels.iter().min().unwrap(), *pixels.iter().max().unwrap())
                })
            );
            let means = verify_hdr_picture(&rgb, variant.width as usize, variant.height as usize);
            let args: Vec<String> = [
                "-v",
                "error",
                "-i",
                playlist.to_str().unwrap(),
                "-map",
                "0:a:0",
                "-t",
                "1",
                "-ac",
                "1",
                "-ar",
                "48000",
                "-f",
                "f32le",
                "pipe:1",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            let pcm = media::process(
                &app,
                "ffmpeg",
                args,
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
            let power = |hz: f64| {
                let coefficient = 2.0 * (2.0 * std::f64::consts::PI * hz / 48000.0).cos();
                let (mut previous, mut earlier) = (0.0, 0.0);
                for sample in &samples {
                    let current = f64::from(*sample) + coefficient * previous - earlier;
                    earlier = previous;
                    previous = current;
                }
                previous * previous + earlier * earlier - coefficient * previous * earlier
            };
            assert!(power(880.0) > power(440.0) * 20.0, "wrong selected audio");
            println!(
                "hdr_hardware={hardware} variant={index} encode_ms={encode_ms} decoded_frames={} patches_rgb={means:?}",
                rgb.len() / (variant.width as usize * variant.height as usize * 3)
            );
        }
    }
    // Exercise actual auth -> probe/enrichment -> opt-in selection -> HLS route.
    // The recorded session accelerator proves this was not a CPU fallback pass.
    assert_eq!(std::env::var("OVL_HDR_ACCELERATOR").unwrap(), "vaapi");
    let router = app.router();
    let credentials = serde_json::json!({"Name":"hdr-physical","Password":"test-password-123"});
    hardware_api(
        &router,
        "POST",
        "/api/v1/setup",
        None,
        Some(credentials.clone()),
        201,
    )
    .await;
    let login = hardware_api(
        &router,
        "POST",
        "/api/v1/login",
        None,
        Some(credentials),
        200,
    )
    .await;
    let token = login["token"].as_str().unwrap();
    let library = hardware_api(
        &router,
        "POST",
        "/api/v1/libraries",
        Some(token),
        Some(serde_json::json!({"Name":"HDR","Path":media_dir,"Kind":"movie"})),
        201,
    )
    .await;
    let library_id = library["id"].as_i64().unwrap();
    let source_db = source.to_string_lossy().into_owned();
    app.db
        .call(move |c| {
            c.execute("INSERT INTO items(id,title) VALUES(1,'HDR10')", [])?;
            c.execute(
                "INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(1,?,?,0,0)",
                rusqlite::params![library_id, source_db],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let session = hardware_api(
        &router,
        "POST",
        "/api/v1/items/1/playback",
        Some(token),
        Some(serde_json::json!({"Mode":"auto","Start":1.25,"Audio":1})),
        201,
    )
    .await;
    assert_eq!(session["mode"], "transcode");
    let id = session["id"].as_str().unwrap();
    let stream = app.streams.lock().await.get(id).unwrap().clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !stream.finished.load(std::sync::atomic::Ordering::Acquire) {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let error = stream.error.lock().await.clone();
    assert!(error.is_none(), "{error:?}");
    assert_eq!(stream.accelerator.lock().await.as_deref(), Some("vaapi"));
    let args: Vec<String> = [
        "-v",
        "error",
        "-i",
        stream.dir.join("v0.m3u8").to_str().unwrap(),
        "-an",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "pipe:1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let rgb = media::process(
        &app,
        "ffmpeg",
        args,
        None,
        Duration::from_secs(15),
        8 << 20,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    verify_hdr_picture(&rgb, 320, 180);
    hardware_api(
        &router,
        "DELETE",
        &format!("/api/v1/playback/{id}"),
        Some(token),
        None,
        200,
    )
    .await;
    println!("HDR authenticated route used VAAPI and decoded calibrated seek output");
    app.shutdown().await;
}

async fn hardware_api(
    router: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
    expected: u16,
) -> serde_json::Value {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("X-OVL-Request", "1")
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let reply = router
        .clone()
        .oneshot(
            request
                .body(axum::body::Body::from(
                    body.map(|v| v.to_string()).unwrap_or_default(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = reply.status();
    let bytes = reply.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status.as_u16(),
        expected,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}
