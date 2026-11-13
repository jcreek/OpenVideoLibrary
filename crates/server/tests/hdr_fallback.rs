//! Simulated hardware faults exercise real server selection/publication/fallback.
//! Software FFmpeg still encodes and decodes the output; no GPU pass is implied.
#![cfg(unix)]
use ovl_server::{App, Config, media, playback};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn response(request: reqwest::RequestBuilder, expected: u16) -> Value {
    let reply = request.send().await.unwrap();
    let status = reply.status();
    let bytes = reply.bytes().await.unwrap();
    assert_eq!(
        status.as_u16(),
        expected,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}
#[tokio::test]
#[ignore = "requires software FFmpeg/libx265; simulated HW faults, not physical GPU acceptance"]
async fn hdr_opt_in_cpu_override_and_publication_boundary_fallback() {
    let root = TempDir::new().unwrap();
    let fixture_app = App::open(Config {
        data: root.path().join("fixture-data"),
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
    let media_dir = root.path().join("media");
    std::fs::create_dir(&media_dir).unwrap();
    let source = media_dir.join("HDR.mp4");
    let mut args:Vec<String>=["-nostdin","-v","error","-f","lavfi","-i","testsrc2=size=320x180:rate=12","-t","1","-c:v","libx265","-preset","ultrafast","-threads","2","-x265-params","pools=1:frame-threads=1:log-level=error:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400","-pix_fmt","yuv420p10le","-color_primaries","bt2020","-color_trc","smpte2084","-colorspace","bt2020nc","-color_range","tv"].into_iter().map(str::to_owned).collect();
    args.push(source.to_string_lossy().into_owned());
    media::process(
        &fixture_app,
        "ffmpeg",
        args,
        None,
        Duration::from_secs(30),
        1024,
        fixture_app.cancel.child_token(),
    )
    .await
    .unwrap();
    let info = media::inspect_with_hdr_metadata(&fixture_app, &source, true)
        .await
        .unwrap();
    let b = playback::Playback {
        mode: "transcode".into(),
        ..Default::default()
    };
    assert!(playback::vaapi_hdr_eligible(&info, &b), "{info:?}");
    let prepared = root.path().join("prepared");
    std::fs::create_dir(&prepared).unwrap();
    media::process(
        &fixture_app,
        "ffmpeg",
        playback::encoding_args(
            &source,
            &b,
            &info,
            &[playback::Rendition {
                width: 320,
                height: 180,
                kbps: 1000,
            }],
        ),
        Some(prepared.clone()),
        Duration::from_secs(30),
        1024,
        fixture_app.cancel.child_token(),
    )
    .await
    .unwrap();
    let original = std::fs::read(&source).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o444)).unwrap();
    let tools = root.path().join("tools");
    std::fs::create_dir(&tools).unwrap();
    let real_ffmpeg = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("ffmpeg"))
        .find(|p| p.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let wrapper = tools.join("ffmpeg");
    std::fs::write(
        &wrapper,
        r##"#!/bin/sh
# Emulate the small driver-validation picture separately from session faults.
case "$PWD" in
  */hdr-probe)
    case "$*" in
      *pipe:1*)
        if [ "$OVL_TEST_FAULT" = black ]; then
          printf 'probe-rejected\n' >> "$OVL_TEST_MARKER"
          cat "$OVL_TEST_BLACK_RGB"
        else cat "$OVL_TEST_VALID_RGB"; fi ;;
      *tonemap_vaapi*)
        if [ "$OVL_TEST_FAULT" = cancel ] || [ "$OVL_TEST_FAULT" = shutdown ]; then
          printf '%s' "$$" > "$OVL_TEST_PID"
          exec sleep 60
        fi
        cp "$OVL_TEST_PREPARED"/* . ;;
      *) exec "$OVL_TEST_FFMPEG" "$@" ;;
    esac
    exit $? ;;
esac
if [ "$OVL_TEST_FAULT" = quota ]; then
  printf '%s' "$$" > "$OVL_TEST_PID"
  exec sleep 60
fi
case "$*" in
  *tonemap_vaapi*)
    printf 'hardware\n' >> "$OVL_TEST_MARKER"
    if [ "$OVL_TEST_FAULT" = before ]; then exit 1; fi
    cp "$OVL_TEST_PREPARED"/* . || exit 2
    count=0
    while [ ! -f index.m3u8 ] && [ "$count" -lt 250 ]; do
      sleep 0.02
      count=$((count+1))
    done
    test -f index.m3u8 || exit 3
    printf 'published-failure\n' >> "$OVL_TEST_MARKER"
    exit 1 ;;
  *h264_vaapi*|*h264_qsv*|*h264_nvenc*) exit 0 ;;
  *) printf 'cpu\n' >> "$OVL_TEST_MARKER"; exec "$OVL_TEST_FFMPEG" "$@" ;;
esac
"##,
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(tools).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let valid_rgb = root.path().join("valid.rgb");
    let black_rgb = root.path().join("black.rgb");
    let mut rgb = vec![];
    for _ in 0..128 {
        for x in 0..128 {
            rgb.extend([[0u8; 3], [41; 3], [130; 3], [244; 3]][x / 32]);
        }
    }
    std::fs::write(&valid_rgb, &rgb).unwrap();
    std::fs::write(&black_rgb, vec![0u8; rgb.len()]).unwrap();
    for scenario in [
        "cpu", "before", "after", "black", "cancel", "shutdown", "quota",
    ] {
        let data = root.path().join(format!("data-{scenario}"));
        let marker = root.path().join(format!("{scenario}.marker"));
        let child_pid = root.path().join(format!("{scenario}.pid"));
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let log = std::fs::File::create(root.path().join(format!("{scenario}.log"))).unwrap();
        let mut server = Server(
            Command::new(env!("CARGO_BIN_EXE_ovl-server"))
                .env("OVL_LISTEN", format!("127.0.0.1:{port}"))
                .env("OVL_DATA", &data)
                .env("OVL_SCAN_INTERVAL", "24h")
                .env("OVL_SECURE_COOKIES", "false")
                .env("OVL_MIN_FREE_BYTES", "1048576")
                .env("OVL_TRANSCODE_BYTES", "33554432")
                .env(
                    "OVL_ACCELERATOR",
                    if ["cpu", "quota"].contains(&scenario) {
                        "cpu"
                    } else {
                        "vaapi"
                    },
                )
                .env("OVL_HDR_ACCELERATOR", "vaapi")
                .env("PATH", &path)
                .env("OVL_TEST_MARKER", &marker)
                .env("OVL_TEST_PID", &child_pid)
                .env("OVL_TEST_FFMPEG", &real_ffmpeg)
                .env("OVL_TEST_VALID_RGB", &valid_rgb)
                .env("OVL_TEST_BLACK_RGB", &black_rgb)
                .env("OVL_TEST_PREPARED", &prepared)
                .env("OVL_TEST_FAULT", scenario)
                .env("RUST_LOG", "warn")
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(25))
            .build()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}/api/v1");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if client.get(format!("{base}/status")).send().await.is_ok() {
                break;
            }
            assert!(server.0.try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let credentials = json!({"Name":"hdr-test","Password":"test-password-123"});
        response(
            client
                .post(format!("{base}/setup"))
                .header("X-OVL-Request", "1")
                .json(&credentials),
            201,
        )
        .await;
        let login = response(
            client
                .post(format!("{base}/login"))
                .header("X-OVL-Request", "1")
                .json(&credentials),
            200,
        )
        .await;
        let token = login["token"].as_str().unwrap();
        response(
            client
                .post(format!("{base}/libraries"))
                .bearer_auth(token)
                .json(&json!({"Name":"HDR","Path":media_dir,"Kind":"movie"})),
            201,
        )
        .await;
        response(client.post(format!("{base}/scan")).bearer_auth(token), 202).await;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if response(client.get(format!("{base}/scan")).bearer_auth(token), 200).await["running"]
                == false
            {
                break;
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let items = response(client.get(format!("{base}/items")).bearer_auth(token), 200).await;
        let item = items[0]["id"].as_i64().unwrap();
        let session = response(
            client
                .post(format!("{base}/items/{item}/playback"))
                .bearer_auth(token)
                .json(&json!({"Mode":"transcode"})),
            201,
        )
        .await;
        let id = session["id"].as_str().unwrap();
        if scenario == "quota" {
            let deadline = Instant::now() + Duration::from_secs(10);
            let pid = loop {
                if let Ok(raw) = std::fs::read_to_string(&child_pid)
                    && let Ok(pid) = raw.parse::<u32>()
                    && pid > 0
                {
                    break pid.to_string();
                }
                assert!(Instant::now() < deadline, "encoder did not start");
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            let dir = data.join("transcodes").join(id);
            std::fs::rename(&dir, data.join("detached-transcode")).unwrap();
            loop {
                let reply = client
                    .get(format!("{base}/playback/{id}/index.m3u8"))
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap();
                assert!(matches!(reply.status().as_u16(), 404 | 502));
                let body: Value = reply.json().await.unwrap();
                if body["error"] == "stream not found" {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "quota failure did not remove stream"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(
                !Command::new("kill")
                    .args(["-0", &pid])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success(),
                "encoder still exists after stream removal: {pid}"
            );
            terminate(&mut server).await;
            assert_eq!(std::fs::read(&source).unwrap(), original);
            continue;
        }
        if ["cancel", "shutdown"].contains(&scenario) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !child_pid.exists() {
                assert!(Instant::now() < deadline, "picture probe did not start");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let pid = std::fs::read_to_string(&child_pid).unwrap();
            if scenario == "cancel" {
                let stopped = Instant::now();
                let reply = client
                    .delete(format!("{base}/playback/{id}"))
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(reply.status(), 200);
                assert!(
                    stopped.elapsed() < Duration::from_secs(3),
                    "session cancellation exceeded deadline"
                );
                assert!(
                    server.0.try_wait().unwrap().is_none(),
                    "DELETE must leave server running"
                );
            } else {
                terminate(&mut server).await;
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while Command::new("kill")
                .args(["-0", &pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
            {
                assert!(
                    Instant::now() < deadline,
                    "cancelled probe child still exists: {pid}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(
                !data.join("transcodes").join(id).exists(),
                "cancelled probe workspace survived"
            );
            assert_eq!(std::fs::read(&source).unwrap(), original);
            if scenario == "cancel" {
                assert!(server.0.try_wait().unwrap().is_none());
                terminate(&mut server).await;
            }
            continue;
        }
        let master = client
            .get(format!(
                "http://127.0.0.1:{port}{}",
                session["url"].as_str().unwrap()
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(master.status(), 200);
        assert!(master.text().await.unwrap().contains("v0.m3u8"));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let markers = std::fs::read_to_string(&marker).unwrap_or_default();
            if scenario != "after" || markers.contains("published-failure") {
                break;
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Give a failed child completion time to reach the fallback decision.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let markers = std::fs::read_to_string(&marker).unwrap();
        match scenario {
            "cpu" => assert_eq!(markers, "cpu\n"),
            "before" => assert_eq!(markers, "hardware\ncpu\n"),
            "black" => assert_eq!(markers, "probe-rejected\ncpu\n"),
            _ => assert_eq!(markers, "hardware\npublished-failure\n"),
        }
        let dir = data.join("transcodes").join(id);
        let output = media::inspect(&fixture_app, &dir.join("v0.m3u8"))
            .await
            .unwrap();
        assert_eq!(output.video().unwrap().color_transfer, "bt709");
        if scenario == "after" {
            assert_eq!(
                std::fs::read(dir.join("v0-0.ts")).unwrap(),
                std::fs::read(prepared.join("v0-0.ts")).unwrap(),
                "published segments must not be overwritten by CPU retry"
            );
        }
        let stop = client
            .delete(format!("{base}/playback/{id}"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(stop.status(), 200);
        terminate(&mut server).await;
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
    fixture_app.shutdown().await;
}

async fn terminate(server: &mut Server) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &server.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "server did not finish probe cancellation"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
