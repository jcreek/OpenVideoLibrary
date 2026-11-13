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
    assert!(
        !ovl_server::hardware::validate_hdr_picture(&app, root.path(), &app.cancel.child_token())
            .await
            .unwrap()
    );
    assert!(!root.path().join("hdr-probe").exists());
    app.shutdown().await;
}
