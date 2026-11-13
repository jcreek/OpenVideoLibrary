//! Explicit negative driver acceptance, separate from successful GPU corpus tests.
#![cfg(feature = "gpu-tests")]
use ovl_server::{App, Config};
use std::{path::PathBuf, time::Duration};
use tempfile::TempDir;
#[tokio::test]
#[ignore = "negative physical test: requires known-broken HDR driver and OVL_EXPECT_HDR_REJECTION=true"]
async fn physical_rejects_black_hdr_driver() {
    assert_eq!(std::env::var("OVL_EXPECT_HDR_REJECTION").unwrap(), "true");
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
    // A missing prerequisite is BLOCKED, not a passing rejection.
    let encoders = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .expect("BLOCKED: ffmpeg is unavailable");
    assert!(
        String::from_utf8_lossy(&encoders.stdout).contains("libx265"),
        "BLOCKED: ffmpeg lacks libx265 for the HDR fixture"
    );
    let detected = ovl_server::hardware::detect(&app).await.unwrap();
    assert!(
        detected.iter().any(|a| a.name == "vaapi" && a.available),
        "BLOCKED: VAAPI encode is unavailable, so the driver was not exercised"
    );
    // Generation and the hardware encode must succeed; only the decoded
    // picture may be rejected.
    let verdict =
        ovl_server::hardware::probe_hdr_picture(&app, root.path(), &app.cancel.child_token())
            .await
            .unwrap_or_else(|e| panic!("HDR probe did not complete: {}", e.1));
    assert!(!verdict, "the known-broken driver produced a valid picture");
    assert!(!root.path().join("hdr-probe").exists());
    app.shutdown().await;
}
