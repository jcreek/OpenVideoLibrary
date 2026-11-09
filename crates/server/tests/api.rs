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
    let dir = dir.to_string_lossy().into_owned();
    let path = path.to_string_lossy().into_owned();
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
    let mut info:media::Info=serde_json::from_value(json!({"streams":[{"codec_type":"video","codec_name":"hevc","color_transfer":"smpte2084","width":3840,"height":2160},{"codec_type":"audio","codec_name":"truehd"}],"format":{"format_name":"matroska","duration":"60"}})).unwrap();
    let mut request = Playback {
        mode: "auto".into(),
        capabilities: Some(ClientCapabilities {
            containers: vec!["matroska".into()],
            video_codecs: vec!["hevc".into()],
            audio_codecs: vec!["truehd".into()],
            hdr: true,
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
            "smpte2084",
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
    assert_eq!(probe["streams"][0]["color_transfer"], "smpte2084");
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
    assert!(App::open(cfg.clone()).is_err());
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
