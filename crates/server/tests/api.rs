use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use ovl_server::{
    App, Config, auth, media,
    playback::{self, ClientCapabilities, Playback},
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tempfile::TempDir;
use tower::ServiceExt;
fn config(root: &Path) -> Config {
    Config {
        data: root.join("data"),
        listen: "127.0.0.1:0".into(),
        web: root.join("web"),
        secure_cookies: false,
        max_streams: 2,
        transcode_bytes: 16 << 20,
        min_free_bytes: 1 << 20,
        scan_interval: Duration::from_secs(3600),
        stream_idle: Duration::from_secs(5),
    }
}
async fn request(
    router: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("X-OVL-Request", "1")
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let response = router
        .clone()
        .oneshot(
            req.body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)})),
    )
}
async fn account(router: &Router, name: &str) -> String {
    let (status, v) = request(
        router,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({"Name":name,"Password":"test-password-123"})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    v["token"].as_str().unwrap().to_owned()
}
async fn setup(root: &TempDir) -> (Arc<App>, Router, String) {
    let app = App::open(config(root.path())).unwrap();
    let router = app.router();
    let (status, v) = request(
        &router,
        "POST",
        "/api/v1/setup",
        None,
        Some(json!({"Name":"admin","Password":"test-password-123"})),
    )
    .await;
    assert_eq!(status, 201, "{v}");
    let token = account(&router, "admin").await;
    (app, router, token)
}
async fn add_media(app: &Arc<App>, root: &Path) -> i64 {
    let dir = root.join("media");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Example.S01E01.mp4");
    std::fs::write(&path, b"0123456789").unwrap();
    let dir = std::fs::canonicalize(dir)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let path = std::fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    app.db
        .call(move |c| {
            c.execute(
                "INSERT INTO libraries(name,path,kind) VALUES('TV',?,'tv')",
                [dir],
            )?;
            let lib = c.last_insert_rowid();
            c.execute(
                "INSERT INTO items(title,series,season,episode) VALUES('Example','Example',1,1)",
                [],
            )?;
            let item = c.last_insert_rowid();
            c.execute(
                "INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,?,?,0,10)",
                rusqlite::params![item, lib, path],
            )?;
            Ok(item)
        })
        .await
        .unwrap()
}
#[tokio::test]
async fn authorization_ranges_lists_revocation_and_csrf() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    assert_eq!(
        request(&router, "GET", "/api/v1/items", None, None).await.0,
        401
    );
    let (_, items) = request(
        &router,
        "GET",
        "/api/v1/items?q=Example",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(items.as_array().unwrap().len(), 1);
    let (_, res) = request(
        &router,
        "POST",
        "/api/v1/users",
        Some(&token),
        Some(json!({"Name":"viewer","Password":"test-password-123","Libraries":[1]})),
    )
    .await;
    let viewer_id = res["id"].as_i64().unwrap();
    let viewer = account(&router, "viewer").await;
    assert_eq!(
        request(&router, "GET", "/api/v1/users", Some(&viewer), None)
            .await
            .0,
        403
    );
    let range = Request::builder()
        .uri(format!("/api/v1/items/{id}/file"))
        .header("Authorization", format!("Bearer {viewer}"))
        .header("Range", "bytes=2-5")
        .body(Body::empty())
        .unwrap();
    let res = router.clone().oneshot(range).await.unwrap();
    assert_eq!(res.status(), 206);
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes(), "2345");
    let list = json!({"Kind":"queue","Name":"Tonight","ItemIDs":[id,id],"Cursor":0});
    let (status, created) = request(
        &router,
        "POST",
        "/api/v1/lists",
        Some(&viewer),
        Some(list.clone()),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let list_id = created["id"].as_i64().unwrap();
    let url = format!("/api/v1/lists/{list_id}");
    let (_, saved) = request(&router, "GET", &url, Some(&viewer), None).await;
    assert_eq!(saved["entries"].as_array().unwrap().len(), 2);
    let mut update = list.clone();
    update["Revision"] = json!(1);
    update["Cursor"] = json!(1);
    assert_eq!(
        request(&router, "PUT", &url, Some(&viewer), Some(update.clone()))
            .await
            .0,
        200
    );
    assert_eq!(
        request(&router, "PUT", &url, Some(&viewer), Some(update))
            .await
            .0,
        409
    );
    assert_eq!(
        request(&router, "GET", &url, Some(&token), None).await.0,
        404
    );
    let cookie = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/items/{id}/progress"))
        .header("Cookie", format!("ovl_session={viewer}"))
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"Position":2,"Watched":false}"#))
        .unwrap();
    assert_eq!(router.clone().oneshot(cookie).await.unwrap().status(), 403);
    assert_eq!(
        request(
            &router,
            "PUT",
            &format!("/api/v1/users/{viewer_id}/access"),
            Some(&token),
            Some(json!({"Libraries":[]}))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &router,
            "GET",
            &format!("/api/v1/items/{id}/file"),
            Some(&viewer),
            None
        )
        .await
        .0,
        404
    );
    let (_, saved) = request(&router, "GET", &url, Some(&viewer), None).await;
    assert!(saved["entries"].as_array().unwrap().is_empty());
    let (_, items) = request(&router, "GET", "/api/v1/items", Some(&viewer), None).await;
    assert!(items.as_array().unwrap().is_empty());
    assert_eq!(
        request(
            &router,
            "POST",
            &format!("/api/v1/users/{viewer_id}/password"),
            Some(&token),
            Some(json!({"Password":"replacement-password"}))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(&router, "GET", "/api/v1/me", Some(&viewer), None)
            .await
            .0,
        401
    );
    app.shutdown().await;
}
#[tokio::test]
async fn v1_migration_preserves_accounts_files_progress_and_sessions() {
    let root = TempDir::new().unwrap();
    let cfg = config(root.path());
    std::fs::create_dir_all(&cfg.data).unwrap();
    let c = rusqlite::Connection::open(cfg.data.join("library.db")).unwrap();
    c.execute_batch(include_str!("../src/schema.sql")).unwrap();
    let hash = bcrypt::hash("test-password-123", 4)
        .unwrap()
        .replacen("$2b$", "$2a$", 1);
    c.execute(
        "INSERT INTO users VALUES(7,'legacy',?,1)",
        [hash.as_bytes()],
    )
    .unwrap();
    let library = root.path().join("media");
    std::fs::create_dir_all(&library).unwrap();
    let path = library.join("sample.mp4");
    std::fs::write(&path, b"media").unwrap();
    c.execute(
        "INSERT INTO libraries VALUES(4,'Movies',?,'movie',0)",
        [library.to_string_lossy()],
    )
    .unwrap();
    c.execute(
        "INSERT INTO items VALUES(9,4,?,'Original','',0,0,12,5,'Overview')",
        [path.to_string_lossy()],
    )
    .unwrap();
    c.execute("INSERT INTO progress VALUES(7,9,42,0,1)", [])
        .unwrap();
    c.execute(
        "INSERT INTO sessions VALUES(?,7,?)",
        rusqlite::params![auth::digest("legacy-token"), auth::now() + 3600],
    )
    .unwrap();
    drop(c);
    let app = App::open(cfg.clone()).unwrap();
    assert!(App::open(cfg).is_err());
    let router = app.router();
    let (_, me) = request(&router, "GET", "/api/v1/me", Some("legacy-token"), None).await;
    assert_eq!(me, json!({"ID":7,"Admin":true}));
    let (_, items) = request(&router, "GET", "/api/v1/items", Some("legacy-token"), None).await;
    assert_eq!(items[0]["id"], 9);
    assert_eq!(items[0]["position"], 42.0);
    let (_, scoped) = request(
        &router,
        "GET",
        "/api/v1/items?library=4",
        Some("legacy-token"),
        None,
    )
    .await;
    assert_eq!(items, scoped);
    let _ = account(&router, "legacy").await;
    app.db
        .call(|c| {
            assert_eq!(
                c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
                2
            );
            assert_eq!(
                c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?,
                "ok"
            );
            Ok(())
        })
        .await
        .unwrap();
    app.shutdown().await;
}
#[tokio::test]
async fn scans_preserve_offline_roots_and_manual_metadata() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    std::fs::write(root.path().join("media/Second.S01E02.mkv"), b"second").unwrap();
    std::fs::write(
        root.path().join("media/Second.S01E02.nfo"),
        "<episodedetails><title>NFO title</title><plot>Imported plot</plot></episodedetails>",
    )
    .unwrap();
    ovl_server::scan::start(app.clone()).await.unwrap();
    while app.scans.available_permits() == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (_, items) = request(&router, "GET", "/api/v1/items", Some(&token), None).await;
    assert_eq!(items.as_array().unwrap().len(), 2);
    assert!(
        items
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["title"] == "NFO title")
    );
    assert_eq!(
        request(
            &router,
            "PATCH",
            &format!("/api/v1/items/{id}"),
            Some(&token),
            Some(json!({"Title":"Edited","Overview":"Manual"}))
        )
        .await
        .0,
        200
    );
    let media = root.path().join("media");
    let offline = root.path().join("offline");
    std::fs::rename(media, offline).unwrap();
    ovl_server::scan::start(app.clone()).await.unwrap();
    while app.scans.available_permits() == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (_, items) = request(&router, "GET", "/api/v1/items", Some(&token), None).await;
    assert_eq!(items.as_array().unwrap().len(), 2);
    let (_, libs) = request(&router, "GET", "/api/v1/libraries", Some(&token), None).await;
    assert_eq!(libs[0]["offline"], true);
    app.shutdown().await;
}
#[cfg(unix)]
#[tokio::test]
async fn symlink_escape_is_denied_on_all_media_paths() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    let path = root.path().join("media/Example.S01E01.mp4");
    let outside = root.path().join("outside.mp4");
    std::fs::write(&outside, b"secret").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(outside, path).unwrap();
    for suffix in ["file", "probe", "artwork"] {
        assert_eq!(
            request(
                &router,
                "GET",
                &format!("/api/v1/items/{id}/{suffix}"),
                Some(&token),
                None
            )
            .await
            .0,
            404
        );
    }
    for suffix in ["playback", "subtitles", "match"] {
        assert_eq!(
            request(
                &router,
                "POST",
                &format!("/api/v1/items/{id}/{suffix}"),
                Some(&token),
                Some(json!({}))
            )
            .await
            .0,
            404
        );
    }
    app.shutdown().await;
}
#[test]
fn negotiation_handles_hdr_audio_container_and_subtitles() {
    let mut info:media::Info=serde_json::from_value(json!({"streams":[{"codec_type":"video","codec_name":"hevc","pix_fmt":"yuv420p10le","color_transfer":"smpte2084","width":3840,"height":2160},{"codec_type":"audio","codec_name":"truehd","channels":2}],"format":{"format_name":"matroska","duration":"60"}})).unwrap();
    let mut request = Playback {
        mode: "auto".into(),
        capabilities: Some(ClientCapabilities {
            containers: vec!["matroska".into()],
            video_codecs: vec!["hevc".into()],
            audio_codecs: vec!["truehd".into()],
            max_audio_channels: None,
            max_audio_sample_rate: None,
            hdr: true,
            video_pixel_formats: [("hevc".into(), vec!["yuv420p10le".into()])].into(),
        }),
        ..Default::default()
    };
    assert_eq!(playback::negotiate(&info, &request).0, "direct");
    request.capabilities.as_mut().unwrap().hdr = false;
    assert_eq!(playback::negotiate(&info, &request).0, "transcode");
    let variants = playback::ladder(3840, 2160, true);
    let args = playback::encoding_args(
        Path::new("media.mkv"),
        &Playback {
            mode: "transcode".into(),
            ..Default::default()
        },
        &info,
        &variants,
    );
    assert!(args.iter().any(|a| a.contains("tonemap=tonemap=hable")));
    assert_eq!(variants.len(), 3);
    info.streams[0].codec_name = "h264".into();
    info.streams[0].pix_fmt = "yuv420p".into();
    info.streams[0].color_transfer.clear();
    request.capabilities = None;
    assert_eq!(playback::negotiate(&info, &request).0, "remux");
    request.subtitle = Some(0);
    assert_eq!(playback::negotiate(&info, &request).0, "transcode");
}
#[tokio::test]
async fn process_timeout_and_output_limits_reap_children() {
    let root = TempDir::new().unwrap();
    let app = App::open(config(root.path())).unwrap();
    for overflow in [false, true] {
        let pidfile = root.path().join(if overflow {
            "overflow.pid"
        } else {
            "timeout.pid"
        });
        let script = format!(
            "import os,time;open({:?},'w').write(str(os.getpid()));{}time.sleep(60)",
            pidfile.to_str().unwrap(),
            if overflow {
                "print('x'*4096,flush=True);"
            } else {
                ""
            }
        );
        let result = media::process(
            &app,
            "python3",
            vec!["-c".into(), script],
            None,
            Duration::from_millis(300),
            64,
            app.cancel.child_token(),
        )
        .await;
        assert!(result.is_err());
        let pid = std::fs::read_to_string(pidfile).unwrap();
        assert!(
            !std::process::Command::new("kill")
                .args(["-0", &pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    app.shutdown().await;
}
#[tokio::test]
async fn dropping_request_cancels_and_reaps_process() {
    let root = TempDir::new().unwrap();
    let app = App::open(config(root.path())).unwrap();
    let pidfile = root.path().join("dropped.pid");
    let script = format!(
        "import os,time;open({:?},'w').write(str(os.getpid()));time.sleep(60)",
        pidfile.to_str().unwrap()
    );
    let task_app = app.clone();
    let task = tokio::spawn(async move {
        media::process(
            &task_app,
            "python3",
            vec!["-c".into(), script],
            None,
            Duration::from_secs(60),
            64,
            task_app.cancel.child_token(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !pidfile.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    app.shutdown().await;
    let pid = std::fs::read_to_string(pidfile).unwrap();
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}
#[tokio::test]
async fn invalid_requests_and_setup_reentry_do_not_mutate() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/v1/setup",
            None,
            Some(json!({"Name":"second","Password":"test-password-123"}))
        )
        .await
        .0,
        409
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/v1/lists",
            Some(&token),
            Some(json!({"Kind":"queue","Name":"Bad","ItemIDs":[],"Cursor":1}))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/v1/users",
            Some(&token),
            Some(json!({"Name":"bad","Password":"short"}))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        request(&router, "GET", "/api/v1/status", None, None)
            .await
            .1["setup"],
        false
    );
    app.shutdown().await;
}

#[tokio::test]
async fn live_backup_restore_preserves_lists_and_revokes_sessions() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/v1/lists",
            Some(&token),
            Some(json!({"Kind":"playlist","Name":"Preserved","ItemIDs":[item]}))
        )
        .await
        .0,
        201
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/backup.py");
    let snapshot = root.path().join("snapshot");
    let restored = root.path().join("restored");
    let data = app.config.data.clone();
    let copy_script = script.clone();
    let copy_snapshot = snapshot.clone();
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("python3")
            .arg(copy_script)
            .args(["backup", "--data"])
            .arg(data)
            .arg("--output")
            .arg(copy_snapshot)
            .status()
    })
    .await
    .unwrap()
    .unwrap();
    assert!(status.success());
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("python3")
            .arg(script)
            .args(["restore", "--source"])
            .arg(snapshot)
            .arg("--data")
            .arg(restored)
            .status()
    })
    .await
    .unwrap()
    .unwrap();
    assert!(status.success());
    let mut cfg = config(root.path());
    cfg.data = root.path().join("restored");
    let restored_app = App::open(cfg).unwrap();
    let restored_router = restored_app.router();
    assert_eq!(
        request(&restored_router, "GET", "/api/v1/me", Some(&token), None)
            .await
            .0,
        401
    );
    let new_token = account(&restored_router, "admin").await;
    let (_, lists) = request(
        &restored_router,
        "GET",
        "/api/v1/lists/1",
        Some(&new_token),
        None,
    )
    .await;
    assert_eq!(lists["name"], "Preserved");
    assert_eq!(lists["entries"][0]["item"]["id"], item);
    let library = lists["entries"][0]["item"]["library_id"].as_i64().unwrap();
    let page = format!("/api/v1/items?library={library}");
    let (status, restored_page) =
        request(&restored_router, "GET", &page, Some(&new_token), None).await;
    assert_eq!(status, 200);
    assert_eq!(restored_page[0]["id"], item);
    assert_eq!(
        restored_page,
        request(&router, "GET", &page, Some(&token), None).await.1
    );
    restored_app.shutdown().await;
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "requires FFmpeg with libx264, libass and AAC; run explicitly with media tooling on PATH"]
async fn real_media_remux_adaptive_audio_subtitles_and_cleanup() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    let srt = root.path().join("captions.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:03,000\nCaption example\n").unwrap();
    let path = root.path().join("media/Example.S01E01.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880",
            "-i",
        ])
        .arg(&srt)
        .args([
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:a",
            "-map",
            "3:s",
            "-t",
            "5",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-threads",
            "2",
            "-c:a",
            "aac",
            "-c:s",
            "mov_text",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::copy(&srt, path.with_extension("srt")).unwrap();
    let (_, probe) = request(
        &router,
        "GET",
        &format!("/api/v1/items/{item}/probe"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(probe["streams"].as_array().unwrap().len(), 4);
    for external in [false, true] {
        let (status, sub) = request(
            &router,
            "POST",
            &format!("/api/v1/items/{item}/subtitles"),
            Some(&token),
            Some(json!({"Index":0,"External":external})),
        )
        .await;
        assert_eq!(status, 200, "{sub}");
        assert!(sub["text"].as_str().unwrap().contains("WEBVTT"));
    }
    for (mode, adaptive, subtitle) in [
        ("remux", false, None),
        ("transcode", true, None),
        ("transcode", false, Some(0)),
    ] {
        let (status, session) = request(
            &router,
            "POST",
            &format!("/api/v1/items/{item}/playback"),
            Some(&token),
            Some(
                json!({"Mode":mode,"Audio":1,"Adaptive":adaptive,"Subtitle":subtitle,"Start":0.5}),
            ),
        )
        .await;
        assert_eq!(status, 201, "{session}");
        let id = session["id"].as_str().unwrap();
        let url = session["url"].as_str().unwrap();
        let (status, master) = request(&router, "GET", url, Some(&token), None).await;
        assert_eq!(status, 200, "{master}");
        let master = master["text"].as_str().unwrap();
        assert!(master.contains("v0.m3u8"));
        if adaptive {
            assert!(master.contains("v1.m3u8"));
        }
        let (status, playlist) = request(
            &router,
            "GET",
            &format!("/api/v1/playback/{id}/v0.m3u8"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{playlist}");
        assert!(playlist["text"].as_str().unwrap().contains(".ts"));
        assert_eq!(
            request(
                &router,
                "DELETE",
                &format!("/api/v1/playback/{id}"),
                Some(&token),
                None
            )
            .await
            .0,
            200
        );
        assert!(!app.config.data.join("transcodes").join(id).exists());
    }
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "requires FFmpeg with libx265, zscale and tonemap; this is a software test, not GPU acceptance"]
async fn real_hdr_software_tone_mapping() {
    for transfer in ["smpte2084", "arib-std-b67"] {
        let root = TempDir::new().unwrap();
        let (app, router, token) = setup(&root).await;
        let item = add_media(&app, root.path()).await;
        let path = root.path().join("media/Example.S01E01.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=12",
                "-t",
                "1",
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-x265-params",
                "pools=1:frame-threads=1:log-level=error",
                "-pix_fmt",
                "yuv420p10le",
                "-color_primaries",
                "bt2020",
                "-color_trc",
                transfer,
                "-colorspace",
                "bt2020nc",
            ])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
        let (_, probe) = request(
            &router,
            "GET",
            &format!("/api/v1/items/{item}/probe"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(probe["streams"][0]["color_transfer"], transfer);
        assert_eq!(probe["streams"][0]["color_primaries"], "bt2020");
        assert_eq!(probe["streams"][0]["color_space"], "bt2020nc");
        assert_eq!(probe["streams"][0]["pix_fmt"], "yuv420p10le");
        assert!(probe["streams"][0]["side_data_list"].is_array());
        let (status, session) = request(
            &router,
            "POST",
            &format!("/api/v1/items/{item}/playback"),
            Some(&token),
            Some(json!({"Mode":"auto","CanPlayMP4":true})),
        )
        .await;
        assert_eq!(status, 201, "{session}");
        assert_eq!(session["mode"], "transcode");
        let (status, master) = request(
            &router,
            "GET",
            session["url"].as_str().unwrap(),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{master}");
        let id = session["id"].as_str().unwrap();
        let output = app.config.data.join("transcodes").join(id).join("v0.m3u8");
        let out = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(output)
            .output()
            .unwrap();
        assert!(out.status.success());
        let info: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(info["streams"][0]["color_transfer"], "bt709");
        assert_eq!(
            request(
                &router,
                "DELETE",
                &format!("/api/v1/playback/{id}"),
                Some(&token),
                None
            )
            .await
            .0,
            200
        );
        app.shutdown().await;
    }
}

#[tokio::test]
async fn database_queue_rejects_overload_without_blocking_the_runtime() {
    let root = TempDir::new().unwrap();
    let app = App::open(config(root.path())).unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let first_db = app.db.clone();
    let first = tokio::spawn(async move {
        first_db
            .call(move |_| {
                let _ = started_tx.send(());
                release_rx.recv().unwrap();
                Ok(())
            })
            .await
    });
    started_rx.await.unwrap();
    let mut requests = Vec::new();
    let overload = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..100 {
        let db = app.db.clone();
        let counter = overload.clone();
        requests.push(tokio::spawn(async move {
            let result = db.call(|_| Ok(())).await;
            if let Err(e) = &result {
                assert_eq!(e.0, 503);
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            result
        }));
    }
    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        while overload.load(std::sync::atomic::Ordering::Relaxed) < 36 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    release_tx.send(()).unwrap();
    first.await.unwrap().unwrap();
    for request in requests {
        let _ = request.await.unwrap();
    }
    assert!(observed.is_ok());
    app.shutdown().await;
}
#[tokio::test]
async fn future_schema_is_rejected_without_rewriting_database() {
    let root = TempDir::new().unwrap();
    let cfg = config(root.path());
    std::fs::create_dir_all(&cfg.data).unwrap();
    let c = rusqlite::Connection::open(cfg.data.join("library.db")).unwrap();
    c.execute_batch("PRAGMA user_version=99;CREATE TABLE sentinel(value TEXT);INSERT INTO sentinel VALUES('preserved');").unwrap();
    drop(c);
    let original = std::fs::read(cfg.data.join("library.db")).unwrap();
    assert!(App::open(cfg.clone()).is_err());
    assert_eq!(
        std::fs::read(cfg.data.join("library.db")).unwrap(),
        original
    );
    assert!(!cfg.data.join("library.db-wal").exists());
    let c = rusqlite::Connection::open(cfg.data.join("library.db")).unwrap();
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
    assert_eq!(
        c.query_row("SELECT value FROM sentinel", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
}
#[tokio::test]
async fn login_rate_limit_and_unknown_fields_are_enforced() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/v1/lists",
            Some(&token),
            Some(json!({"Kind":"playlist","Name":"invalid","ItemIDs":[],"Unexpected":true}))
        )
        .await
        .0,
        400
    );
    for _ in 0..19 {
        assert_eq!(
            request(
                &router,
                "POST",
                "/api/v1/login",
                None,
                Some(json!({"Name":"missing","Password":"wrong"}))
            )
            .await
            .0,
            401
        );
    }
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"Name":"missing","Password":"wrong"}))
        )
        .await
        .0,
        429
    );
    app.shutdown().await;
}

#[tokio::test]
async fn queue_cursor_is_atomic_and_preserves_hidden_entries_across_restart() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    let (_, viewer) = request(
        &router,
        "POST",
        "/api/v1/users",
        Some(&token),
        Some(json!({"Name":"queue-viewer","Password":"test-password-123","Libraries":[1]})),
    )
    .await;
    let viewer_id = viewer["id"].as_i64().unwrap();
    let viewer_token = account(&router, "queue-viewer").await;
    let (status, created) = request(
        &router,
        "POST",
        "/api/v1/lists",
        Some(&viewer_token),
        Some(json!({"Kind":"queue","Name":"Persistent","ItemIDs":[id,id,id]})),
    )
    .await;
    assert_eq!(status, 201);
    let url = format!("/api/v1/lists/{}", created["id"]);
    let cursor_url = format!("{url}/cursor");
    let write = json!({"Revision":1,"Cursor":1});
    let (first, second) = tokio::join!(
        request(
            &router,
            "PUT",
            &cursor_url,
            Some(&viewer_token),
            Some(write.clone())
        ),
        request(
            &router,
            "PUT",
            &cursor_url,
            Some(&viewer_token),
            Some(write)
        )
    );
    let mut statuses = [first.0.as_u16(), second.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let file = root.path().join("media/Example.S01E01.mp4");
    std::fs::rename(&file, file.with_extension("offline")).unwrap();
    let (_, hidden) = request(&router, "GET", &url, Some(&viewer_token), None).await;
    assert_eq!(hidden["entry_count"], 3);
    assert_eq!(hidden["cursor"], 1);
    assert!(hidden["entries"].as_array().unwrap().is_empty());
    assert_eq!(
        request(
            &router,
            "PUT",
            &cursor_url,
            Some(&viewer_token),
            Some(json!({"Revision":2,"Cursor":2}))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &router,
            "PUT",
            &cursor_url,
            Some(&token),
            Some(json!({"Revision":3,"Cursor":0}))
        )
        .await
        .0,
        404
    );
    assert_eq!(
        request(
            &router,
            "PUT",
            &cursor_url,
            Some(&viewer_token),
            Some(json!({"Revision":3,"Cursor":4}))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        request(
            &router,
            "PUT",
            &cursor_url,
            Some(&viewer_token),
            Some(json!({"Cursor":0}))
        )
        .await
        .0,
        400
    );
    request(
        &router,
        "PUT",
        &format!("/api/v1/users/{viewer_id}/access"),
        Some(&token),
        Some(json!({"Libraries":[]})),
    )
    .await;
    // Ownership authorizes a cursor change, but no hidden media information leaks.
    assert_eq!(
        request(
            &router,
            "PUT",
            &cursor_url,
            Some(&viewer_token),
            Some(json!({"Revision":3,"Cursor":0}))
        )
        .await
        .0,
        200
    );
    app.shutdown().await;
    drop(router);
    drop(app);
    let app = App::open(config(root.path())).unwrap();
    let router = app.router();
    let (_, hidden) = request(&router, "GET", &url, Some(&viewer_token), None).await;
    assert_eq!(hidden["revision"], 4);
    assert_eq!(hidden["cursor"], 0);
    assert_eq!(hidden["entry_count"], 3);
    assert!(hidden["entries"].as_array().unwrap().is_empty());
    std::fs::rename(file.with_extension("offline"), &file).unwrap();
    assert_eq!(
        request(
            &router,
            "PUT",
            &format!("/api/v1/users/{viewer_id}/access"),
            Some(&token),
            Some(json!({"Libraries":[1]}))
        )
        .await
        .0,
        200
    );
    let (_, saved) = request(&router, "GET", &url, Some(&viewer_token), None).await;
    assert_eq!(saved["entries"].as_array().unwrap().len(), 3);
    assert_eq!(saved["entries"][2]["position"], 2);
    app.shutdown().await;
}

#[tokio::test]
async fn database_shutdown_drains_admitted_work_and_rejects_clones() {
    let root = TempDir::new().unwrap();
    let app = App::open(config(root.path())).unwrap();
    let clone = app.db.clone();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let pending = tokio::spawn({
        let db = clone.clone();
        async move {
            db.call(move |c| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                c.execute(
                    "INSERT INTO users(name,password,admin) VALUES('drained',x'00',0)",
                    [],
                )?;
                Ok(())
            })
            .await
        }
    });
    entered_rx.await.unwrap();
    let stopping = tokio::spawn({
        let app = app.clone();
        async move { app.shutdown().await }
    });
    tokio::task::yield_now().await;
    assert!(!stopping.is_finished());
    release_tx.send(()).unwrap();
    pending.await.unwrap().unwrap();
    stopping.await.unwrap();
    assert_eq!(clone.call(|_| Ok(())).await.unwrap_err().0, 503);
    // Idempotent shutdown, even with retained Db clones.
    app.shutdown().await;
    drop(app);
    let reopened = App::open(config(root.path())).unwrap();
    assert_eq!(
        reopened
            .db
            .call(|c| Ok(c.query_row(
                "SELECT count(*) FROM users WHERE name='drained'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        1
    );
    reopened.shutdown().await;
}

#[test]
fn demuxer_aliases_do_not_claim_webm_compatibility_for_matroska() {
    let info:media::Info=serde_json::from_value(json!({"streams":[{"codec_type":"video","codec_name":"vp9","pix_fmt":"yuv420p"},{"codec_type":"audio","codec_name":"opus","channels":2}],"format":{"format_name":"matroska,webm"}})).unwrap();
    let request = Playback {
        capabilities: Some(ClientCapabilities {
            containers: vec!["webm".into()],
            video_codecs: vec!["vp9".into()],
            audio_codecs: vec!["opus".into()],
            max_audio_channels: None,
            max_audio_sample_rate: None,
            hdr: false,
            video_pixel_formats: Default::default(),
        }),
        ..Default::default()
    };
    assert_eq!(playback::negotiate(&info, &request).0, "transcode");
    let mut webm = info;
    webm.format.format_name = "webm".into();
    assert_eq!(playback::negotiate(&webm, &request).0, "direct");
}

#[tokio::test]
async fn browse_pagination_preserves_order_search_and_current_grants() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    app.db.call(|c|{
        let tx=c.transaction()?;
        tx.execute("INSERT INTO libraries(id,name,path,kind) VALUES(1,'Synthetic','/synthetic','movie')",[])?;
        for id in 1..=3000 {
            tx.execute("INSERT INTO items(id,title) VALUES(?,?)",rusqlite::params![id,format!("Movie {:04}",1001-id)])?;
            tx.execute("INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,1,?,0,0)",rusqlite::params![id,format!("/synthetic/{id}.mp4")])?;
        }
        tx.commit()?;Ok(())
    }).await.unwrap();
    for path in [
        "/api/v1/items?offset=2900",
        "/api/v1/items?offset=2900&q=Movie",
        "/api/v1/items?offset=2900&library=1",
    ] {
        let (status, items) = request(&router, "GET", path, Some(&token), None).await;
        assert_eq!(status, 200, "{items}");
        let ids: Vec<i64> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, (1..=100).rev().collect::<Vec<_>>());
    }
    assert!(
        request(
            &router,
            "GET",
            "/api/v1/items?q=NoMatches",
            Some(&token),
            None
        )
        .await
        .1
        .as_array()
        .unwrap()
        .is_empty()
    );
    let (status, _) = request(
        &router,
        "POST",
        "/api/v1/users",
        Some(&token),
        Some(json!({"Name":"no-grants","Password":"test-password-123"})),
    )
    .await;
    assert_eq!(status, 201);
    let viewer = account(&router, "no-grants").await;
    assert!(
        request(
            &router,
            "GET",
            "/api/v1/items?offset=2900",
            Some(&viewer),
            None
        )
        .await
        .1
        .as_array()
        .unwrap()
        .is_empty()
    );
    let page = "/api/v1/items?library=1&offset=2900";
    assert_eq!(
        request(&router, "GET", page, Some(&viewer), None).await.1,
        json!([])
    );
    app.db.call(|c| {
        c.execute("INSERT INTO access(user_id,library_id) SELECT id,1 FROM users WHERE name='no-grants'", [])?;
        c.execute("INSERT INTO progress(user_id,item_id,position,watched,updated) SELECT id,100,12,1,0 FROM users WHERE name='no-grants'", [])?;
        Ok(())
    }).await.unwrap();
    let (_, single) = request(&router, "GET", page, Some(&viewer), None).await;
    assert_eq!(single[0]["position"].as_f64(), Some(12.0));
    assert_eq!(single[0]["watched"], true);
    app.db.call(|c| {
        c.execute("INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(101,1,'/synthetic/alternate.mp4',0,0)", [])?;
        Ok(())
    }).await.unwrap();
    let (_, single) = request(&router, "GET", page, Some(&viewer), None).await;
    assert_eq!(single[0]["id"], 101);
    assert_eq!(single[1]["id"], 100);

    app.db
        .call(|c| {
            c.execute(
                "INSERT INTO libraries(id,name,path,kind) VALUES(2,'Sparse','/sparse','movie')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (_, multiple) = request(&router, "GET", page, Some(&viewer), None).await;
    assert_eq!(single, multiple);
    let (_, original) = request(
        &router,
        "GET",
        "/api/v1/items?library=1.0&offset=2900",
        Some(&viewer),
        None,
    )
    .await;
    assert_eq!(single, original);
    app.db.call(|c| {
        c.execute("UPDATE media_files SET library_id=2,item_id=100 WHERE path='/synthetic/alternate.mp4'", [])?;
        c.execute("UPDATE items SET title='Renamed' WHERE id=100", [])?;
        Ok(())
    }).await.unwrap();
    let (status, moved) = request(
        &router,
        "GET",
        "/api/v1/items?library=2",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(moved[0]["title"], "Renamed");
    assert_eq!(moved[0]["id"], 100);
    assert_eq!(
        request(
            &router,
            "GET",
            "/api/v1/items?library=2",
            Some(&viewer),
            None
        )
        .await
        .1,
        json!([])
    );
    app.db
        .call(|c| {
            c.execute("DELETE FROM items WHERE id=100", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        request(
            &router,
            "GET",
            "/api/v1/items?library=2",
            Some(&token),
            None
        )
        .await
        .1,
        json!([])
    );
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "generated software corpus requires FFmpeg with VP9, Opus, H.264, MPEG4 and MP2"]
async fn real_media_container_audio_matrix_decodes_negotiated_output() {
    for (extension, muxer, video, pixel_format, audio, expected_container, expected_mode) in [
        ("mp4", "mp4", "libx264", "yuv420p", "aac", "mp4", "direct"),
        (
            "mp4",
            "mp4",
            "libx264",
            "yuv420p10le",
            "aac",
            "mp4",
            "transcode",
        ),
        (
            "mp4",
            "mp4",
            "libx264",
            "yuv444p",
            "aac",
            "mp4",
            "transcode",
        ),
        (
            "mkv", "matroska", "libx264", "yuv420p", "aac", "matroska", "remux",
        ),
        (
            "webm",
            "webm",
            "libvpx-vp9",
            "yuv420p",
            "libopus",
            "webm",
            "direct",
        ),
        (
            "ts", "mpegts", "libx264", "yuv420p", "mp2", "mpegts", "remux",
        ),
        (
            "avi",
            "avi",
            "mpeg4",
            "yuv420p",
            "libmp3lame",
            "avi",
            "transcode",
        ),
        (
            "mov",
            "mov",
            "libx264",
            "yuv420p",
            "pcm_s16le",
            "mov",
            "remux",
        ),
    ] {
        let root = TempDir::new().unwrap();
        let (app, router, token) = setup(&root).await;
        let item = add_media(&app, root.path()).await;
        let path = root
            .path()
            .join("media")
            .join(format!("corpus.{extension}"));
        let args: Vec<String> = vec![
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=12",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "1",
            "-c:v",
            video,
            "-threads",
            "2",
            "-pix_fmt",
            pixel_format,
            "-c:a",
            audio,
            "-ac",
            "2",
            "-f",
            muxer,
            path.to_str().unwrap(),
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
        let saved_path = path.to_string_lossy().into_owned();
        app.db
            .call(move |c| {
                c.execute(
                    "UPDATE media_files SET path=? WHERE item_id=?",
                    rusqlite::params![saved_path, item],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let info = media::inspect(&app, &path).await.unwrap();
        assert_eq!(info.format.format_name, expected_container, "{extension}");
        assert_eq!(info.video().unwrap().pix_fmt, pixel_format, "{extension}");
        let (status,session)=request(&router,"POST",&format!("/api/v1/items/{item}/playback"),Some(&token),Some(json!({"Mode":"auto","Capabilities":{"containers":["mp4","webm"],"video_codecs":["h264","vp9"],"video_pixel_formats":{"h264":["yuv420p","yuvj420p"],"vp9":["yuv420p"]},"audio_codecs":["aac","opus","mp3"],"hdr":false}}))).await;
        assert!(status == 200 || status == 201, "{extension}: {session}");
        assert_eq!(session["mode"], expected_mode, "{extension}");
        let output = if expected_mode == "direct" {
            path.clone()
        } else {
            let (status, master) = request(
                &router,
                "GET",
                session["url"].as_str().unwrap(),
                Some(&token),
                None,
            )
            .await;
            assert_eq!(status, 200, "{extension}: {master}");
            app.config
                .data
                .join("transcodes")
                .join(session["id"].as_str().unwrap())
                .join("v0.m3u8")
        };
        if expected_mode == "transcode" {
            let converted = media::inspect(&app, &output).await.unwrap();
            let track = converted.video().unwrap();
            assert_eq!(track.codec_name, "h264", "{extension}/{pixel_format}");
            assert_eq!(track.pix_fmt, "yuv420p", "{extension}/{pixel_format}");
        }
        // Compare decoded RGB, not merely encoder exit status or a nonblack frame.
        let mut pictures = Vec::new();
        for picture_path in [&path, &output] {
            let args = [
                "-v",
                "error",
                "-i",
                picture_path.to_str().unwrap(),
                "-map",
                "0:v:0",
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            pictures.push(
                media::process(
                    &app,
                    "ffmpeg",
                    args,
                    None,
                    Duration::from_secs(15),
                    1 << 20,
                    app.cancel.child_token(),
                )
                .await
                .unwrap(),
            );
        }
        assert_eq!(pictures[0].len(), 320 * 180 * 3);
        assert_eq!(pictures[1].len(), pictures[0].len());
        let difference = pictures[0]
            .iter()
            .zip(&pictures[1])
            .map(|(a, b)| f64::from(a.abs_diff(*b)))
            .sum::<f64>()
            / pictures[0].len() as f64;
        assert!(
            difference < 12.0,
            "{extension}/{pixel_format}: RGB mean absolute error {difference}"
        );
        let args: Vec<String> = vec![
            "-v",
            "error",
            "-i",
            output.to_str().unwrap(),
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let pixels = media::process(
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
        assert_eq!(pixels.len(), 320 * 180, "{extension}");
        assert!(
            pixels.iter().any(|v| *v > 100) && pixels.iter().any(|v| *v < 40),
            "{extension}: decoded image lacks expected test pattern"
        );
        let args: Vec<String> = vec![
            "-v",
            "error",
            "-i",
            output.to_str().unwrap(),
            "-map",
            "0:a:0",
            "-t",
            "0.5",
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
        assert!(pcm.len() > 80000, "{extension}: missing decoded audio");
        let rms = (pcm
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f64::from(f32::from_le_bytes(*v)).powi(2))
            .sum::<f64>()
            / (pcm.len() / 4) as f64)
            .sqrt();
        assert!(
            rms > 0.01 && rms < 0.2,
            "{extension}: unexpected decoded audio RMS {rms}"
        );
        app.shutdown().await;
    }
}

#[tokio::test]
async fn stopping_an_owned_unavailable_stream_still_cleans_up() {
    use std::sync::atomic::{AtomicBool, AtomicI64};
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    let stream_id = "a".repeat(64);
    let dir = app.config.data.join("transcodes").join(&stream_id);
    std::fs::create_dir_all(&dir).unwrap();
    let (done_tx, done) = tokio::sync::watch::channel(false);
    let cancel = app.cancel.child_token();
    let stream = Arc::new(playback::Stream {
        owner: 1,
        library: 1,
        item: id,
        dir: dir.clone(),
        cancel: cancel.clone(),
        done,
        touched: AtomicI64::new(auth::now()),
        finished: AtomicBool::new(false),
        error: tokio::sync::Mutex::new(None),
        accelerator: tokio::sync::Mutex::new(None),
    });
    app.streams.lock().await.insert(stream_id.clone(), stream);
    app.tasks.spawn({
        let app = app.clone();
        let stream_id = stream_id.clone();
        let dir = dir.clone();
        async move {
            cancel.cancelled().await;
            tokio::fs::remove_dir_all(dir).await.unwrap();
            app.streams.lock().await.remove(&stream_id);
            done_tx.send(true).unwrap();
        }
    });
    std::fs::remove_file(root.path().join("media/Example.S01E01.mp4")).unwrap();
    assert_eq!(
        request(
            &router,
            "DELETE",
            &format!("/api/v1/playback/{stream_id}"),
            Some(&token),
            None
        )
        .await
        .0,
        404
    );
    assert!(!dir.exists());
    assert!(app.streams.lock().await.is_empty());
    app.shutdown().await;
}

#[tokio::test]
async fn active_direct_response_body_ends_when_server_stops() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/items/{id}/file"))
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    // Hold the response body without consuming it, as a stalled network peer can.
    app.shutdown().await;
    let bytes = tokio::time::timeout(Duration::from_secs(1), response.into_body().collect())
        .await
        .unwrap()
        .unwrap()
        .to_bytes();
    assert!(bytes.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn owned_storage_symlinks_never_redirect_startup_cleanup_or_database_writes() {
    for (name, directory) in [
        ("transcodes", true),
        ("artwork", true),
        ("library.db", false),
        ("library.db-wal", false),
        ("library.db-shm", false),
        ("server.lock", false),
    ] {
        let root = TempDir::new().unwrap();
        let cfg = config(root.path());
        std::fs::create_dir_all(&cfg.data).unwrap();
        let outside = root.path().join("outside");
        if directory {
            let media = outside.join("a".repeat(64));
            std::fs::create_dir_all(&media).unwrap();
            std::fs::write(media.join("keep.mp4"), b"preserved").unwrap();
        } else {
            std::fs::write(&outside, b"preserved").unwrap();
        }
        std::os::unix::fs::symlink(&outside, cfg.data.join(name)).unwrap();
        assert!(App::open(cfg).is_err(), "{name}");
        let preserved = if directory {
            outside.join("a".repeat(64)).join("keep.mp4")
        } else {
            outside
        };
        assert_eq!(std::fs::read(preserved).unwrap(), b"preserved", "{name}");
    }
}

#[tokio::test]
async fn media_library_cannot_overlap_application_storage() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let nested = app.config.data.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    for path in [root.path(), &app.config.data, &nested] {
        assert_eq!(
            request(
                &router,
                "POST",
                "/api/v1/libraries",
                Some(&token),
                Some(json!({"Name":"unsafe","Kind":"movie","Path":path}))
            )
            .await
            .0,
            400
        );
    }
    assert!(
        request(&router, "GET", "/api/v1/libraries", Some(&token), None)
            .await
            .1
            .as_array()
            .unwrap()
            .is_empty()
    );
    app.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn cached_artwork_cannot_escape_owned_cache() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    let artwork = app.config.data.join("artwork");
    std::fs::create_dir(&artwork).unwrap();
    let outside = root.path().join("private.jpg");
    std::fs::write(&outside, b"not public artwork").unwrap();
    std::os::unix::fs::symlink(&outside, artwork.join(format!("{id}.jpg"))).unwrap();
    assert_eq!(
        request(
            &router,
            "GET",
            &format!("/api/v1/items/{id}/artwork"),
            Some(&token),
            None
        )
        .await
        .0,
        404
    );
    app.shutdown().await;
}

#[tokio::test]
async fn broad_search_cache_keeps_metadata_and_permissions_current_and_falls_back_on_quota() {
    for constrained in [false, true] {
        let root = TempDir::new().unwrap();
        let (app, router, token) = setup(&root).await;
        app.db.call(move|c|{
            let tx=c.transaction()?;
            tx.execute("INSERT INTO libraries(id,name,path,kind) VALUES(1,'Synthetic','/synthetic','movie')",[])?;
            for id in 1..=5000 {
                tx.execute("INSERT INTO items(id,title) VALUES(?,?)",rusqlite::params![id,format!("Movie {id:05}")])?;
                tx.execute("INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,1,?,0,0)",rusqlite::params![id,format!("/synthetic/{id}.mp4")])?;
            }
            tx.commit()?;
            if constrained {c.execute_batch("PRAGMA ovl_query_cache.max_page_count=4")?;}
            Ok(())
        }).await.unwrap();
        let url = "/api/v1/items?q=Movie&offset=4900";
        for _ in 0..3 {
            let (status, items) = request(&router, "GET", url, Some(&token), None).await;
            assert_eq!(status, 200, "constrained={constrained}: {items}");
            let ids: Vec<i64> = items
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].as_i64().unwrap())
                .collect();
            assert_eq!(
                ids,
                (4901..=5000).collect::<Vec<_>>(),
                "cache must not truncate matches"
            );
        }
        if !constrained {
            app.db
                .call(|c| {
                    assert_eq!(
                        c.query_row("SELECT count(*) FROM ovl_query_cache.entries", [], |r| r
                            .get::<_, i64>(0))?,
                        5000
                    );
                    c.execute("INSERT INTO progress VALUES(1,3,42,0,0)", [])?;
                    Ok(())
                })
                .await
                .unwrap();
            let (_, progress) =
                request(&router, "GET", "/api/v1/items?q=Movie", Some(&token), None).await;
            assert_eq!(
                progress
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["id"] == 3)
                    .unwrap()["position"],
                42.0
            );
            app.db
                .call(|c| {
                    c.execute(
                        "UPDATE items SET title='Removed from search' WHERE id=1",
                        [],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
            let (status, items) =
                request(&router, "GET", "/api/v1/items?q=Movie", Some(&token), None).await;
            assert_eq!(status, 200);
            assert_eq!(items[0]["id"], 2);
            // Repopulate after the internal mutation so the external-write
            // assertion exercises invalidation of actual cached membership.
            assert_eq!(
                request(&router, "GET", "/api/v1/items?q=Movie", Some(&token), None)
                    .await
                    .0,
                200
            );
            app.db
                .call(|c| {
                    assert_eq!(
                        c.query_row("SELECT count(*) FROM ovl_query_cache.entries", [], |r| r
                            .get::<_, i64>(0))?,
                        4999
                    );
                    Ok(())
                })
                .await
                .unwrap();
            // External writes occur only on this disposable fixture. They must
            // also invalidate the cache, independently of our TEMP triggers.
            let c = rusqlite::Connection::open(app.config.data.join("library.db")).unwrap();
            c.execute("UPDATE items SET title='Removed externally' WHERE id=2", [])
                .unwrap();
            drop(c);
            let (status, items) =
                request(&router, "GET", "/api/v1/items?q=Movie", Some(&token), None).await;
            assert_eq!(status, 200);
            assert_eq!(items[0]["id"], 3);
            for term in [
                "movie", "MOVIE", "mOvie", "moVie", "movIe", "moviE", "MOvie", "MovIe", "MOvIE",
            ] {
                for _ in 0..2 {
                    assert_eq!(
                        request(
                            &router,
                            "GET",
                            &format!("/api/v1/items?q={term}"),
                            Some(&token),
                            None
                        )
                        .await
                        .0,
                        200
                    );
                }
            }
            app.db
                .call(|c| {
                    assert!(
                        c.query_row("SELECT count(*) FROM ovl_query_cache.queries", [], |r| r
                            .get::<_, i64>(0))?
                            <= 8
                    );
                    assert!(
                        c.query_row("SELECT count(*) FROM ovl_query_cache.entries", [], |r| r
                            .get::<_, i64>(0))?
                            <= 200000
                    );
                    Ok(())
                })
                .await
                .unwrap();
        }
        let (status, created) = request(
            &router,
            "POST",
            "/api/v1/users",
            Some(&token),
            Some(json!({"Name":"cache-viewer","Password":"test-password-123","Libraries":[1]})),
        )
        .await;
        assert_eq!(status, 201);
        let viewer = account(&router, "cache-viewer").await;
        assert!(
            !request(&router, "GET", url, Some(&viewer), None)
                .await
                .1
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            request(
                &router,
                "PUT",
                &format!("/api/v1/users/{}/access", created["id"]),
                Some(&token),
                Some(json!({"Libraries":[]}))
            )
            .await
            .0,
            200
        );
        assert!(
            request(&router, "GET", url, Some(&viewer), None)
                .await
                .1
                .as_array()
                .unwrap()
                .is_empty()
        );
        app.shutdown().await;
    }
}

#[tokio::test]
async fn broad_search_above_cache_cap_preserves_tail_results() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    app.db.call(|c| {
        let tx = c.transaction()?;
        tx.execute("INSERT INTO libraries(id,name,path,kind) VALUES(1,'Synthetic','/synthetic','movie')", [])?;
        tx.execute_batch("WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM ids WHERE id<100005) INSERT INTO items(id,title) SELECT id,printf('Movie %08d',id) FROM ids;
            INSERT INTO media_files(item_id,library_id,path,mtime,size) SELECT id,1,printf('/synthetic/%d.mp4',id),0,0 FROM items;")?;
        tx.commit()?;
        Ok(())
    }).await.unwrap();
    for _ in 0..3 {
        let (status, items) = request(
            &router,
            "GET",
            "/api/v1/items?q=Movie&offset=100000",
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{items}");
        let ids: Vec<_> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, (100001..=100005).collect::<Vec<_>>());
    }
    app.db
        .call(|c| {
            assert_eq!(
                c.query_row("SELECT count(*) FROM ovl_query_cache.entries", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            assert_eq!(
                c.query_row("SELECT status FROM ovl_query_cache.queries", [], |r| r
                    .get::<_, i64>(0))?,
                2
            );
            Ok(())
        })
        .await
        .unwrap();
    app.shutdown().await;
}

#[test]
fn hdr_hardware_selection_requires_verified_color_metadata_and_opt_in() {
    let mut info: media::Info = serde_json::from_value(json!({"streams":[{
        "codec_type":"video","codec_name":"hevc","color_transfer":"smpte2084",
        "color_primaries":"bt2020","color_space":"bt2020nc","color_range":"tv","pix_fmt":"yuv420p10le",
        "side_data_list":[{"side_data_type":"Mastering display metadata"},{"side_data_type":"Content light level metadata"}]
    }]})).unwrap();
    let b = Playback {
        mode: "transcode".into(),
        ..Default::default()
    };
    assert!(
        !playback::vaapi_hdr_eligible(&info, &b),
        "unchecked frame metadata must not admit hardware"
    );
    info.streams[0].hdr_frame_metadata_checked = true;
    assert!(playback::vaapi_hdr_eligible(&info, &b));
    for changes in [
        json!({"color_transfer":"arib-std-b67"}),
        json!({"color_primaries":""}),
        json!({"color_space":"bt709"}),
        json!({"color_range":"pc"}),
        json!({"color_range":""}),
        json!({"pix_fmt":"yuv420p"}),
        json!({"codec_name":"av1"}),
        json!({"codec_tag_string":"dvh1"}),
        json!({"side_data_list":[]}),
        json!({"side_data_list":[{"side_data_type":"DOVI configuration record"},{"side_data_type":"Mastering display metadata"},{"side_data_type":"Content light level metadata"}]}),
    ] {
        let mut changed = serde_json::to_value(&info).unwrap();
        for (key, value) in changes.as_object().unwrap() {
            changed["streams"][0][key] = value.clone();
        }
        let mut changed: media::Info = serde_json::from_value(changed).unwrap();
        changed.streams[0].hdr_frame_metadata_checked = true;
        assert!(!playback::vaapi_hdr_eligible(&changed, &b));
    }
    assert!(!playback::vaapi_hdr_eligible(
        &info,
        &Playback {
            subtitle: Some(0),
            ..b.clone()
        }
    ));
    assert!(!playback::vaapi_hdr_eligible(
        &info,
        &Playback {
            mode: "remux".into(),
            ..b
        }
    ));
    for accelerator in ["auto", "vaapi"] {
        assert!(ovl_server::hardware::wants_vaapi_hdr(accelerator, "vaapi"));
    }
    for accelerator in ["cpu", "qsv", "nvenc", ""] {
        assert!(!ovl_server::hardware::wants_vaapi_hdr(accelerator, "vaapi"));
    }
    for hdr in ["", "auto", "cpu", "qsv", "nvenc"] {
        assert!(!ovl_server::hardware::wants_vaapi_hdr("auto", hdr));
    }
}

#[test]
fn negotiation_requires_source_pixel_format_support() {
    let mut info:media::Info=serde_json::from_value(json!({"streams":[{"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p"},{"codec_type":"audio","codec_name":"aac","channels":2}],"format":{"format_name":"mp4"}})).unwrap();
    let mut request:Playback=serde_json::from_value(json!({"Mode":"auto","CanPlayMP4":true,"Capabilities":{"containers":["mp4"],"video_codecs":["h264"],"audio_codecs":["aac"],"hdr":false}})).unwrap();
    assert_eq!(playback::negotiate(&info, &request).0, "direct");
    for format in ["yuv420p10le", "yuv444p", "yuv444p10le", ""] {
        info.streams[0].pix_fmt = format.into();
        assert_eq!(
            playback::negotiate(&info, &request).0,
            "transcode",
            "{format}"
        );
        let mut legacy = request.clone();
        legacy.capabilities = None;
        assert_eq!(
            playback::negotiate(&info, &legacy).0,
            "transcode",
            "legacy {format}"
        );
        legacy.can_play_mp4 = false;
        assert_eq!(
            playback::negotiate(&info, &legacy).0,
            "transcode",
            "legacy remux {format}"
        );
    }
    info.streams[0].pix_fmt = "yuv420p10le".into();
    request
        .capabilities
        .as_mut()
        .unwrap()
        .video_pixel_formats
        .insert("h264".into(), vec!["yuv420p10le".into()]);
    assert_eq!(playback::negotiate(&info, &request).0, "direct");
    info.format.format_name = "matroska".into();
    assert_eq!(playback::negotiate(&info, &request).0, "remux");
    request
        .capabilities
        .as_mut()
        .unwrap()
        .video_pixel_formats
        .insert("h264".into(), vec![]);
    assert_eq!(playback::negotiate(&info, &request).0, "transcode");
    info.streams[0].pix_fmt.clear();
    request
        .capabilities
        .as_mut()
        .unwrap()
        .video_pixel_formats
        .insert("h264".into(), vec![String::new()]);
    assert_eq!(playback::negotiate(&info, &request).0, "transcode");
}

// Original synthetic PGS display sets: a white rectangle from 1s through 3s.
// No media or subtitle artwork is copied from an external source.
fn bitmap_fixture(x: u16, y: u16) -> Vec<u8> {
    fn segment(out: &mut Vec<u8>, second: u32, kind: u8, payload: &[u8]) {
        out.extend(b"PG");
        out.extend((second * 90_000).to_be_bytes());
        out.extend(0_u32.to_be_bytes());
        out.push(kind);
        out.extend(u16::try_from(payload.len()).unwrap().to_be_bytes());
        out.extend(payload);
    }
    fn composition(number: u16, state: u8, position: Option<(u16, u16)>) -> Vec<u8> {
        let mut bytes = vec![];
        bytes.extend(1280_u16.to_be_bytes());
        bytes.extend(720_u16.to_be_bytes());
        bytes.push(0x10);
        bytes.extend(number.to_be_bytes());
        bytes.extend([state, 0, 0, u8::from(position.is_some())]);
        if let Some((x, y)) = position {
            bytes.extend([0, 0, 0, 0]);
            bytes.extend(x.to_be_bytes());
            bytes.extend(y.to_be_bytes());
        }
        bytes
    }
    let mut out = vec![];
    segment(&mut out, 1, 0x16, &composition(0, 0x80, Some((x, y))));
    let mut window = vec![1, 0];
    for value in [x, y, 160, 40] {
        window.extend(value.to_be_bytes());
    }
    segment(&mut out, 1, 0x17, &window);
    segment(
        &mut out,
        1,
        0x14,
        &[
            0, 0, 0, 16, 128, 128, 0, 1, 235, 128, 128, 255, 2, 235, 128, 128, 128,
        ],
    );
    let mut rle = vec![];
    for _ in 0..40 {
        rle.extend([1; 80]);
        rle.extend([2; 80]);
        rle.extend([0, 0]);
    }
    let mut object = vec![0, 0, 0, 0xc0];
    let length = u32::try_from(rle.len() + 4).unwrap().to_be_bytes();
    object.extend(&length[1..]);
    object.extend(160_u16.to_be_bytes());
    object.extend(40_u16.to_be_bytes());
    object.extend(rle);
    segment(&mut out, 1, 0x15, &object);
    segment(&mut out, 1, 0x80, &[]);
    segment(&mut out, 3, 0x16, &composition(1, 0, None));
    segment(&mut out, 3, 0x80, &[]);
    out
}

#[tokio::test]
#[ignore = "generated bitmap matrix requires PGS/DVD/DVB/XSUB, overlay, libx264/libx265 and tone mapping; software acceptance"]
async fn real_bitmap_subtitles_select_seek_clear_and_scale() {
    for hdr in [false, true] {
        let root = TempDir::new().unwrap();
        let (app, router, token) = setup(&root).await;
        let item = add_media(&app, root.path()).await;
        let first = root.path().join("first.sup");
        let second = root.path().join("second.sup");
        std::fs::write(&first, bitmap_fixture(200, 560)).unwrap();
        std::fs::write(&second, bitmap_fixture(800, 100)).unwrap();
        let path = root.path().join("media/bitmap.mkv");
        let mut args: Vec<String> = [
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=1280x720:rate=12",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-itsoffset",
            "1",
            "-i",
            first.to_str().unwrap(),
            "-itsoffset",
            "1",
            "-i",
            second.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:s",
            "-map",
            "3:s",
            "-t",
            "5",
            "-c:v",
            if hdr { "libx265" } else { "libx264" },
            "-preset",
            "ultrafast",
            "-threads",
            "2",
            "-g",
            "12",
            "-c:a",
            "aac",
            "-c:s",
            "copy",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if hdr {
            args.extend(
                [
                    "-pix_fmt",
                    "yuv420p10le",
                    "-x265-params",
                    "pools=1:frame-threads=1:log-level=error",
                    "-color_trc",
                    "smpte2084",
                    "-color_primaries",
                    "bt2020",
                    "-colorspace",
                    "bt2020nc",
                ]
                .into_iter()
                .map(str::to_owned),
            );
        }
        args.push(path.to_string_lossy().into_owned());
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
        let source_path = path;
        for (codec, extension, expected_codec) in [
            ("copy", "mkv", "hdmv_pgs_subtitle"),
            ("dvdsub", "mkv", "dvd_subtitle"),
            // DVD-style MPEG program stream: MPEG-2 video, AC-3 and dvd_subtitle.
            ("dvdsub", "vob", "dvd_subtitle"),
            ("dvbsub", "ts", "dvb_subtitle"),
            ("xsub", "avi", "xsub"),
        ] {
            if hdr && (codec == "xsub" || extension == "vob") {
                continue;
            }
            let path = if codec == "copy" {
                source_path.clone()
            } else {
                let output = root
                    .path()
                    .join("media")
                    .join(format!("{codec}.{extension}"));
                let mut args: Vec<String> = [
                    "-nostdin",
                    "-v",
                    "error",
                    "-fix_sub_duration",
                    "-i",
                    source_path.to_str().unwrap(),
                    "-map",
                    "0",
                    "-c",
                    "copy",
                    "-c:s",
                    codec,
                ]
                .into_iter()
                .map(str::to_owned)
                .collect();
                if codec == "xsub" {
                    args.extend(["-bsf:v".into(), "h264_mp4toannexb".into()]);
                }
                if extension == "vob" {
                    args.extend(
                        [
                            "-c:v",
                            "mpeg2video",
                            "-g",
                            "12",
                            "-b:v",
                            "4M",
                            "-c:a",
                            "ac3",
                            "-f",
                            "vob",
                        ]
                        .into_iter()
                        .map(str::to_owned),
                    );
                }
                args.push(output.to_string_lossy().into_owned());
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
                output
            };
            if codec == "xsub" {
                let args = [
                    "-v",
                    "error",
                    "-select_streams",
                    "s:0",
                    "-show_frames",
                    "-of",
                    "json",
                    path.to_str().unwrap(),
                ]
                .into_iter()
                .map(str::to_owned)
                .collect();
                let decoded = media::process(
                    &app,
                    "ffprobe",
                    args,
                    None,
                    Duration::from_secs(15),
                    1 << 20,
                    app.cancel.child_token(),
                )
                .await
                .unwrap();
                let decoded: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
                let frame = &decoded["frames"][0];
                let pts = frame["pts_time"].as_str().unwrap().parse::<f64>().unwrap();
                let from = pts + frame["start_display_time"].as_f64().unwrap() / 1000.0;
                let to = pts + frame["end_display_time"].as_f64().unwrap() / 1000.0;
                assert!(
                    (from - 1.0).abs() < 0.15 && (to - 3.0).abs() < 0.15,
                    "XSUB decoded interval {from}..{to}"
                );
            }
            // Check the converted source's cue timeline and native alpha semantics
            // independently before exercising the application's seek/scaling path.
            for track in [0, 1] {
                for (at, visible) in [(0.25, false), (1.5, true), (3.5, false)] {
                    let graph = format!(
                        "[0:v:0][{}:s:{track}]overlay=eof_action=pass:repeatlast=0[v]",
                        // A second program-stream input shows DVD cues from its
                        // start, so stream-based fixtures composite one input.
                        if codec == "dvbsub" || extension == "vob" {
                            0
                        } else {
                            1
                        }
                    );
                    let mut args: Vec<String> = [
                        "-v",
                        "error",
                        "-i",
                        path.to_str().unwrap(),
                        "-filter_complex",
                        &graph,
                        "-map",
                        "[v]",
                        "-ss",
                        &at.to_string(),
                        "-frames:v",
                        "1",
                        "-pix_fmt",
                        "gray",
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                    if codec != "dvbsub" && extension != "vob" {
                        args.splice(4..4, ["-i".into(), path.to_string_lossy().into_owned()]);
                    }
                    let pixels = media::process(
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
                    assert_eq!(pixels.len(), 1280 * 720);
                    let (x, y) = if track == 0 { (240, 580) } else { (840, 120) };
                    let white = pixels[y * 1280 + x];
                    let alpha = pixels[y * 1280 + x + 80];
                    assert!(
                        if visible { white > 220 } else { white < 20 },
                        "native {codec} t{at} opaque {white}"
                    );
                    assert!(
                        if !visible {
                            alpha < 20
                        } else if codec == "xsub" {
                            alpha > 220
                        } else {
                            (90..=160).contains(&alpha)
                        },
                        "native {codec} t{at} alpha {alpha}"
                    );
                }
            }
            let saved = path.to_string_lossy().into_owned();
            app.db
                .call(move |c| {
                    c.execute(
                        "UPDATE media_files SET path=? WHERE item_id=?",
                        rusqlite::params![saved, item],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
            let info = media::inspect(&app, &path).await.unwrap();
            assert_eq!(info.hdr(), hdr);
            assert_eq!(info.tracks("subtitle").len(), 2);
            assert!(
                info.tracks("subtitle")
                    .iter()
                    .all(|t| t.codec_name == expected_codec)
            );
            assert_eq!(
                request(
                    &router,
                    "POST",
                    &format!("/api/v1/items/{item}/playback"),
                    Some(&token),
                    Some(json!({"Mode":"auto","Subtitle":2}))
                )
                .await
                .0,
                400
            );
            assert!(app.streams.lock().await.is_empty());
            for subtitle in [0, 1] {
                for (start, adaptive) in [
                    (0.0, true),
                    (1.5, true),
                    (2.5, true),
                    (3.5, true),
                    (0.0, false),
                ] {
                    let (status, session) = request(
                    &router,
                    "POST",
                    &format!("/api/v1/items/{item}/playback"),
                    Some(&token),
                    Some(json!({"Mode":"auto","Subtitle":subtitle,"Start":start,"Adaptive":adaptive})),
                )
                .await;
                    assert_eq!(status, 201, "{session}");
                    assert_eq!(session["mode"], "transcode");
                    let id = session["id"].as_str().unwrap();
                    let (status, master) = request(
                        &router,
                        "GET",
                        session["url"].as_str().unwrap(),
                        Some(&token),
                        None,
                    )
                    .await;
                    assert_eq!(status, 200, "{master}");
                    assert_eq!(
                        master["text"].as_str().unwrap().contains("v1.m3u8"),
                        adaptive
                    );
                    let stream = app.streams.lock().await.get(id).unwrap().clone();
                    tokio::time::timeout(Duration::from_secs(30), async {
                        while !stream.finished.load(std::sync::atomic::Ordering::Acquire) {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    })
                    .await
                    .unwrap();
                    let failure = stream.error.lock().await.clone();
                    assert!(failure.is_none(), "{failure:?}");
                    let samples: Vec<(f64, bool)> = if start == 0.0 {
                        vec![(0.25, false), (1.5, true), (3.5, false)]
                    } else if start < 3.0 {
                        vec![(0.25, true), (3.5 - start, false)]
                    } else {
                        vec![(0.25, false)]
                    };
                    for (variant, width, height) in [(0, 1280, 720), (1, 852, 480)]
                        .into_iter()
                        .take(if adaptive { 2 } else { 1 })
                    {
                        let output = stream.dir.join(format!("v{variant}.m3u8"));
                        let converted = media::inspect(&app, &output).await.unwrap();
                        assert_eq!(converted.video().unwrap().pix_fmt, "yuv420p");
                        if hdr {
                            assert!(!converted.hdr());
                            assert_eq!(converted.video().unwrap().color_transfer, "bt709");
                        }
                        for (at, visible) in &samples {
                            let args = [
                                "-v",
                                "error",
                                "-i",
                                output.to_str().unwrap(),
                                "-ss",
                                &at.to_string(),
                                "-frames:v",
                                "1",
                                "-pix_fmt",
                                "gray",
                                "-f",
                                "rawvideo",
                                "pipe:1",
                            ]
                            .into_iter()
                            .map(str::to_owned)
                            .collect();
                            let pixels = media::process(
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
                            assert_eq!(pixels.len(), width * height);
                            for (index, (x, y)) in [(240, 580), (840, 120)].into_iter().enumerate()
                            {
                                let x = x * width / 1280;
                                let y = y * height / 720;
                                let value = pixels[y * width + x];
                                let translucent = pixels[y * width + x + 80 * width / 1280];
                                let present = *visible && index == subtitle;
                                assert!(
                                    if present {
                                        if codec == "xsub" {
                                            translucent > 220
                                        } else {
                                            (90..=160).contains(&translucent)
                                        }
                                    } else {
                                        translucent < 20
                                    },
                                    "codec {codec}, HDR {hdr}, track {subtitle}, start {start}, sample {at}, alpha pixel {translucent}"
                                );
                                let white = *visible && index == subtitle;
                                assert!(
                                    if white { value > 220 } else { value < 20 },
                                    "codec {codec}, track {subtitle}, start {start}, t {at}, variant {variant}, region {index}, pixel {value}"
                                );
                            }
                        }
                        let args = [
                            "-v",
                            "error",
                            "-i",
                            output.to_str().unwrap(),
                            "-map",
                            "0:a:0",
                            "-t",
                            "0.25",
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
                        let audio = media::process(
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
                        let samples = audio
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|b| f32::from_le_bytes(*b))
                            .collect::<Vec<_>>();
                        assert!(samples.len() > 8000);
                        let power = samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>()
                            / samples.len() as f64;
                        assert!(power > 0.001 && power < 0.1, "audio power {power}");
                    }
                    assert_eq!(
                        request(
                            &router,
                            "DELETE",
                            &format!("/api/v1/playback/{id}"),
                            Some(&token),
                            None
                        )
                        .await
                        .0,
                        200
                    );
                    assert!(!stream.dir.exists());
                }
            }
        }
        app.shutdown().await;
    }
}

#[tokio::test]
#[ignore = "original embedded-font ASS corpus requires FFmpeg libass/libx264; software acceptance"]
async fn real_styled_subtitles_preserve_embedded_font_and_seek_timing() {
    for source_offset in [0, 5] {
        let root = TempDir::new().unwrap();
        let (app, router, token) = setup(&root).await;
        let item = add_media(&app, root.path()).await;
        let font = root.path().join("font.ttf");
        std::fs::write(&font, include_bytes!("fixtures/OVLFixtureBars.ttf")).unwrap();
        let mut inputs = vec![];
        for (index, (x, y)) in [(80, 60), (400, 180)].into_iter().enumerate() {
            let file = root.path().join(format!("track{index}.ass"));
            let script = format!(
                "[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,OVLFixtureBars,100,&H000000FF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,7,0,0,0,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{{\\an7\\pos({x},{y})}}X\n"
            );
            std::fs::write(&file, script).unwrap();
            inputs.push(file);
        }
        let path = root.path().join("media/styled.mkv");
        let mut args: Vec<String> = [
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=640x360:rate=12",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000,volume='if(between(t,1,3),1,0)':eval=frame",
            "-i",
            inputs[0].to_str().unwrap(),
            "-i",
            inputs[1].to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:s",
            "-map",
            "3:s",
            "-t",
            "5",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-threads",
            "2",
            "-c:a",
            "aac",
            "-c:s",
            "ass",
            "-attach",
            font.to_str().unwrap(),
            "-metadata:s:t:0",
            "mimetype=application/x-truetype-font",
            "-metadata:s:t:0",
            "filename=OVLFixtureBars.ttf",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if source_offset != 0 {
            args.extend(["-output_ts_offset".into(), source_offset.to_string()]);
        }
        args.push(path.to_string_lossy().into_owned());
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
        let saved = path.to_string_lossy().into_owned();
        app.db
            .call(move |c| {
                c.execute(
                    "UPDATE media_files SET path=? WHERE item_id=?",
                    rusqlite::params![saved, item],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        // A fontless control must not accidentally satisfy the distinctive glyph geometry.
        let fontless = root.path().join("fontless.mkv");
        let args = [
            "-nostdin",
            "-v",
            "error",
            "-i",
            path.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "0:a",
            "-map",
            "0:s",
            "-c",
            "copy",
            fontless.to_str().unwrap(),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        media::process(
            &app,
            "ffmpeg",
            args,
            None,
            Duration::from_secs(15),
            1024,
            app.cancel.child_token(),
        )
        .await
        .unwrap();
        let args = [
            "-v",
            "error",
            "-i",
            fontless.to_str().unwrap(),
            "-vf",
            "subtitles=filename=fontless.mkv:si=0",
            "-ss",
            "1.5",
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let control = media::process(
            &app,
            "ffmpeg",
            args,
            Some(root.path().to_path_buf()),
            Duration::from_secs(15),
            1 << 20,
            app.cancel.child_token(),
        )
        .await
        .unwrap();
        assert_eq!(control.len(), 640 * 360 * 3);
        let red = |x: usize| {
            let p = (90 * 640 + x) * 3;
            control[p] > 180 && control[p + 1] < 70 && control[p + 2] < 70
        };
        assert!(
            !(red(90) && !red(130) && red(170)),
            "fontless fallback unexpectedly matches original bars"
        );
        let info = media::inspect(&app, &path).await.unwrap();
        let (status, probe) = request(
            &router,
            "GET",
            &format!("/api/v1/items/{item}/probe"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{probe}");
        let origin = probe["format"]["start_time"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(
            (origin - f64::from(source_offset)).abs() < 0.05,
            "reported origin {origin}"
        );
        assert_eq!(info.tracks("subtitle").len(), 2);
        assert_eq!(info.tracks("attachment").len(), 1);
        assert!(
            info.tracks("subtitle")
                .iter()
                .all(|t| t.codec_name == "ass")
        );
        for subtitle in [0, 1] {
            for start in [0.0, 1.5, 3.5] {
                let (status, session) = request(
                    &router,
                    "POST",
                    &format!("/api/v1/items/{item}/playback"),
                    Some(&token),
                    Some(json!({"Mode":"auto","Subtitle":subtitle,"Start":start})),
                )
                .await;
                assert_eq!(status, 201, "{session}");
                let id = session["id"].as_str().unwrap();
                let (status, master) = request(
                    &router,
                    "GET",
                    session["url"].as_str().unwrap(),
                    Some(&token),
                    None,
                )
                .await;
                assert_eq!(status, 200, "{master}");
                let stream = app.streams.lock().await.get(id).unwrap().clone();
                tokio::time::timeout(Duration::from_secs(30), async {
                    while !stream.finished.load(std::sync::atomic::Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap();
                let failure = stream.error.lock().await.clone();
                assert!(failure.is_none(), "{failure:?}");
                let samples = if start == 0.0 {
                    vec![(0.25, false), (1.5, true), (3.5, false)]
                } else if start == 1.5 {
                    vec![(0.25, true), (1.75, false)]
                } else {
                    vec![(0.25, false)]
                };
                let output = stream.dir.join("v0.m3u8");
                for (at, visible) in samples {
                    let args = [
                        "-v",
                        "error",
                        "-i",
                        output.to_str().unwrap(),
                        "-ss",
                        &at.to_string(),
                        "-frames:v",
                        "1",
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
                        1 << 20,
                        app.cancel.child_token(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(rgb.len(), 640 * 360 * 3);
                    let args = [
                        "-v",
                        "error",
                        "-i",
                        output.to_str().unwrap(),
                        "-ss",
                        &at.to_string(),
                        "-map",
                        "0:a:0",
                        "-t",
                        "0.15",
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
                    let audio = media::process(
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
                    let samples = audio.as_chunks::<4>().0;
                    assert!(samples.len() > 6000);
                    let power = samples
                        .iter()
                        .map(|b| f64::from(f32::from_le_bytes(*b)).powi(2))
                        .sum::<f64>()
                        / samples.len() as f64;
                    assert!(
                        if visible {
                            power > 0.001 && power < 0.1
                        } else {
                            power < 0.00001
                        },
                        "audio timing at start {start}, sample {at}, power {power}"
                    );
                    for (index, (x, y)) in [(80, 60), (400, 180)].into_iter().enumerate() {
                        for (dx, colored) in [(10, true), (50, false), (90, true)] {
                            let p = ((y + 30) * 640 + x + dx) * 3;
                            let pixel = &rgb[p..p + 3];
                            let red = visible && index == subtitle && colored;
                            assert!(
                                if red {
                                    pixel[0] > 180 && pixel[1] < 70 && pixel[2] < 70
                                } else {
                                    pixel.iter().all(|v| *v < 20)
                                },
                                "track {subtitle}, start {start}, sample {at}, region {index}, dx {dx}, RGB {pixel:?}"
                            );
                        }
                    }
                }
                assert_eq!(
                    request(
                        &router,
                        "DELETE",
                        &format!("/api/v1/playback/{id}"),
                        Some(&token),
                        None
                    )
                    .await
                    .0,
                    200
                );
                assert!(!stream.dir.exists());
            }
        }
        app.shutdown().await;
    }
}

#[test]
fn negotiation_respects_audio_channel_limits_and_unknown_sources() {
    let mut info: media::Info = serde_json::from_value(json!({"streams":[
        {"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p"},
        {"codec_type":"audio","codec_name":"aac","channels":6},
        {"codec_type":"audio","codec_name":"aac","channels":2}
    ],"format":{"format_name":"mp4"}}))
    .unwrap();
    let mut b: Playback = serde_json::from_value(json!({"Mode":"auto","CanPlayMP4":true,
        "Capabilities":{"containers":["mp4"],"video_codecs":["h264"],"audio_codecs":["aac"],"hdr":false}})).unwrap();
    for channels in [0, 1, 2, 6, 8] {
        info.streams[1].channels = channels;
        for limit in [None, Some(1), Some(2), Some(6), Some(8)] {
            b.capabilities.as_mut().unwrap().max_audio_channels = limit;
            let expected = if channels > 0 && channels <= u32::from(limit.unwrap_or(2)) {
                "direct"
            } else {
                "remux"
            };
            assert_eq!(
                playback::negotiate(&info, &b).0,
                expected,
                "source {channels}, limit {limit:?}"
            );
        }
        let mut legacy = b.clone();
        legacy.capabilities = None;
        assert_eq!(
            playback::negotiate(&info, &legacy).0,
            if (1..=2).contains(&channels) {
                "direct"
            } else {
                "remux"
            }
        );
    }
    info.streams[1].channels = 2;
    for (source, limit, mode) in [
        ("", None, "direct"),
        ("", Some(48_000), "remux"),
        ("48000", Some(48_000), "direct"),
        ("48001", Some(48_000), "remux"),
        ("8000", Some(8_000), "direct"),
        ("384000", Some(384_000), "direct"),
    ] {
        info.streams[1].sample_rate = source.into();
        b.capabilities.as_mut().unwrap().max_audio_sample_rate = limit;
        assert_eq!(
            playback::negotiate(&info, &b).0,
            mode,
            "{source}, {limit:?}"
        );
    }
    b.capabilities.as_mut().unwrap().max_audio_sample_rate = Some(8_000);
    let args = playback::encoding_args(
        std::path::Path::new("fixture.mp4"),
        &b,
        &info,
        &playback::ladder(320, 180, false),
    );
    assert!(args.windows(2).any(|pair| pair == ["-ar", "8000"]));
    b.audio = 1;
    assert_eq!(
        playback::negotiate(&info, &b).0,
        "remux",
        "secondary track must be selected by conversion"
    );
    b.audio = 0;
    info.streams.truncate(1);
    assert_eq!(
        playback::negotiate(&info, &b).0,
        "direct",
        "silent video needs no audio conversion"
    );
    let null: Playback = serde_json::from_value(json!({"Capabilities":{"containers":[],"video_codecs":[],"audio_codecs":[],"hdr":false,"max_audio_channels":null}})).unwrap();
    assert_eq!(null.capabilities.unwrap().max_audio_channels, None);
}

#[tokio::test]
async fn invalid_audio_channel_reports_fail_before_probe_or_allocation() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    // The source is deliberately not valid media: probing would fail with 502.
    for limit in [
        json!(0),
        json!(9),
        json!(256),
        json!(-1),
        json!(1.5),
        json!("2"),
    ] {
        for mode in ["auto", "direct", "remux", "transcode"] {
            let (status,body)=request(&router,"POST",&format!("/api/v1/items/{item}/playback"),Some(&token),
                Some(json!({"Mode":mode,"Capabilities":{"containers":["mp4"],"video_codecs":["h264"],"audio_codecs":["aac"],"hdr":false,"max_audio_channels":limit}}))).await;
            assert_eq!(status, 400, "{mode}, {limit}: {body}");
            assert!(app.streams.lock().await.is_empty());
        }
    }
    for (rate, expected) in [(7999, 400), (8000, 200), (384000, 200), (384001, 400)] {
        let (status, body) = request(
            &router,
            "POST",
            &format!("/api/v1/items/{item}/playback"),
            Some(&token),
            Some(
                json!({"Mode":"direct","Capabilities":{"containers":["mp4"],"video_codecs":["h264"],
                "audio_codecs":["aac"],"hdr":false,"max_audio_sample_rate":rate}}),
            ),
        )
        .await;
        assert_eq!(status, expected, "{rate}: {body}");
    }
    let temporary = app.config.data.join("transcodes");
    assert!(!temporary.exists() || std::fs::read_dir(temporary).unwrap().count() == 0);
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "requires FFmpeg AAC 5.1 encode/decode and libx264; decoded channel-tone software acceptance"]
async fn real_surround_audio_negotiates_and_downmixes_selected_channels() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    let path = root.path().join("media/Example.S01E01.mp4");
    let args=["-nostdin","-v","error","-y","-f","lavfi","-i","testsrc2=size=320x180:rate=12",
        "-f","lavfi","-i","aevalsrc=0.08*sin(2*PI*400*t)|0.08*sin(2*PI*600*t)|0.08*sin(2*PI*800*t)|0.08*sin(2*PI*80*t)|0.08*sin(2*PI*1000*t)|0.08*sin(2*PI*1200*t):s=48000:c=5.1",
        "-f","lavfi","-i","aevalsrc=0.08*sin(2*PI*1600*t)|0.08*sin(2*PI*1800*t):s=48000:c=stereo",
        "-map","0:v:0","-map","1:a:0","-map","2:a:0","-t","3","-c:v","libx264","-threads","2","-g","12","-pix_fmt","yuv420p",
        "-c:a","aac","-b:a:0","384k","-b:a:1","128k",path.to_str().unwrap()].into_iter().map(str::to_owned).collect();
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
    let (status, probe) = request(
        &router,
        "GET",
        &format!("/api/v1/items/{item}/probe"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{probe}");
    let info: media::Info = serde_json::from_value(probe).unwrap();
    assert_eq!(
        info.tracks("audio")
            .iter()
            .map(|a| a.channels)
            .collect::<Vec<_>>(),
        vec![6, 2]
    );
    // The source is valid H.264/AAC, but these clients cannot decode the
    // conversion that automatic mode would otherwise allocate.
    for (video, formats, audio, expected) in [
        (json!([]), json!({}), json!(["aac"]), "H.264"),
        (json!(["h264"]), json!({"h264":[]}), json!(["aac"]), "H.264"),
        (json!(["h264"]), json!({}), json!(["opus"]), "AAC"),
    ] {
        let (status, body) = request(
            &router,
            "POST",
            &format!("/api/v1/items/{item}/playback"),
            Some(&token),
            Some(
                json!({"Mode":"auto","Capabilities":{"containers":[],"video_codecs":video,
                "video_pixel_formats":formats,"audio_codecs":audio,"hdr":false}}),
            ),
        )
        .await;
        assert_eq!(status, 422, "{body}");
        assert!(body.to_string().contains(expected), "{body}");
        assert!(app.streams.lock().await.is_empty());
    }
    // Mono and stereo conversion, legacy default, explicit six-channel direct,
    // secondary-track selection and a seeked full-video conversion.
    for (limit, audio, mode, start, expected) in [
        (Some(2), 0, "auto", 0.0, "remux"),
        (Some(1), 0, "auto", 0.0, "remux"),
        (None, 0, "auto", 0.0, "remux"),
        (Some(6), 0, "auto", 0.0, "direct"),
        (Some(2), 1, "auto", 0.0, "remux"),
        (Some(2), 0, "transcode", 1.5, "transcode"),
    ] {
        let (status,session)=request(&router,"POST",&format!("/api/v1/items/{item}/playback"),Some(&token),
            Some(json!({"Mode":mode,"Audio":audio,"Start":start,"Capabilities":{"containers":["mov","mp4","m4a","3gp","3g2","mj2"],"video_codecs":["h264"],"audio_codecs":["aac"],"hdr":false,"max_audio_channels":limit}}))).await;
        assert!(status == 200 || status == 201, "{session}");
        assert_eq!(session["mode"], expected, "{session}");
        if expected == "direct" {
            continue;
        }
        let id = session["id"].as_str().unwrap();
        let (status, master) = request(
            &router,
            "GET",
            session["url"].as_str().unwrap(),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{master}");
        let output = app.config.data.join("transcodes").join(id).join("v0.m3u8");
        let converted = media::inspect(&app, &output).await.unwrap();
        let channels = limit.unwrap_or(2).min(2) as usize;
        assert_eq!(converted.tracks("audio")[0].channels, channels as u32);
        assert_eq!(converted.tracks("audio")[0].codec_name, "aac");
        let args = [
            "-v",
            "error",
            "-i",
            output.to_str().unwrap(),
            "-ss",
            "0.25",
            "-map",
            "0:a:0",
            "-t",
            "0.5",
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
        let samples: Vec<f64> = pcm
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f64::from(f32::from_le_bytes(*b)))
            .collect();
        assert!(samples.len() >= 23_000 * channels);
        assert!(
            samples.iter().all(|s| s.is_finite() && s.abs() < 0.98),
            "nonfinite or clipped audio"
        );
        let amplitude = |channel: usize, hz: f64| {
            let signal: Vec<_> = samples
                .iter()
                .skip(channel)
                .step_by(channels)
                .copied()
                .collect();
            let mut re = 0.0;
            let mut im = 0.0;
            for (n, sample) in signal.iter().enumerate() {
                let phase = std::f64::consts::TAU * hz * n as f64 / 48000.0;
                re += sample * phase.cos();
                im += sample * phase.sin();
            }
            2.0 * re.hypot(im) / signal.len() as f64
        };
        if audio == 1 {
            assert!(
                amplitude(0, 1600.0) > 0.04 && amplitude(1, 1800.0) > 0.04,
                "wrong secondary track"
            );
            assert!(
                amplitude(0, 400.0) < 0.002 && amplitude(1, 600.0) < 0.002,
                "primary track leaked"
            );
            assert!(
                amplitude(0, 1800.0) < 0.002 && amplitude(1, 1600.0) < 0.002,
                "secondary channel swap/leak"
            );
        } else {
            for hz in [400.0, 600.0, 800.0, 1000.0, 1200.0] {
                assert!(
                    (0..channels).map(|c| amplitude(c, hz)).fold(0.0, f64::max) > 0.008,
                    "lost {hz}Hz channel, cap {limit:?}"
                );
            }
            if channels == 2 {
                for hz in [400.0, 1000.0] {
                    assert!(
                        amplitude(0, hz) > 4.0 * amplitude(1, hz),
                        "left channel {hz} misrouted"
                    );
                }
                for hz in [600.0, 1200.0] {
                    assert!(
                        amplitude(1, hz) > 4.0 * amplitude(0, hz),
                        "right channel {hz} misrouted"
                    );
                }
                assert!(
                    (amplitude(0, 800.0) / amplitude(1, 800.0) - 1.0).abs() < 0.15,
                    "center not balanced"
                );
            }
        }
        assert_eq!(
            request(
                &router,
                "DELETE",
                &format!("/api/v1/playback/{id}"),
                Some(&token),
                None
            )
            .await
            .0,
            200
        );
        assert!(!app.config.data.join("transcodes").join(id).exists());
    }
    app.shutdown().await;
}

#[test]
fn playable_video_selection_skips_artwork_and_timed_thumbnails() {
    let info: media::Info = serde_json::from_value(json!({"streams":[
        {"index":0,"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},
        {"index":1,"codec_type":"video","codec_name":"h264","disposition":{"timed_thumbnails":1}},
        {"index":2,"codec_type":"audio"},
        {"index":3,"codec_type":"video","codec_name":"hevc","color_transfer":"smpte2084"}
    ]}))
    .unwrap();
    assert_eq!(info.video().unwrap().index, 3);
    assert!(info.hdr());
    let args = playback::encoding_args(
        Path::new("movie.mp4"),
        &Playback {
            mode: "transcode".into(),
            ..Default::default()
        },
        &info,
        &playback::ladder(320, 180, false),
    );
    assert!(args.windows(2).any(|a| a == ["-map", "0:3"]));
}

#[tokio::test]
#[ignore = "requires FFmpeg AAC, MJPEG and H.264 encoders"]
async fn real_embedded_artwork_does_not_count_as_playable_video() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    let path = root.path().join("media/Example.S01E01.mp4");
    for movie in [false, true] {
        let mut args = vec![
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-f",
            "lavfi",
            "-i",
            "color=red:size=64x64:rate=1",
        ];
        if movie {
            args.extend(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=12"]);
        }
        args.extend([
            "-map",
            "0:a:0",
            "-map",
            "1:v:0",
            "-c:a",
            "aac",
            "-c:v:0",
            "mjpeg",
            "-threads:v:0",
            "1",
            "-frames:v:0",
            "1",
            "-disposition:v:0",
            "attached_pic",
        ]);
        if movie {
            args.extend([
                "-map",
                "2:v:0",
                "-c:v:1",
                "libx264",
                "-threads:v:1",
                "2",
                "-pix_fmt:v:1",
                "yuv420p",
            ]);
        }
        args.extend(["-t", "2", path.to_str().unwrap()]);
        media::process(
            &app,
            "ffmpeg",
            args.into_iter().map(str::to_owned).collect(),
            None,
            Duration::from_secs(20),
            1024,
            app.cancel.child_token(),
        )
        .await
        .unwrap();
        let info = media::inspect(&app, &path).await.unwrap();
        assert!(info.streams.iter().any(|t| t.disposition.attached_pic == 1));
        assert_eq!(info.video().is_some(), movie);
        let (status, session) = request(
            &router,
            "POST",
            &format!("/api/v1/items/{item}/playback"),
            Some(&token),
            Some(json!({"Mode":"auto","CanPlayMP4":false})),
        )
        .await;
        if !movie {
            assert_eq!(status, 422, "{session}");
            assert!(app.streams.lock().await.is_empty());
            continue;
        }
        assert_eq!(status, 201, "{session}");
        assert_eq!(session["mode"], "remux");
        let (status, master) = request(
            &router,
            "GET",
            session["url"].as_str().unwrap(),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{master}");
        let id = session["id"].as_str().unwrap();
        let output = app.config.data.join("transcodes").join(id).join("v0.m3u8");
        let info = media::inspect(&app, &output).await.unwrap();
        let v = info.video().unwrap();
        assert_eq!(
            (v.codec_name.as_str(), v.width, v.height),
            ("h264", 320, 180)
        );
        assert_eq!(
            request(
                &router,
                "DELETE",
                &format!("/api/v1/playback/{id}"),
                Some(&token),
                None
            )
            .await
            .0,
            200
        );
    }
    app.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires FFmpeg audio encoders and FFprobe"]
async fn real_music_library_tags_playback_and_current_grants() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let dir = root.path().join("music");
    std::fs::create_dir(&dir).unwrap();
    let cover = root.path().join("fixture-cover.png");
    media::process(
        &app,
        "ffmpeg",
        [
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=red:size=640x320",
            "-frames:v",
            "1",
            "-threads",
            "1",
            cover.to_str().unwrap(),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        None,
        Duration::from_secs(20),
        1024,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    let cases = [
        ("mp3", "libmp3lame", "mp3", "mp3"),
        ("m4a", "aac", "m4a", "aac"),
        ("flac", "flac", "flac", "flac"),
        ("ogg", "libvorbis", "ogg", "vorbis"),
        ("opus", "libopus", "ogg", "opus"),
        ("wav", "pcm_s16le", "wav", "pcm_s16le"),
    ];
    for (extension, encoder, _, _) in cases {
        let path = dir.join(format!("S01E01 Music {extension}.{extension}"));
        let mut args = vec![
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
        ];
        if matches!(extension, "mp3" | "flac") {
            args.extend([
                "-i",
                cover.to_str().unwrap(),
                "-map",
                "0:a:0",
                "-map",
                "1:v:0",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
            ]);
        }
        if matches!(extension, "mp3" | "flac") {
            args.extend([
                "-metadata",
                "album_artist=Fixture band",
                "-metadata",
                if extension == "mp3" {
                    "track=2/2"
                } else {
                    "track=1/2"
                },
            ]);
        }
        args.extend([
            "-t",
            "1",
            "-c:a",
            encoder,
            "-metadata",
            "TITLE=Tagged track",
            "-metadata",
            "ARTIST=Fixture artist",
            "-metadata",
            "ALBUM=Fixture album",
            "-metadata",
            "comment=private comment",
            "-metadata",
            "LYRICS=[00:00.10]Embedded lyric",
            path.to_str().unwrap(),
        ]);
        media::process(
            &app,
            "ffmpeg",
            args.into_iter().map(str::to_owned).collect(),
            None,
            Duration::from_secs(20),
            1024,
            app.cancel.child_token(),
        )
        .await
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    let single = dir.join("Standalone track.mp3");
    media::process(
        &app,
        "ffmpeg",
        [
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:a",
            "libmp3lame",
            "-metadata",
            "artist=Fixture band",
            single.to_str().unwrap(),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        None,
        Duration::from_secs(20),
        1024,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    std::fs::set_permissions(single, std::os::unix::fs::PermissionsExt::from_mode(0o444)).unwrap();
    std::fs::write(dir.join("Disguised.mp3"), b"#EXTM3U\n#EXT-X-VERSION:3\n").unwrap();
    std::fs::write(dir.join("ignore.mp4"), b"not in the music library").unwrap();
    let outside = root.path().join("outside.mp3");
    std::fs::write(&outside, b"outside media").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("escape.mp3")).unwrap();
    let (status, library) = request(
        &router,
        "POST",
        "/api/v1/libraries",
        Some(&token),
        Some(json!({"Name":"Music fixture","Path":dir,"Kind":"music"})),
    )
    .await;
    assert_eq!(status, 201, "{library}");
    assert_eq!(
        request(&router, "POST", "/api/v1/scan", Some(&token), None)
            .await
            .0,
        202
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while request(&router, "GET", "/api/v1/scan", Some(&token), None)
            .await
            .1["running"]
            == true
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let (_, items) = request(&router, "GET", "/api/v1/items", Some(&token), None).await;
    assert_eq!(items.as_array().unwrap().len(), cases.len() + 2);
    let (_, series) = request(&router, "GET", "/api/v1/series", Some(&token), None).await;
    assert_eq!(
        series,
        json!([]),
        "episode-like track names must not become TV series"
    );
    let (status,viewer)=request(&router,"POST","/api/v1/users",Some(&token),
        Some(json!({"Name":"music-viewer","Password":"test-password-123","Libraries":[library["id"]]}))).await;
    assert_eq!(status, 201, "{viewer}");
    let listener = account(&router, "music-viewer").await;
    assert_eq!(
        request(
            &router,
            "GET",
            "/api/v1/music/albums?offset=100001",
            Some(&listener),
            None
        )
        .await
        .0,
        400
    );
    let albums = request(
        &router,
        "GET",
        "/api/v1/music/albums?q=Fixture%20band",
        Some(&listener),
        None,
    )
    .await
    .1;
    assert_eq!(
        albums,
        json!([{"library_id":library["id"],"artist":"Fixture band","album":"Fixture album","tracks":2}])
    );
    let (status, artists) = request(
        &router,
        "GET",
        "/api/v1/music/artists?q=Fixture%20band",
        Some(&listener),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        artists,
        json!([{"library_id":library["id"],"artist":"Fixture band","albums":1,"tracks":3}])
    );
    let artist_tracks = format!(
        "/api/v1/music/tracks?library={}&artist=Fixture%20band",
        library["id"]
    );
    let (status, all_tracks) = request(&router, "GET", &artist_tracks, Some(&listener), None).await;
    assert_eq!(status, 200);
    assert_eq!(all_tracks.as_array().unwrap().len(), 3);
    assert_eq!(all_tracks[0]["title"], "Standalone track");
    let (status, rows) = request(
        &router,
        "GET",
        &format!("{artist_tracks}&album="),
        Some(&listener),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(rows, json!([]));
    let group_url = format!(
        "/api/v1/music/tracks?library={}&artist=Fixture%20band&album=Fixture%20album",
        library["id"]
    );
    let tracks = request(&router, "GET", &group_url, Some(&listener), None)
        .await
        .1;
    let file_id = |name: &str| {
        items
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["title"] == name)
            .unwrap()["id"]
            .clone()
    };
    assert_eq!(tracks[0]["id"], file_id("S01E01 Music flac"));
    assert_eq!(tracks[1]["id"], file_id("S01E01 Music mp3"));
    assert_eq!(tracks[0]["title"], "Tagged track");
    for (extension, _, container, codec) in cases {
        let item = items
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["title"] == format!("S01E01 Music {extension}"))
            .unwrap();
        let id = item["id"].as_i64().unwrap();
        let (status, probe) = request(
            &router,
            "GET",
            &format!("/api/v1/items/{id}/probe"),
            Some(&listener),
            None,
        )
        .await;
        assert_eq!(status, 200, "{probe}");
        assert_eq!(probe["format"]["format_name"], container);
        assert!(probe["format"]["tags"].get("comment").is_none());
        assert!(!probe.to_string().contains("private comment"));
        if extension == "mp3" || extension == "opus" {
            assert_eq!(probe["format"]["tags"]["artist"], "Fixture artist");
            assert_eq!(probe["format"]["tags"]["album"], "Fixture album");
        }
        assert!(!probe.to_string().contains("Embedded lyric"));
        let lyrics = format!("/api/v1/items/{id}/lyrics");
        let (status, body) = request(&router, "GET", &lyrics, Some(&listener), None).await;
        assert_eq!(status, 200, "{extension}: {body}");
        let expected = if ["mp3", "flac", "ogg", "opus"].contains(&extension) {
            json!({"lines":[{"time_ms":100,"text":"Embedded lyric"}]})
        } else {
            json!({"lines":[]})
        };
        assert_eq!(body, expected, "{extension}");
        if extension == "mp3" {
            std::fs::write(dir.join("S01E01 Music mp3.txt"), "Local override").unwrap();
            assert_eq!(
                request(&router, "GET", &lyrics, Some(&listener), None)
                    .await
                    .1,
                json!({"lines":[{"time_ms":null,"text":"Local override"}]})
            );
        }
        let cover_url = format!("/api/v1/items/{id}/artwork");
        assert_eq!(
            request(&router, "GET", &cover_url, Some(&listener), None)
                .await
                .0,
            404
        );
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{cover_url}?embedded=true"))
                    .header("Authorization", format!("Bearer {listener}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if matches!(extension, "mp3" | "flac") {
            assert_eq!(response.status(), 200, "{extension}");
            assert_eq!(response.headers()["content-type"], "image/png");
            let png = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(
                &png[16..24],
                &[0, 0, 2, 0, 0, 0, 1, 0],
                "512x256 scaled cover"
            );
        } else {
            assert_eq!(response.status(), 404);
        }
        let path = format!("/api/v1/items/{id}/playback");
        let (status, session) = request(
            &router,
            "POST",
            &path,
            Some(&listener),
            Some(
                json!({"Mode":"auto","Capabilities":{"containers":[container],"video_codecs":[],
                "audio_codecs":[codec],"hdr":false}}),
            ),
        )
        .await;
        assert_eq!(status, 200, "{session}");
        assert_eq!(session["mode"], "direct");
        assert_eq!(session["url"], format!("/api/v1/items/{id}/file"));
        assert_eq!(
            request(
                &router,
                "POST",
                &path,
                Some(&listener),
                Some(
                    json!({"Mode":"auto","Capabilities":{"containers":[container],"video_codecs":[],
                "audio_codecs":[],"hdr":false}})
                )
            )
            .await
            .0,
            422
        );
        assert_eq!(
            request(
                &router,
                "POST",
                &path,
                Some(&listener),
                Some(json!({"Mode":"transcode"}))
            )
            .await
            .0,
            422
        );
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(session["url"].as_str().unwrap())
                    .header("Authorization", format!("Bearer {listener}"))
                    .header("Range", "bytes=0-63")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        let content = response.into_body().collect().await.unwrap().to_bytes();
        let original =
            std::fs::read(dir.join(format!("S01E01 Music {extension}.{extension}"))).unwrap();
        assert_eq!(&content[..], &original[..64]);
    }
    let (status, converted) = request(
        &router, "POST", &format!("/api/v1/items/{}/playback", file_id("S01E01 Music flac")),
        Some(&listener), Some(json!({"Mode":"auto","Start":0.25,"Capabilities":{
            "containers":["flac","hls"],"video_codecs":[],"audio_codecs":["flac","aac"],"max_audio_channels":1,"max_audio_sample_rate":25000}})),
    ).await;
    assert_eq!(status, 201, "{converted}");
    assert_eq!(converted["mode"], "transcode");
    let id = converted["id"].as_str().unwrap();
    let base = format!("/api/v1/playback/{id}");
    for (playlist, entry) in [
        (converted["url"].as_str().unwrap().to_owned(), "v0.m3u8"),
        (format!("{base}/v0.m3u8"), "v0-0.ts"),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(playlist)
                    .header("Authorization", format!("Bearer {listener}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .lines()
                .any(|line| line == entry)
        );
    }
    let segment = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{base}/v0-0.ts"))
                .header("Authorization", format!("Bearer {listener}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(segment.status(), 200);
    let sample = root.path().join("converted.ts");
    std::fs::write(
        &sample,
        segment.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap();
    let info = media::inspect(&app, &sample).await.unwrap();
    assert!(info.video().is_none());
    assert_eq!(info.tracks("audio")[0].codec_name, "aac");
    assert_eq!(info.tracks("audio")[0].channels, 1);
    assert_eq!(info.tracks("audio")[0].sample_rate, "24000");
    let decoded = media::process(
        &app,
        "ffmpeg",
        [
            "-v",
            "error",
            "-i",
            sample.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            "8000",
            "pipe:1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        None,
        Duration::from_secs(10),
        64_000,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    assert!(
        (11_000..=14_000).contains(&decoded.len()),
        "unexpected seeked audio length: {}",
        decoded.len()
    );
    assert!(
        decoded
            .as_chunks::<2>()
            .0
            .iter()
            .any(|b| i16::from_le_bytes(*b).unsigned_abs() > 1000)
    );
    assert_eq!(
        request(&router, "DELETE", &base, Some(&listener), None)
            .await
            .0,
        200
    );
    assert!(app.streams.lock().await.is_empty());
    assert_eq!(
        request(
            &router,
            "PUT",
            &format!("/api/v1/users/{}/access", viewer["id"]),
            Some(&token),
            Some(json!({"Libraries":[]}))
        )
        .await
        .0,
        200
    );
    for url in ["/api/v1/music/artists", artist_tracks.as_str()] {
        let (status, rows) = request(&router, "GET", url, Some(&listener), None).await;
        assert_eq!(status, 200);
        assert_eq!(rows, json!([]));
    }
    assert_eq!(
        request(&router, "GET", &group_url, Some(&listener), None)
            .await
            .1,
        json!([])
    );
    assert_eq!(
        request(
            &router,
            "GET",
            "/api/v1/music/albums",
            Some(&listener),
            None
        )
        .await
        .1,
        json!([])
    );
    let id = items[0]["id"].as_i64().unwrap();
    for (method, suffix) in [
        ("GET", "probe"),
        ("GET", "file"),
        ("POST", "playback"),
        ("GET", "artwork"),
    ] {
        assert_eq!(
            request(
                &router,
                method,
                &format!("/api/v1/items/{id}/{suffix}"),
                Some(&listener),
                (method == "POST").then(|| json!({"Mode":"auto"}))
            )
            .await
            .0,
            404
        );
    }
    app.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn local_lyrics_are_bounded_and_follow_current_music_grants() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let canonical = std::fs::canonicalize(root.path()).unwrap();
    let id = add_media(&app, &canonical).await;
    let url = format!("/api/v1/items/{id}/lyrics");
    let lrc = root.path().join("media/Example.S01E01.lrc");
    let txt = root.path().join("media/Example.S01E01.txt");
    std::fs::write(
        &lrc,
        "\u{feff}[ar:Metadata]\n[00:02.5][00:01.025]Second and first\n[00:03]Third\n",
    )
    .unwrap();
    assert_eq!(
        request(&router, "GET", &url, Some(&token), None).await.0,
        404
    );
    app.db
        .call(|c| {
            c.execute(
                "INSERT INTO ovl_music_libraries SELECT id FROM libraries",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, body) = request(&router, "GET", &url, Some(&token), None).await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({"lines":[{"time_ms":1025,"text":"Second and first"},{"time_ms":2500,"text":"Second and first"},{"time_ms":3000,"text":"Third"}]})
    );
    std::fs::remove_file(&lrc).unwrap();
    let outside = root.path().join("outside.lrc");
    std::fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(&outside, &lrc).unwrap();
    std::fs::write(&txt, "Plain line\n<script>text</script>\n").unwrap();
    assert_eq!(
        request(&router, "GET", &url, Some(&token), None).await.1,
        json!({"lines":[{"time_ms":null,"text":"Plain line"},{"time_ms":null,"text":"<script>text</script>"}]})
    );
    std::fs::remove_file(&lrc).unwrap();
    std::fs::write(&lrc, vec![b'a'; 128 * 1024 + 1]).unwrap();
    assert_eq!(
        request(&router, "GET", &url, Some(&token), None).await.0,
        422
    );
    std::fs::write(&lrc, format!("{}text", "[00:01]".repeat(4097))).unwrap();
    assert_eq!(
        request(&router, "GET", &url, Some(&token), None).await.0,
        422
    );
    std::fs::write(
        &lrc,
        format!("{}{}", "[00:01]".repeat(300), "x".repeat(512)),
    )
    .unwrap();
    assert_eq!(
        request(&router, "GET", &url, Some(&token), None).await.0,
        422
    );
    let original = canonical.join("original");
    let outside_root = canonical.join("outside");
    std::fs::create_dir(&outside_root).unwrap();
    std::fs::write(outside_root.join("Example.S01E01.mp4"), b"outside track").unwrap();
    std::fs::write(outside_root.join("Example.S01E01.lrc"), b"[00:01]outside").unwrap();
    std::fs::rename(canonical.join("media"), &original).unwrap();
    std::os::unix::fs::symlink(&outside_root, canonical.join("media")).unwrap();
    assert_eq!(
        request(&router, "GET", &url, Some(&token), None).await.0,
        404
    );
    std::fs::remove_file(canonical.join("media")).unwrap();
    std::fs::rename(original, canonical.join("media")).unwrap();
    let (_, viewer) = request(
        &router,
        "POST",
        "/api/v1/users",
        Some(&token),
        Some(json!({"Name":"lyric-viewer","Password":"test-password-123","Libraries":[1]})),
    )
    .await;
    let listener = account(&router, "lyric-viewer").await;
    std::fs::remove_file(&lrc).unwrap();
    assert_eq!(
        request(&router, "GET", &url, Some(&listener), None).await.0,
        200
    );
    assert_eq!(
        request(
            &router,
            "PUT",
            &format!("/api/v1/users/{}/access", viewer["id"]),
            Some(&token),
            Some(json!({"Libraries":[]}))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(&router, "GET", &url, Some(&listener), None).await.0,
        404
    );
    app.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn replaced_library_root_is_unavailable_without_losing_queue_or_records() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    let media = root.path().join("media");
    let original = root.path().join("original");
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("Example.S01E01.mp4"), b"outside bytes").unwrap();
    std::fs::write(outside.join("Outside.mp4"), b"outside title").unwrap();
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/v1/lists",
            Some(&token),
            Some(json!({"Kind":"queue","Name":"Root replacement","ItemIDs":[id]}))
        )
        .await
        .0,
        201
    );
    let queue = "/api/v1/lists/1";
    assert_eq!(
        request(&router, "GET", queue, Some(&token), None).await.1["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    std::fs::rename(&media, &original).unwrap();
    std::os::unix::fs::symlink(&outside, &media).unwrap();
    for (method, suffix) in [
        ("GET", "file"),
        ("GET", "probe"),
        ("GET", "artwork"),
        ("POST", "playback"),
        ("POST", "subtitles"),
    ] {
        assert_eq!(
            request(
                &router,
                method,
                &format!("/api/v1/items/{id}/{suffix}"),
                Some(&token),
                (method == "POST").then(|| json!({"Mode":"direct"}))
            )
            .await
            .0,
            404
        );
    }
    let saved = request(&router, "GET", queue, Some(&token), None).await.1;
    assert_eq!(saved["entry_count"], 1);
    assert_eq!(saved["entries"], json!([]));
    for offline in [true, false] {
        if !offline {
            std::fs::remove_file(&media).unwrap();
            std::fs::rename(&original, &media).unwrap();
        }
        assert_eq!(
            request(&router, "POST", "/api/v1/scan", Some(&token), None)
                .await
                .0,
            202
        );
        tokio::time::timeout(Duration::from_secs(10), async {
            while request(&router, "GET", "/api/v1/scan", Some(&token), None)
                .await
                .1["running"]
                == true
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let libs = request(&router, "GET", "/api/v1/libraries", Some(&token), None)
            .await
            .1;
        assert_eq!(libs[0]["offline"], offline);
        let items = request(&router, "GET", "/api/v1/items", Some(&token), None)
            .await
            .1;
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["id"], id);
    }
    assert_eq!(
        request(&router, "GET", queue, Some(&token), None).await.1["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn music_artwork_prefers_album_cover_and_enforces_grants() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let id = add_media(&app, root.path()).await;
    let dir = root.path().join("media");
    std::fs::write(dir.join("folder.jpg"), b"folder fixture").unwrap();
    std::fs::write(dir.join("cover.png"), b"cover fixture").unwrap();
    let url = format!("/api/v1/items/{id}/artwork");
    async fn bytes(router: &Router, token: &str, url: &str) -> (StatusCode, Vec<u8>) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(url)
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        (
            response.status(),
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
    }
    assert_eq!(
        bytes(&router, &token, &url).await,
        (StatusCode::OK, b"folder fixture".to_vec())
    );
    app.db
        .call(|c| {
            c.execute(
                "INSERT INTO ovl_music_libraries SELECT id FROM libraries",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, viewer) = request(
        &router,
        "POST",
        "/api/v1/users",
        Some(&token),
        Some(json!({"Name":"cover-viewer","Password":"test-password-123","Libraries":[1]})),
    )
    .await;
    assert_eq!(status, 201);
    let viewer_token = account(&router, "cover-viewer").await;
    assert_eq!(
        bytes(&router, &viewer_token, &url).await,
        (StatusCode::OK, b"cover fixture".to_vec())
    );
    std::fs::remove_file(dir.join("cover.png")).unwrap();
    let nonimage = dir.join("payload.html");
    std::fs::write(&nonimage, b"<script>test</script>").unwrap();
    std::os::unix::fs::symlink(&nonimage, dir.join("cover.png")).unwrap();
    assert_eq!(
        bytes(&router, &viewer_token, &url).await,
        (StatusCode::OK, b"folder fixture".to_vec())
    );
    std::fs::remove_file(dir.join("cover.png")).unwrap();
    let outside = root.path().join("outside.png");
    std::fs::write(&outside, b"outside fixture").unwrap();
    std::os::unix::fs::symlink(outside, dir.join("cover.png")).unwrap();
    assert_eq!(
        bytes(&router, &viewer_token, &url).await,
        (StatusCode::OK, b"folder fixture".to_vec())
    );
    assert_eq!(
        request(
            &router,
            "PUT",
            &format!("/api/v1/users/{}/access", viewer["id"]),
            Some(&token),
            Some(json!({"Libraries":[]}))
        )
        .await
        .0,
        200
    );
    assert_eq!(bytes(&router, &viewer_token, &url).await.0, 404);
    app.shutdown().await;
}

/// Decodes the first 416x240 frame presented at or after timestamp `t` of `src`
/// (timestamps preserved) and returns the frame number encoded by the
/// long-seek fixture's sixteen binary bars. Selecting by timestamp avoids
/// FFmpeg's output-seek skew when a stream starts later than another.
async fn coded_frame(app: &Arc<App>, src: &Path, t: f64) -> u32 {
    let args = [
        "-v",
        "error",
        "-copyts",
        "-i",
        src.to_str().unwrap(),
        "-map",
        "0:v:0",
        "-vf",
        &format!("select='gte(t\\,{t:.4})',scale=416:240"),
        "-frames:v",
        "1",
        "-pix_fmt",
        "gray",
        "-f",
        "rawvideo",
        "pipe:1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let gray = media::process(
        app,
        "ffmpeg",
        args,
        None,
        Duration::from_secs(60),
        1 << 20,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    assert_eq!(gray.len(), 416 * 240, "no decoded frame at {t}");
    (0..16)
        .filter(|bit| gray[120 * 416 + bit * 26 + 13] > 128)
        .map(|bit| 1 << bit)
        .sum()
}

/// Returns the delay after timestamp `t` of the next one-second beep onset in
/// `src` (timestamps preserved) and the beep's tone.
async fn beep_onset(app: &Arc<App>, src: &Path, t: f64) -> (f64, f64) {
    let args = [
        "-v",
        "error",
        "-copyts",
        "-i",
        src.to_str().unwrap(),
        "-map",
        "0:a:0",
        "-af",
        &format!("atrim=start={t:.4}:duration=1.3"),
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
        app,
        "ffmpeg",
        args,
        None,
        Duration::from_secs(60),
        1 << 20,
        app.cancel.child_token(),
    )
    .await
    .unwrap();
    let samples = pcm
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect::<Vec<_>>();
    assert!(samples.len() > 48_000, "short audio at {t}");
    let mut quiet = 0;
    for (i, sample) in samples.iter().enumerate() {
        if sample.abs() > 0.05 {
            if quiet >= 2_400 {
                let tone = &samples[i..(i + 3_600).min(samples.len())];
                let rises = tone
                    .windows(2)
                    .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
                    .count();
                return (
                    i as f64 / 48_000.0,
                    rises as f64 * 48_000.0 / tone.len() as f64,
                );
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    panic!("no beep onset after {t}");
}

#[tokio::test]
#[ignore = "generated 10-minute B-frame corpus requires FFmpeg libx264, AAC and lavfi; software acceptance"]
async fn real_long_file_seeks_keep_source_timeline_and_av_sync() {
    const FPS: f64 = 24_000.0 / 1_001.0;
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    let media_dir = std::fs::canonicalize(root.path().join("media")).unwrap();
    // Sixteen bars encode the source frame number; audio beeps for 0.1 s at each
    // source second with a tone that steps every minute.
    let bars = (0..16)
        .map(|bit| {
            format!(
                "drawbox=x={}:y=0:w=26:h=240:color=white:t=fill:enable='gt(bitand(n\\,{})\\,0)'",
                bit * 26,
                1 << bit
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    // B-frames make FFmpeg's Matroska/MPEG-TS seeks differ from MP4; 23.976 fps
    // puts keyframes off whole milliseconds. (file, format, seconds, GOP, seeks)
    let fixtures: [(&str, &str, u32, u32, &[f64]); 4] = [
        ("Long.S01E01.mp4", "mp4", 600, 240, &[37.3, 301.7, 580.4]),
        ("Long.S01E02.mkv", "matroska", 600, 240, &[37.3, 301.7]),
        ("Long.S01E03.ts", "mpegts", 600, 240, &[37.3, 301.7]),
        ("Long.S01E04.mp4", "mp4", 180, 1080, &[134.0]),
    ];
    for (name, container, seconds, gop, starts) in fixtures {
        let path = media_dir.join(name);
        let status = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("color=black:size=416x240:rate=24000/1001,{bars}"))
            .args([
                "-f",
                "lavfi",
                "-i",
                "aevalsrc='0.3*lt(mod(t\\,1)\\,0.1)*sin(2*PI*(200+100*floor(t/60))*t)':s=48000",
                "-t",
                &seconds.to_string(),
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-bf",
                "3",
                "-threads",
                "2",
                "-g",
                &gop.to_string(),
                "-keyint_min",
                &gop.to_string(),
                "-sc_threshold",
                "0",
                "-c:a",
                "aac",
                "-b:a",
                "64k",
                "-f",
                container,
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let stored = path.to_string_lossy().into_owned();
        app.db
            .call(move |c| {
                c.execute(
                    "UPDATE media_files SET path=? WHERE item_id=?",
                    rusqlite::params![stored, item],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let gop_seconds = f64::from(gop) / FPS;
        for &requested in starts {
            for mode in ["remux", "transcode"] {
                let (status, session) = request(
                    &router,
                    "POST",
                    &format!("/api/v1/items/{item}/playback"),
                    Some(&token),
                    Some(json!({"Mode":mode,"Start":requested})),
                )
                .await;
                assert_eq!(status, 201, "{session}");
                let start = session["start"].as_f64().unwrap();
                let case = format!("{name} {mode} requested {requested} reported {start}");
                if mode == "remux" {
                    // Copied video begins on a keyframe at or before the request.
                    assert!(
                        start <= requested + 0.002 && start > requested - gop_seconds - 0.2,
                        "{case}"
                    );
                } else {
                    assert!((start - requested).abs() < 0.001, "{case}");
                }
                let id = session["id"].as_str().unwrap().to_owned();
                let stream = app.streams.lock().await.get(&id).unwrap().clone();
                let output = stream.dir.join("v0.m3u8");
                // The client skips any remux lead-in to land on the requested start.
                let skip = requested - start;
                let needed = skip + 25.0;
                tokio::time::timeout(Duration::from_secs(180), async {
                    loop {
                        let listed = tokio::fs::read_to_string(&output)
                            .await
                            .unwrap_or_default()
                            .lines()
                            .filter_map(|l| l.strip_prefix("#EXTINF:"))
                            .filter_map(|l| l.trim_end_matches(',').parse::<f64>().ok())
                            .sum::<f64>();
                        if listed >= needed
                            || stream.finished.load(std::sync::atomic::Ordering::Acquire)
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{case}: output not published"));
                let failure = stream.error.lock().await.clone();
                assert!(failure.is_none(), "{case}: {failure:?}");
                // Decode a closed snapshot of the published segments; FFmpeg treats
                // a growing event playlist as live, which skews seek timestamps.
                let listed = tokio::fs::read_to_string(&output).await.unwrap();
                let mut snapshot = listed
                    .lines()
                    .filter(|l| *l != "#EXT-X-ENDLIST")
                    .map(|l| {
                        if l.starts_with('#') || l.is_empty() {
                            l.to_owned()
                        } else {
                            stream.dir.join(l).to_string_lossy().into_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                snapshot.push_str("\n#EXT-X-ENDLIST\n");
                let output = root.path().join(format!("snapshot-{id}.m3u8"));
                tokio::fs::write(&output, snapshot).await.unwrap();
                let remaining = f64::from(seconds) - start;
                // Stream time zero is the earliest presented timestamp.
                let probe = media::process(
                    &app,
                    "ffprobe",
                    [
                        "-v",
                        "error",
                        "-show_entries",
                        "format=start_time",
                        "-of",
                        "csv=p=0",
                        output.to_str().unwrap(),
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                    None,
                    Duration::from_secs(30),
                    4096,
                    app.cancel.child_token(),
                )
                .await
                .unwrap();
                let zero = String::from_utf8_lossy(&probe)
                    .trim()
                    .parse::<f64>()
                    .unwrap();
                let first = coded_frame(&app, &output, zero).await;
                assert!(
                    (f64::from(first) / FPS - start).abs() <= 2.0 / FPS + 0.05,
                    "{case}: stream begins at frame {first}"
                );
                for at in [skip + 0.2, skip + 3.0, skip + 20.0]
                    .into_iter()
                    .filter(|at| at + 2.0 < remaining)
                {
                    // The picture at stream time `at` is the source at reported start + at.
                    let frame = coded_frame(&app, &output, zero + at).await;
                    let error = f64::from(frame) / FPS - (start + at);
                    assert!(
                        error.abs() <= 2.0 / FPS + 0.05,
                        "{case}: at {at} frame {frame} error {error:+.3}s"
                    );
                    // Audio and video agree: a beep starts on a whole source second.
                    let (onset, tone) = beep_onset(&app, &output, zero + at).await;
                    let beep = f64::from(coded_frame(&app, &output, zero + at + onset).await) / FPS;
                    let drift = beep - beep.round();
                    assert!(
                        drift.abs() <= 2.0 / FPS + 0.01,
                        "{case}: at {at} beep at source {beep:.3} drift {drift:+.3}s"
                    );
                    let expected = 200.0 + 100.0 * (beep.round() / 60.0).floor();
                    assert!(
                        (tone - expected).abs() < expected * 0.04 + 10.0,
                        "{case}: at {at} tone {tone} expected {expected}"
                    );
                }
                assert_eq!(
                    request(
                        &router,
                        "DELETE",
                        &format!("/api/v1/playback/{id}"),
                        Some(&token),
                        None
                    )
                    .await
                    .0,
                    200
                );
                assert!(
                    !app.config.data.join("transcodes").join(&id).exists(),
                    "{case}"
                );
            }
        }
    }
    app.shutdown().await;
}

#[test]
fn keyframe_selection_and_seek_arguments_keep_requested_start() {
    // ffprobe packet CSV: pts_time,flags; non-key, N/A and later keys are ignored.
    let rows = "29.958333,__\n30.021333,K_\nN/A,K_\n35.000000,__\n40.021333,K_\n";
    assert_eq!(media::latest_keyframe(rows, 38.7), Some(30.021333));
    assert_eq!(media::latest_keyframe(rows, 30.021333), Some(30.021333));
    assert_eq!(media::latest_keyframe(rows, 30.0), None);
    assert_eq!(media::latest_keyframe("", 10.0), None);
    let info: media::Info = serde_json::from_value(json!({"streams":[{"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","width":416,"height":240},{"codec_type":"audio","codec_name":"aac","channels":2}],"format":{"format_name":"mpegts","duration":"1200","start_time":"1.400000"}})).unwrap();
    let variants = playback::ladder(416, 240, false);
    let seeks = |b: &Playback| {
        let args = playback::encoding_args(Path::new("media.ts"), b, &info, &variants);
        args.windows(2)
            .filter(|w| w[0] == "-ss")
            .map(|w| w[1].clone())
            .collect::<Vec<_>>()
    };
    // Transcode: decode from the keyframe, then trim to the requested start.
    let transcode = Playback {
        mode: "transcode".into(),
        start: 37.3,
        keyframe: Some(30.022),
        ..Default::default()
    };
    assert_eq!(seeks(&transcode), ["30.022", "7.278"]);
    // Text subtitles render on the decoded (keyframe) timeline.
    let args = playback::encoding_args(
        Path::new("media.ts"),
        &Playback {
            subtitle: Some(0),
            ..transcode.clone()
        },
        &info,
        &variants,
    );
    assert!(
        args.iter().any(|a| a.contains("setpts=PTS+(31.422000)/TB")),
        "{args:?}"
    );
    // Remux seeks onto the keyframe, keeps source timestamps for start
    // measurement and never trims copied video.
    let remux = Playback {
        mode: "remux".into(),
        start: 37.3,
        keyframe: Some(30.022),
        ..Default::default()
    };
    assert_eq!(seeks(&remux), ["30.022"]);
    let args = playback::encoding_args(Path::new("media.ts"), &remux, &info, &variants);
    assert!(args.contains(&"-copyts".to_owned()));
    assert!(args.windows(2).any(|w| w[0] == "-muxdelay" && w[1] == "0"));
    let args = playback::encoding_args(Path::new("media.ts"), &transcode, &info, &variants);
    assert!(!args.contains(&"-copyts".to_owned()));
    // Without a resolved keyframe the original single input seek is kept,
    // rounded up so it can never fall before a keyframe.
    let unresolved = Playback {
        mode: "transcode".into(),
        start: 37.3,
        ..Default::default()
    };
    assert_eq!(seeks(&unresolved), ["37.300"]);
    assert_eq!(
        seeks(&Playback {
            start: 8.3412,
            ..unresolved.clone()
        }),
        ["8.342"]
    );
    // A keyframe after the start is never used.
    assert_eq!(
        seeks(&Playback {
            keyframe: Some(40.0),
            ..unresolved.clone()
        }),
        ["37.300"]
    );
    // Starting at 0 adds no input seek (it fails on unindexed FLV), and
    // hardware decode options still precede the input.
    let at_zero = Playback {
        mode: "transcode".into(),
        ..Default::default()
    };
    assert!(seeks(&at_zero).is_empty());
    let args = playback::hardware_args(
        playback::encoding_args(Path::new("media.ts"), &at_zero, &info, &variants),
        "vaapi",
        &variants,
    );
    let hwaccel = args.iter().position(|a| a == "-hwaccel").unwrap();
    let input = args.iter().position(|a| a == "-i").unwrap();
    assert!(hwaccel < input, "{args:?}");
    // Measured remux starts are accepted only between 0 and the request + 0.1 s.
    assert_eq!(playback::remux_start(37.3, None, Some(30.0304)), 30.03);
    assert_eq!(playback::remux_start(37.3, None, Some(37.39)), 37.39);
    // Wrapped or shifted timestamps fall back to the keyframe or the request.
    assert_eq!(playback::remux_start(20.0, None, Some(42.34)), 20.0);
    assert_eq!(
        playback::remux_start(37.3, Some(30.022), Some(80.0)),
        30.022
    );
    assert_eq!(
        playback::remux_start(37.3, Some(30.022), Some(-1.0)),
        30.022
    );
    assert_eq!(playback::remux_start(37.3, Some(40.0), None), 37.3);
    assert_eq!(playback::remux_start(37.3, None, Some(f64::NAN)), 37.3);
    // Clients cannot supply the server-resolved keyframe.
    assert!(
        serde_json::from_value::<Playback>(json!({"Mode":"remux","Start":5,"Keyframe":0})).is_err()
    );
}

#[tokio::test]
#[ignore = "requires FFmpeg libx264 and AAC; software acceptance"]
async fn abandoned_remux_start_cancels_the_session() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    let path = std::fs::canonicalize(root.path().join("media/Example.S01E01.mp4")).unwrap();
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "120",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "240",
            "-c:a",
            "aac",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    // Drop the start request at increasing delays. Whenever a session already
    // existed, it must be cancelled promptly rather than linger until the idle
    // reaper (5 s here), holding a stream slot.
    let mut abandoned_mid_wait = 0;
    for delay in (0..60).map(|step| Duration::from_millis(step * 5)) {
        let body = json!({"Mode":"remux","Start":37.3});
        let started = tokio::time::timeout(
            delay,
            request(
                &router,
                "POST",
                &format!("/api/v1/items/{item}/playback"),
                Some(&token),
                Some(body),
            ),
        )
        .await;
        match started {
            Ok((status, session)) => {
                assert_eq!(status, 201, "{session}");
                let id = session["id"].as_str().unwrap();
                request(
                    &router,
                    "DELETE",
                    &format!("/api/v1/playback/{id}"),
                    Some(&token),
                    None,
                )
                .await;
            }
            Err(_) => {
                if !app.streams.lock().await.is_empty() {
                    abandoned_mid_wait += 1;
                }
            }
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !app.streams.lock().await.is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("session left running after a {delay:?} disconnect"));
    }
    assert!(
        abandoned_mid_wait > 0,
        "no request was dropped while waiting"
    );
    assert!(
        std::fs::read_dir(app.config.data.join("transcodes"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 0
    );
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "generated surround corpus requires FFmpeg AC-3/E-AC-3/DTS/TrueHD/FLAC/PCM/Opus/AAC encoders and libx264; software acceptance"]
async fn real_surround_codec_corpus_downmixes_with_correct_channel_routing() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    let media_dir = std::fs::canonicalize(root.path().join("media")).unwrap();
    // Each source channel carries its own tone; a stereo downmix must keep left
    // channels left, right channels right, centre in both and drop the LFE.
    let tone = |c: &str| match c {
        "FL" => 400.0,
        "FR" => 600.0,
        "FC" => 800.0,
        "LFE" => 60.0,
        "BL" => 1000.0,
        "BR" => 1200.0,
        "SL" => 1400.0,
        _ => 1600.0,
    };
    let layouts = |layout: &str| match layout {
        "5.1" => vec!["FL", "FR", "FC", "LFE", "BL", "BR"],
        "5.1(side)" => vec!["FL", "FR", "FC", "LFE", "SL", "SR"],
        _ => vec!["FL", "FR", "FC", "LFE", "BL", "BR", "SL", "SR"],
    };
    let cases: [(&str, &str, &str, &[&str]); 8] = [
        ("ac3", "5.1", "ac3", &["-c:a", "ac3", "-b:a", "448k"]),
        (
            "eac3",
            "5.1(side)",
            "eac3",
            &["-c:a", "eac3", "-b:a", "640k"],
        ),
        (
            "dts",
            "5.1",
            "dts",
            &["-c:a", "dca", "-strict", "-2", "-b:a", "1509k"],
        ),
        (
            "truehd",
            "5.1",
            "truehd",
            &["-c:a", "truehd", "-strict", "-2"],
        ),
        ("flac", "7.1", "flac", &["-c:a", "flac"]),
        ("pcm", "7.1", "pcm_s24le", &["-c:a", "pcm_s24le"]),
        ("opus", "7.1", "opus", &["-c:a", "libopus", "-b:a", "448k"]),
        ("aac", "7.1", "aac", &["-c:a", "aac", "-b:a", "512k"]),
    ];
    for (name, layout, codec, encode) in cases {
        let channels = layouts(layout);
        let expr = channels
            .iter()
            .map(|c| format!("0.1*sin(2*PI*{}*t)", tone(c)))
            .collect::<Vec<_>>()
            .join("|");
        let path = media_dir.join(format!("Surround.{name}.mkv"));
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=12",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(format!("aevalsrc={expr}:s=48000:c={layout}"))
            .args([
                "-t",
                "3",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-threads",
                "2",
                "-g",
                "12",
            ])
            .args(encode)
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "{name}: fixture encode failed");
        let stored = path.to_string_lossy().into_owned();
        app.db
            .call(move |c| {
                c.execute(
                    "UPDATE media_files SET path=? WHERE item_id=?",
                    rusqlite::params![stored, item],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (status, probe) = request(
            &router,
            "GET",
            &format!("/api/v1/items/{item}/probe"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{probe}");
        let info: media::Info = serde_json::from_value(probe).unwrap();
        let source = info.tracks("audio")[0];
        assert_eq!(source.codec_name, codec, "{name}");
        assert_eq!(source.channels as usize, channels.len(), "{name}");
        // Stereo and mono AAC-only browsers; DTS also proves the mono fold-down.
        let limits: &[u32] = if name == "dts" { &[2, 1] } else { &[2] };
        for &limit in limits {
            let (status, session) = request(
                &router,
                "POST",
                &format!("/api/v1/items/{item}/playback"),
                Some(&token),
                Some(json!({"Mode":"auto","Capabilities":{"containers":["mov","mp4","m4a","3gp","3g2","mj2"],"video_codecs":["h264"],"audio_codecs":["aac"],"hdr":false,"max_audio_channels":limit}})),
            )
            .await;
            assert_eq!(status, 201, "{name}: {session}");
            assert_eq!(session["mode"], "remux", "{name}: {session}");
            let id = session["id"].as_str().unwrap().to_owned();
            let stream = app.streams.lock().await.get(&id).unwrap().clone();
            tokio::time::timeout(Duration::from_secs(60), async {
                while !stream.finished.load(std::sync::atomic::Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            let failure = stream.error.lock().await.clone();
            assert!(failure.is_none(), "{name}: {failure:?}");
            let output = stream.dir.join("v0.m3u8");
            let converted = media::inspect(&app, &output).await.unwrap();
            assert_eq!(converted.tracks("audio")[0].codec_name, "aac");
            assert_eq!(converted.tracks("audio")[0].channels, limit, "{name}");
            let args = [
                "-v",
                "error",
                "-i",
                output.to_str().unwrap(),
                "-map",
                "0:a:0",
                "-af",
                "atrim=start=1:duration=0.5",
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
                Duration::from_secs(30),
                1 << 20,
                app.cancel.child_token(),
            )
            .await
            .unwrap();
            let samples: Vec<f64> = pcm
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f64::from(f32::from_le_bytes(*b)))
                .collect();
            let width = limit as usize;
            assert!(samples.len() >= 23_000 * width, "{name}: short audio");
            assert!(
                samples.iter().all(|s| s.is_finite() && s.abs() < 0.98),
                "{name}: nonfinite or clipped audio"
            );
            let amplitude = |channel: usize, hz: f64| {
                let signal: Vec<_> = samples.iter().skip(channel).step_by(width).collect();
                let (mut re, mut im) = (0.0, 0.0);
                for (n, sample) in signal.iter().enumerate() {
                    let phase = std::f64::consts::TAU * hz * n as f64 / 48_000.0;
                    re += *sample * phase.cos();
                    im += *sample * phase.sin();
                }
                2.0 * f64::hypot(re, im) / signal.len() as f64
            };
            for channel in &channels {
                let hz = tone(channel);
                if width == 1 {
                    let level = amplitude(0, hz);
                    if *channel == "LFE" {
                        assert!(level < 0.01, "{name} mono: LFE {level:.3}");
                    } else {
                        assert!(level > 0.01, "{name} mono: {channel} missing {level:.3}");
                    }
                    continue;
                }
                let (left, right) = (amplitude(0, hz), amplitude(1, hz));
                let detail = format!("{name}: {channel} L={left:.3} R={right:.3}");
                match *channel {
                    "FL" | "BL" | "SL" => assert!(left > 0.02 && right < 0.005, "{detail}"),
                    "FR" | "BR" | "SR" => assert!(right > 0.02 && left < 0.005, "{detail}"),
                    "FC" => assert!(
                        left > 0.02 && right > 0.02 && (left - right).abs() < 0.2 * left.max(right),
                        "{detail}"
                    ),
                    _ => assert!(left < 0.01 && right < 0.01, "{detail}"),
                }
            }
            assert_eq!(
                request(
                    &router,
                    "DELETE",
                    &format!("/api/v1/playback/{id}"),
                    Some(&token),
                    None
                )
                .await
                .0,
                200
            );
        }
    }
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "generated legacy container corpus requires FFmpeg MPEG-1/2, WMV2, FLV, Theora, AV1 (SVT), HEVC and their audio encoders; software acceptance"]
async fn real_legacy_container_corpus_converts_with_exact_timeline_and_sync() {
    const FPS: f64 = 24.0;
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let item = add_media(&app, root.path()).await;
    let media_dir = std::fs::canonicalize(root.path().join("media")).unwrap();
    let bars = (0..16)
        .map(|bit| {
            format!(
                "drawbox=x={}:y=0:w=26:h=240:color=white:t=fill:enable='gt(bitand(n\\,{})\\,0)'",
                bit * 26,
                1 << bit
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    // (file, muxer, video encoder args, audio encoder args, expected mode)
    type Case<'a> = (&'a str, &'a str, &'a [&'a str], &'a [&'a str], &'a str);
    let cases: [Case; 9] = [
        (
            "Legacy.mpeg1.mpg",
            "mpeg",
            &["-c:v", "mpeg1video", "-b:v", "2M"],
            &["-c:a", "mp2"],
            "transcode",
        ),
        (
            "Legacy.mpeg2.mpg",
            "vob",
            &["-c:v", "mpeg2video", "-b:v", "3M", "-g", "60"],
            &["-c:a", "ac3"],
            "transcode",
        ),
        (
            "Legacy.mpeg2.ts",
            "mpegts",
            &["-c:v", "mpeg2video", "-b:v", "3M", "-g", "60"],
            &["-c:a", "ac3"],
            "transcode",
        ),
        (
            "Legacy.wmv2.wmv",
            "asf",
            &["-c:v", "wmv2", "-b:v", "2M"],
            &["-c:a", "wmav2"],
            "transcode",
        ),
        (
            "Legacy.sorenson.flv",
            "flv",
            &["-c:v", "flv", "-b:v", "2M"],
            &["-c:a", "libmp3lame", "-ar", "44100"],
            "transcode",
        ),
        (
            "Legacy.h264.flv",
            "flv",
            &["-c:v", "libx264", "-preset", "ultrafast", "-g", "24"],
            &["-c:a", "aac"],
            "remux",
        ),
        (
            "Legacy.theora.ogv",
            "ogg",
            &["-c:v", "libtheora", "-q:v", "7"],
            &["-c:a", "libvorbis"],
            "transcode",
        ),
        (
            "Legacy.av1.mkv",
            "matroska",
            &["-c:v", "libsvtav1", "-preset", "12", "-g", "48"],
            &["-c:a", "libopus"],
            "transcode",
        ),
        (
            "Legacy.hevc.mp4",
            "mp4",
            &[
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-tag:v",
                "hvc1",
                "-x265-params",
                "log-level=error",
            ],
            &["-c:a", "aac"],
            "transcode",
        ),
    ];
    for (name, muxer, video, audio, expected_mode) in cases {
        let path = media_dir.join(name);
        let status = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!(
                "color=black:size=416x240:rate=24,{bars},format=yuv420p"
            ))
            .args([
                "-f",
                "lavfi",
                "-i",
                "aevalsrc='0.3*lt(mod(t\\,1)\\,0.1)*sin(2*PI*800*t)':s=48000:c=stereo",
                "-t",
                "10",
                "-threads",
                "2",
            ])
            .args(video)
            .args(audio)
            .args(["-f", muxer])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "{name}: fixture encode failed");
        let stored = path.to_string_lossy().into_owned();
        app.db
            .call(move |c| {
                c.execute(
                    "UPDATE media_files SET path=? WHERE item_id=?",
                    rusqlite::params![stored, item],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        // Mid-GOP and just past a keyframe (MPEG-2 fixtures use a 2.5 s GOP).
        for requested in [0.0, 4.3, 5.02] {
            let (status, session) = request(
                &router,
                "POST",
                &format!("/api/v1/items/{item}/playback"),
                Some(&token),
                Some(json!({"Mode":"auto","Start":requested,"Capabilities":{"containers":["mov","mp4","m4a","3gp","3g2","mj2","hls"],"video_codecs":["h264"],"audio_codecs":["aac"],"hdr":false,"max_audio_channels":2}})),
            )
            .await;
            assert_eq!(status, 201, "{name}: {session}");
            assert_eq!(session["mode"], expected_mode, "{name}: {session}");
            let start = session["start"].as_f64().unwrap();
            let case = format!("{name} requested {requested} reported {start}");
            assert!(
                start <= requested + 0.002 && start > requested - 2.2,
                "{case}"
            );
            let id = session["id"].as_str().unwrap().to_owned();
            let stream = app.streams.lock().await.get(&id).unwrap().clone();
            tokio::time::timeout(Duration::from_secs(120), async {
                while !stream.finished.load(std::sync::atomic::Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("{case}: conversion did not finish"));
            let failure = stream.error.lock().await.clone();
            assert!(failure.is_none(), "{case}: {failure:?}");
            let output = stream.dir.join("v0.m3u8");
            let converted = media::inspect(&app, &output).await.unwrap();
            assert_eq!(converted.video().unwrap().codec_name, "h264", "{case}");
            assert_eq!(converted.tracks("audio")[0].codec_name, "aac", "{case}");
            let probe = media::process(
                &app,
                "ffprobe",
                [
                    "-v",
                    "error",
                    "-show_entries",
                    "format=start_time",
                    "-of",
                    "csv=p=0",
                    output.to_str().unwrap(),
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                None,
                Duration::from_secs(30),
                4096,
                app.cancel.child_token(),
            )
            .await
            .unwrap();
            let zero = String::from_utf8_lossy(&probe)
                .trim()
                .parse::<f64>()
                .unwrap();
            let first = coded_frame(&app, &output, zero).await;
            assert!(
                (f64::from(first) / FPS - start).abs() <= 2.0 / FPS + 0.05,
                "{case}: stream begins at frame {first}"
            );
            if expected_mode == "transcode" {
                // Converted audio and video begin together (no dropped lead-in).
                let starts = media::process(
                    &app,
                    "ffprobe",
                    [
                        "-v",
                        "error",
                        "-show_entries",
                        "stream=start_time",
                        "-of",
                        "csv=p=0",
                        stream.dir.join("v0-0.ts").to_str().unwrap(),
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                    None,
                    Duration::from_secs(30),
                    4096,
                    app.cancel.child_token(),
                )
                .await
                .unwrap();
                let starts = String::from_utf8_lossy(&starts)
                    .lines()
                    .filter_map(|l| l.trim().trim_end_matches(',').parse::<f64>().ok())
                    .collect::<Vec<_>>();
                // Program and stream sections both list each stream's start.
                let spread = starts.iter().copied().fold(f64::MIN, f64::max)
                    - starts.iter().copied().fold(f64::MAX, f64::min);
                assert!(
                    starts.len() >= 2 && spread < 0.1,
                    "{case}: stream starts {starts:?}"
                );
            }
            let skip = requested - start;
            for at in [skip + 0.4, skip + 2.4] {
                let frame = coded_frame(&app, &output, zero + at).await;
                let error = f64::from(frame) / FPS - (start + at);
                assert!(
                    error.abs() <= 2.0 / FPS + 0.05,
                    "{case}: at {at} frame {frame} error {error:+.3}s"
                );
                let (onset, tone) = beep_onset(&app, &output, zero + at).await;
                let beep = f64::from(coded_frame(&app, &output, zero + at + onset).await) / FPS;
                let drift = beep - beep.round();
                assert!(
                    drift.abs() <= 2.0 / FPS + 0.01,
                    "{case}: at {at} beep at source {beep:.3} drift {drift:+.3}s"
                );
                assert!((tone - 800.0).abs() < 50.0, "{case}: tone {tone}");
            }
            assert_eq!(
                request(
                    &router,
                    "DELETE",
                    &format!("/api/v1/playback/{id}"),
                    Some(&token),
                    None
                )
                .await
                .0,
                200
            );
        }
    }
    app.shutdown().await;
}

#[tokio::test]
async fn library_roots_cannot_nest_in_either_direction() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let media = std::fs::canonicalize(root.path()).unwrap().join("media");
    for dir in ["shows/kids", "films"] {
        std::fs::create_dir_all(media.join(dir)).unwrap();
    }
    let add = |path: std::path::PathBuf| {
        let router = router.clone();
        let token = token.clone();
        async move {
            request(
                &router,
                "POST",
                "/api/v1/libraries",
                Some(&token),
                Some(json!({"Name":"lib","Kind":"tv","Path":path})),
            )
            .await
            .0
        }
    };
    assert_eq!(add(media.join("shows")).await, 201);
    // Child, parent and the same root all overlap the existing library.
    assert_eq!(add(media.join("shows/kids")).await, 409);
    assert_eq!(add(media.clone()).await, 409);
    assert_eq!(add(media.join("shows")).await, 409);
    // A sibling, including one sharing a name prefix, does not.
    assert_eq!(add(media.join("films")).await, 201);
    std::fs::create_dir_all(media.join("shows-extra")).unwrap();
    assert_eq!(add(media.join("shows-extra")).await, 201);
    app.shutdown().await;
}

#[tokio::test]
async fn configured_setup_is_rejected_before_body_validation() {
    let root = TempDir::new().unwrap();
    let (app, router, _) = setup(&root).await;
    // An invalid password would be 400 if setup still reached account creation.
    for body in [
        json!({"Name":"second","Password":"short"}),
        json!({"Name":"second","Password":"test-password-123"}),
    ] {
        assert_eq!(
            request(&router, "POST", "/api/v1/setup", None, Some(body))
                .await
                .0,
            409
        );
    }
    app.shutdown().await;
}

#[tokio::test]
async fn list_names_are_limited_by_characters_not_bytes() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    for (count, status) in [(200, 201), (201, 400)] {
        assert_eq!(
            request(
                &router,
                "POST",
                "/api/v1/lists",
                Some(&token),
                Some(json!({"Kind":"playlist","Name":"é".repeat(count),"ItemIDs":[]}))
            )
            .await
            .0,
            status,
            "{count} two-byte characters"
        );
    }
    app.shutdown().await;
}

#[tokio::test]
async fn music_search_folds_unicode_case() {
    let root = TempDir::new().unwrap();
    let (app, _, _) = setup(&root).await;
    let (found, missing, null): (bool, bool, Option<String>) = app
        .db
        .call(|c| {
            Ok(c.query_row(
                "SELECT instr(ovl_lower('Ébène ÑANDÚ Straße'),ovl_lower('ébène ñandú')) > 0,
                        instr(ovl_lower('Ébène'),ovl_lower('ebene')) > 0,
                        ovl_lower(NULL)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    assert!(found, "non-ASCII letters must match across case");
    assert!(!missing, "case folding must not strip accents");
    assert_eq!(null, None);
    app.shutdown().await;
}

#[tokio::test]
async fn trickled_request_body_hits_the_total_deadline() {
    let root = TempDir::new().unwrap();
    let app = App::open(config(root.path())).unwrap();
    let router = app.router();
    // One byte per second never trips an idle timeout; the total deadline must.
    let trickle = futures_util::stream::unfold((), |()| async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Some((
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b" ")),
            (),
        ))
    });
    let started = std::time::Instant::now();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header("X-OVL-Request", "1")
                .header("Content-Type", "application/json")
                .body(Body::from_stream(trickle))
                .unwrap(),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert!(
        elapsed >= ovl_server::api::BODY_DEADLINE
            && elapsed < ovl_server::api::BODY_DEADLINE + Duration::from_secs(5),
        "{elapsed:?}"
    );
    assert_eq!(
        app.db
            .call(|c| Ok(c.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))?))
            .await
            .unwrap(),
        0
    );
    app.shutdown().await;
}

fn ffmpeg(args: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg is required");
    assert!(status.success(), "ffmpeg {args:?}");
}

/// Registers one file as the only item of a new library and returns its id.
async fn add_file(app: &Arc<App>, dir: &Path, file: &Path, music: bool) -> i64 {
    let (dir, file) = (
        dir.to_string_lossy().into_owned(),
        file.to_string_lossy().into_owned(),
    );
    app.db
        .call(move |c| {
            c.execute(
                "INSERT INTO libraries(name,path,kind) VALUES('Library',?,'movie')",
                [dir],
            )?;
            let lib = c.last_insert_rowid();
            if music {
                c.execute(
                    "INSERT INTO ovl_music_libraries(library_id) VALUES(?)",
                    [lib],
                )?;
            }
            c.execute(
                "INSERT INTO items(title,series,season,episode) VALUES('Track','',0,0)",
                [],
            )?;
            let item = c.last_insert_rowid();
            c.execute(
                "INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,?,?,0,1)",
                rusqlite::params![item, lib, file],
            )?;
            Ok(item)
        })
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires FFmpeg/FFprobe; software acceptance"]
async fn real_id3_uslt_lyrics_are_read_without_a_sidecar() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let dir = std::fs::canonicalize(root.path()).unwrap().join("music");
    std::fs::create_dir(&dir).unwrap();
    let bare = root.path().join("bare.mp3");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1",
        "-c:a",
        "libmp3lame",
        "-id3v2_version",
        "0",
        bare.to_str().unwrap(),
    ]);
    // ID3v2.3 with one USLT frame: Latin-1, language "eng", descriptor "Verse".
    let mut frame = vec![0u8];
    frame.extend(b"engVerse\0Embedded line one\nEmbedded line two");
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    let size = 10 + frame.len();
    tag.extend([
        (size >> 21) as u8 & 0x7f,
        (size >> 14) as u8 & 0x7f,
        (size >> 7) as u8 & 0x7f,
        size as u8 & 0x7f,
    ]);
    tag.extend(b"USLT");
    tag.extend((frame.len() as u32).to_be_bytes());
    tag.extend([0, 0]);
    tag.extend(frame);
    tag.extend(std::fs::read(&bare).unwrap());
    let track = dir.join("Track.mp3");
    std::fs::write(&track, tag).unwrap();
    let id = add_file(&app, &dir, &track, true).await;
    let (status, body) = request(
        &router,
        "GET",
        &format!("/api/v1/items/{id}/lyrics"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({"lines":[{"time_ms":null,"text":"Embedded line one"},{"time_ms":null,"text":"Embedded line two"}]})
    );
    assert!(!dir.join("Track.lrc").exists() && !dir.join("Track.txt").exists());
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "requires FFmpeg/FFprobe; software acceptance"]
async fn real_playlist_disguised_as_media_cannot_read_outside_the_library() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let canonical = std::fs::canonicalize(root.path()).unwrap();
    let (dir, outside) = (canonical.join("library"), canonical.join("outside"));
    std::fs::create_dir(&dir).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let secret = outside.join("secret.ts");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x64:rate=10",
        "-t",
        "2",
        "-c:v",
        "libx264",
        "-f",
        "mpegts",
        secret.to_str().unwrap(),
    ]);
    // Unconfined FFprobe detects this as HLS and opens the outside segment.
    let evil = dir.join("Evil.mkv");
    std::fs::write(
        &evil,
        format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\n{}\n#EXT-X-ENDLIST\n",
            secret.display()
        ),
    )
    .unwrap();
    let id = add_file(&app, &dir, &evil, false).await;
    let (status, body) = request(
        &router,
        "GET",
        &format!("/api/v1/items/{id}/probe"),
        Some(&token),
        None,
    )
    .await;
    assert_ne!(status, 200, "probe followed the playlist: {body}");
    for mode in ["remux", "transcode"] {
        let (status, body) = request(
            &router,
            "POST",
            &format!("/api/v1/items/{id}/playback"),
            Some(&token),
            Some(json!({"Mode":mode})),
        )
        .await;
        assert!(
            !status.is_success(),
            "{mode} started from the playlist: {body}"
        );
    }
    app.shutdown().await;
}

#[test]
fn every_encode_input_is_rebound_to_its_own_inherited_handle() {
    let source = Path::new("/library/Movie.mkv");
    let info = |subtitle: &str| -> media::Info {
        serde_json::from_value(json!({"streams":[
            {"index":0,"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","width":1920,"height":1080},
            {"index":1,"codec_type":"audio","codec_name":"aac","channels":2},
            {"index":2,"codec_type":"subtitle","codec_name":subtitle}],
            "format":{"format_name":"matroska","duration":"600","start_time":"0"}}))
        .unwrap()
    };
    let cases = [
        ("remux", None, false, "subrip", 1),
        ("transcode", None, false, "subrip", 1),
        ("transcode", Some(0), false, "hdmv_pgs_subtitle", 2),
        ("transcode", Some(0), true, "subrip", 0),
    ];
    for (mode, subtitle, adaptive, codec, inputs) in cases {
        let info = info(codec);
        let b = Playback {
            mode: mode.into(),
            subtitle,
            adaptive,
            start: 30.0,
            ..Default::default()
        };
        let variants = playback::ladder(1920, 1080, adaptive && mode == "transcode");
        let args = playback::encoding_args(source, &b, &info, &variants);
        let (bound, handles) = playback::bind_input_args(args, source).unwrap();
        let source = source.to_string_lossy();
        assert!(
            !bound.iter().any(|a| a.contains(source.as_ref())),
            "{bound:?}"
        );
        assert!(
            !bound.iter().any(|a| a.contains("filename=source")),
            "{bound:?}"
        );
        let named: Vec<String> = bound
            .iter()
            .flat_map(|a| a.split(['=', ':', ' ', ';', ',', '[', ']']))
            .filter(|p| p.starts_with("/dev/fd/"))
            .map(str::to_owned)
            .collect();
        let expected: Vec<String> = (3..3 + handles).map(|fd| format!("/dev/fd/{fd}")).collect();
        let mut seen = named.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), named.len(), "a handle is shared: {named:?}");
        assert_eq!(seen, expected, "{mode} {codec} adaptive={adaptive}");
        let rebound_inputs = bound.windows(2).filter(|w| w[0] == "-i").count();
        if inputs > 0 {
            assert_eq!(rebound_inputs, inputs, "{bound:?}");
        } else {
            // One input plus one text-subtitle filter per rendition.
            assert_eq!(rebound_inputs, 1);
            assert_eq!(handles, 1 + variants.len(), "{bound:?}");
        }
        for w in bound.windows(2).filter(|w| w[0] == "-i") {
            assert!(w[1].starts_with("/dev/fd/"), "{w:?}");
        }
        for (i, w) in bound.iter().enumerate().filter(|(_, a)| *a == "-i") {
            assert_eq!(
                bound[i - 4..i],
                [
                    "-protocol_whitelist".to_string(),
                    "file".into(),
                    "-format_whitelist".into(),
                    "mov,matroska,avi,mpegts,mpeg,mpegvideo,flv,asf,ogg,mp3,flac,wav,aac".into()
                ],
                "{w}"
            );
        }
    }
    // Fails closed on any input other than the authorized media.
    let foreign = vec!["-i".into(), "/elsewhere.mkv".into()];
    assert!(playback::bind_input_args(foreign, source).is_err());
}

async fn scan_and_wait(app: &Arc<App>) {
    ovl_server::scan::start(app.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while app.scans.available_permits() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn nfo_sidecars_are_read_only_from_regular_files_inside_the_root() {
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    add_media(&app, root.path()).await;
    let media = root.path().join("media");
    let outside = root.path().join("outside.nfo");
    std::fs::write(
        &outside,
        "<episodedetails><title>Leaked title</title></episodedetails>",
    )
    .unwrap();
    std::fs::write(media.join("Linked.S01E02.mkv"), b"linked").unwrap();
    std::os::unix::fs::symlink(&outside, media.join("Linked.S01E02.nfo")).unwrap();
    // A FIFO must neither block the scan nor be read as metadata.
    std::fs::write(media.join("Fifo.S01E03.mkv"), b"fifo").unwrap();
    let fifo = std::ffi::CString::new(media.join("Fifo.S01E03.nfo").to_string_lossy().into_owned())
        .unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    std::fs::write(media.join("Plain.S01E04.mkv"), b"plain").unwrap();
    std::fs::write(
        media.join("Plain.S01E04.nfo"),
        "<episodedetails><title>Inside title</title></episodedetails>",
    )
    .unwrap();
    scan_and_wait(&app).await;
    let (_, items) = request(&router, "GET", "/api/v1/items", Some(&token), None).await;
    let titles: Vec<_> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(titles.len(), 4, "{titles:?}");
    assert!(titles.iter().any(|t| t == "Inside title"), "{titles:?}");
    assert!(!titles.iter().any(|t| t == "Leaked title"), "{titles:?}");
    app.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn failed_music_probes_do_not_starve_later_tracks() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let (app, router, token) = setup(&root).await;
    let dir = std::fs::canonicalize(root.path()).unwrap().join("music");
    std::fs::create_dir(&dir).unwrap();
    // More unreadable tracks than one scan's probe budget of 256.
    for n in 0..300 {
        let track = dir.join(format!("{n:03}.mp3"));
        std::fs::write(&track, b"unreadable").unwrap();
        std::fs::set_permissions(&track, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    if std::fs::File::open(dir.join("000.mp3")).is_ok() {
        eprintln!("skipped: permissions do not block this user (root?)");
        return;
    }
    let (status, library) = request(
        &router,
        "POST",
        "/api/v1/libraries",
        Some(&token),
        Some(json!({"Name":"Music","Path":dir,"Kind":"music"})),
    )
    .await;
    assert_eq!(status, 201, "{library}");
    let failures = |app: Arc<App>| async move {
        app.db
            .call(|c| {
                Ok(
                    c.query_row("SELECT count(*) FROM ovl_music_probe_failures", [], |r| {
                        r.get::<_, i64>(0)
                    })?,
                )
            })
            .await
            .unwrap()
    };
    scan_and_wait(&app).await;
    assert_eq!(failures(app.clone()).await, 256);
    // The next scan skips recorded failures and reaches the remaining tracks.
    scan_and_wait(&app).await;
    assert_eq!(failures(app.clone()).await, 300);
    // A changed file is probed again.
    let first = dir.join("000.mp3");
    std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(&first, b"changed and still not audio").unwrap();
    std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o000)).unwrap();
    let stale = |app: Arc<App>| async move {
        app.db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM ovl_music_probe_failures p JOIN media_files f ON f.id=p.file_id
                      WHERE p.mtime<>f.mtime OR p.size<>f.size",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await
            .unwrap()
    };
    scan_and_wait(&app).await;
    assert_eq!(stale(app.clone()).await, 0);
    // Unreadable tracks are skipped for the long delay.
    let soonest = app
        .db
        .call(|c| {
            Ok(c.query_row(
                "SELECT min(retry_at) FROM ovl_music_probe_failures",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!(soonest > now + 23 * 3600, "{soonest} vs {now}");
    for n in 0..300 {
        let _ = std::fs::set_permissions(
            dir.join(format!("{n:03}.mp3")),
            std::fs::Permissions::from_mode(0o644),
        );
    }
    app.shutdown().await;
}
