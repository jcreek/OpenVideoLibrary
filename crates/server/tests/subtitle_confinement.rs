//! Text-subtitle burn-in confinement, observed at the FFmpeg boundary.
//!
//! libass's `subtitles` filter opens its file without FFmpeg's input
//! whitelists, so it must never receive library media. These checks put an
//! FFmpeg wrapper first on PATH that records what every invocation was handed,
//! can fail or stall chosen invocations, and can rewrite the library file in
//! place just before FFmpeg reads it. PATH is process-wide, so this binary
//! holds a single test that runs the scenarios in order.
//!
//! Scenario 1 (the inode check) is the regression guard for the former
//! residual. The rewrite scenario shows the confined extraction refusing a
//! library file rewritten after the server's checks; it does not reproduce
//! the old timing window inside one FFmpeg process. A 504 at the 6 h
//! process deadline is not exercised.
#![cfg(unix)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use ovl_server::{App, Config, media};
use serde_json::{Value, json};
use std::{
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tempfile::TempDir;
use tower::ServiceExt;

const WRAPPER: &str = r#"#!/bin/sh
# Test double for ffmpeg: logs each call, then fails, rewrites or delegates
# according to the mode file.
dir=$(dirname "$0")
mode=$(cat "$dir/mode")
kind=other
attach=no
inodes=""
for a in "$@"; do
  case "$a" in
    subtitle.mkv) kind=extract ;;
    "0:t?") attach=yes ;;
    *subtitles=filename=/dev/fd/*)
      kind=convert
      fd=${a#*subtitles=filename=/dev/fd/}
      fd=${fd%%:*}
      inodes="$inodes $(python3 -I -c 'import os,sys;print(os.fstat(int(sys.argv[1])).st_ino)' "$fd")" ;;
  esac
done
echo "$kind attach=$attach inodes=$inodes" >> "$dir/log"
case "$mode:$kind" in
  fail-attachments:extract) [ "$attach" = yes ] && { echo "Attachment stream 1 has no mimetype tag" >&2; exit 1; } ;;
  fail-extract:extract) echo "extraction refused" >&2; exit 1 ;;
  hang:extract) echo $$ > "$dir/pid"; exec sleep 60 ;;
  swap:extract|swap:convert)
    if [ ! -e "$dir/swapped" ]; then
      : > "$dir/swapped"
      # Same inode: the anchored handle the server already opened sees this.
      cat "$dir/playlist" > "$(cat "$dir/media")"
    fi ;;
esac
exec "$(cat "$dir/real")" "$@"
"#;

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

fn find(program: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join(program))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{program} not on PATH"))
}

/// Writes a 6 s Matroska source with one ASS cue and an embedded font.
fn write_source(root: &Path, path: &Path, ffmpeg: &Path) {
    let font = root.join("font.ttf");
    std::fs::write(&font, include_bytes!("fixtures/OVLFixtureBars.ttf")).unwrap();
    let ass = root.join("cue.ass");
    std::fs::write(
        &ass,
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,OVLFixtureBars,100,&H000000FF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,7,0,0,0,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\an7\\pos(80,60)}X\n",
    )
    .unwrap();
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=640x360:rate=12",
        ])
        .arg("-i")
        .arg(&ass)
        .args([
            "-t", "6", "-map", "0:v", "-map", "1:s", "-c:v", "libx264", "-c:s", "copy",
        ])
        .arg("-attach")
        .arg(&font)
        .args(["-metadata:s:t", "mimetype=font/ttf"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

struct Harness {
    app: Arc<App>,
    router: Router,
    token: String,
    item: i64,
    media: PathBuf,
    wrapper: PathBuf,
}

impl Harness {
    async fn new(root: &TempDir, wrapper: &Path, ffmpeg: &Path) -> Self {
        let app = App::open(Config {
            data: root.path().join("data"),
            listen: "127.0.0.1:0".into(),
            web: root.path().join("web"),
            secure_cookies: false,
            max_streams: 2,
            transcode_bytes: 16 << 20,
            min_free_bytes: 1 << 20,
            scan_interval: Duration::from_secs(3600),
            stream_idle: Duration::from_secs(600),
        })
        .unwrap();
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
        let (status, v) = request(
            &router,
            "POST",
            "/api/v1/login",
            None,
            Some(json!({"Name":"admin","Password":"test-password-123"})),
        )
        .await;
        assert_eq!(status, 200, "{v}");
        let token = v["token"].as_str().unwrap().to_owned();
        let dir = root.path().join("media");
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let media = dir.join("Example.S01E01.mkv");
        write_source(root.path(), &media, ffmpeg);
        let (library, path) = (
            dir.to_string_lossy().into_owned(),
            media.to_string_lossy().into_owned(),
        );
        let size = std::fs::metadata(&media).unwrap().len() as i64;
        let item = app
            .db
            .call(move |c| {
                c.execute(
                    "INSERT INTO libraries(name,path,kind) VALUES('TV',?,'tv')",
                    [library],
                )?;
                let lib = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO items(title,series,season,episode) VALUES('Example','Example',1,1)",
                    [],
                )?;
                let item = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,?,?,0,?)",
                    rusqlite::params![item, lib, path, size],
                )?;
                Ok(item)
            })
            .await
            .unwrap();
        std::fs::write(wrapper.join("media"), media.to_string_lossy().as_bytes()).unwrap();
        Self {
            app,
            router,
            token,
            item,
            media,
            wrapper: wrapper.to_owned(),
        }
    }

    /// Sets the wrapper mode, clears its log, and starts a burn-in session.
    async fn start(&self, mode: &str) -> (String, Arc<ovl_server::playback::Stream>) {
        std::fs::write(self.wrapper.join("mode"), mode).unwrap();
        for name in ["log", "swapped", "pid"] {
            let _ = std::fs::remove_file(self.wrapper.join(name));
        }
        let (status, session) = request(
            &self.router,
            "POST",
            &format!("/api/v1/items/{}/playback", self.item),
            Some(&self.token),
            Some(json!({"Mode":"transcode","Subtitle":0})),
        )
        .await;
        assert_eq!(status, 201, "{mode}: {session}");
        let id = session["id"].as_str().unwrap().to_owned();
        let stream = self.app.streams.lock().await.get(&id).unwrap().clone();
        (id, stream)
    }

    /// Starts a burn-in session and returns it once its conversion has
    /// finished or failed. A read blocked on a FIFO would time out here.
    async fn burn_in(&self, mode: &str) -> (String, Arc<ovl_server::playback::Stream>) {
        let (id, stream) = self.start(mode).await;
        tokio::time::timeout(Duration::from_secs(60), async {
            while !stream.finished.load(std::sync::atomic::Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{mode}: session did not finish"));
        (id, stream)
    }

    /// Calls the wrapper saw: (kind, attachments mapped, subtitle inodes).
    fn calls(&self) -> Vec<(String, bool, Vec<u64>)> {
        std::fs::read_to_string(self.wrapper.join("log"))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let mut parts = line.split_whitespace();
                let kind = parts.next().unwrap().to_owned();
                let attach = parts.next() == Some("attach=yes");
                let inodes = parts
                    .filter(|p| *p != "inodes=")
                    .map(|p| p.trim_start_matches("inodes=").parse().unwrap())
                    .collect();
                (kind, attach, inodes)
            })
            .filter(|(kind, _, _)| kind != "other")
            .collect()
    }

    async fn stop(&self, id: &str) {
        assert_eq!(
            request(
                &self.router,
                "DELETE",
                &format!("/api/v1/playback/{id}"),
                Some(&self.token),
                None,
            )
            .await
            .0,
            200
        );
    }
}

#[tokio::test]
#[ignore = "requires FFmpeg with libx264 and libass, FFprobe and python3 on PATH; replaces PATH for this test binary"]
async fn libass_only_ever_reads_the_extracted_subtitle() {
    let (real_ffmpeg, real_ffprobe) = (find("ffmpeg"), find("ffprobe"));
    let wrapper = TempDir::new().unwrap();
    let bin = wrapper.path();
    std::fs::write(bin.join("ffmpeg"), WRAPPER).unwrap();
    std::fs::set_permissions(bin.join("ffmpeg"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&real_ffprobe, bin.join("ffprobe")).unwrap();
    std::fs::write(bin.join("real"), real_ffmpeg.to_string_lossy().as_bytes()).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    // SAFETY: the only test in this binary sets PATH before starting the
    // server, and nothing else here reads the environment concurrently.
    unsafe { std::env::set_var("PATH", path) };

    let root = TempDir::new().unwrap();
    let h = Harness::new(&root, bin, &real_ffmpeg).await;
    let media_inode = std::fs::metadata(&h.media).unwrap().ino();

    // 1. libass gets a handle on the session's extracted file, never on the
    //    library media. The old code bound the filter to the media itself.
    let (id, stream) = h.burn_in("observe").await;
    let failure = stream.error.lock().await.clone();
    assert!(failure.is_none(), "{failure:?}");
    let calls = h.calls();
    let filter_inodes: Vec<u64> = calls
        .iter()
        .filter(|c| c.0 == "convert")
        .flat_map(|c| c.2.clone())
        .collect();
    assert!(!filter_inodes.is_empty(), "{calls:?}");
    assert!(
        !filter_inodes.contains(&media_inode),
        "libass was handed library media: {calls:?}"
    );
    let extracted = std::fs::metadata(stream.dir.join("subtitle.mkv")).unwrap();
    assert!(
        filter_inodes.iter().all(|i| *i == extracted.ino()),
        "libass handles {filter_inodes:?}, extracted {}",
        extracted.ino()
    );
    assert_eq!(
        calls.iter().filter(|c| c.0 == "extract").count(),
        1,
        "{calls:?}"
    );
    let info = media::inspect(&h.app, &stream.dir.join("subtitle.mkv"))
        .await
        .unwrap();
    assert_eq!(info.tracks("subtitle").len(), 1);
    assert_eq!(info.tracks("attachment").len(), 1, "font copied");
    h.stop(&id).await;

    // 2. An extraction refused because of its attachments is retried once
    //    without them, and burn-in still runs from the extracted file.
    let (id, stream) = h.burn_in("fail-attachments").await;
    let failure = stream.error.lock().await.clone();
    assert!(failure.is_none(), "{failure:?}");
    let calls = h.calls();
    let extractions: Vec<bool> = calls
        .iter()
        .filter(|c| c.0 == "extract")
        .map(|c| c.1)
        .collect();
    assert_eq!(extractions, [true, false], "{calls:?}");
    assert_eq!(calls.iter().filter(|c| c.0 == "convert").count(), 1);
    let info = media::inspect(&h.app, &stream.dir.join("subtitle.mkv"))
        .await
        .unwrap();
    assert_eq!(info.tracks("subtitle").len(), 1);
    assert!(info.tracks("attachment").is_empty());
    assert!(stream.dir.join("index.m3u8").exists());
    h.stop(&id).await;

    // 3. When extraction keeps failing, burn-in fails closed after exactly one
    //    retry: no conversion runs without its confined subtitle source.
    let (id, stream) = h.burn_in("fail-extract").await;
    assert_eq!(
        stream.error.lock().await.as_deref(),
        Some("subtitle extraction failed")
    );
    let calls = h.calls();
    assert_eq!(
        calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["extract", "extract"],
        "{calls:?}"
    );
    assert!(!stream.dir.join("index.m3u8").exists());
    h.stop(&id).await;

    // 4. The library file is rewritten in place, after the server's checks
    //    and just before FFmpeg reads it, into a playlist naming a local
    //    secret. The secret is a FIFO: any process opening it to read would
    //    block, so the session would never finish. The confined extraction
    //    refuses the playlist, nothing opens the secret, and burn-in fails
    //    closed after one retry, with no conversion.
    // A media extension, so FFmpeg's HLS demuxer would open it as a segment.
    let secret = root.path().join("secret.ts");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&secret)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(
        bin.join("playlist"),
        format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nfile:{}\n#EXT-X-ENDLIST\n",
            secret.display()
        ),
    )
    .unwrap();
    let original = std::fs::read(&h.media).unwrap();
    let (id, stream) = h.burn_in("swap").await;
    assert!(bin.join("swapped").exists(), "rewrite did not happen");
    assert_eq!(
        std::fs::metadata(&h.media).unwrap().ino(),
        media_inode,
        "rewritten in place"
    );
    assert_eq!(
        stream.error.lock().await.as_deref(),
        Some("subtitle extraction failed")
    );
    let calls = h.calls();
    assert_eq!(
        calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["extract", "extract"],
        "{calls:?}"
    );
    // No reader holds the FIFO open now (ENXIO for a non-blocking writer).
    let writer = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&secret);
    assert_eq!(
        writer.err().and_then(|e| e.raw_os_error()),
        Some(libc::ENXIO)
    );
    h.stop(&id).await;
    std::fs::write(&h.media, original).unwrap();

    // 5. Stopping the session during extraction kills and reaps the stalled
    //    extraction, does not retry it, and removes the session.
    let (id, stream) = h.start("hang").await;
    let pid = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(pid) = std::fs::read_to_string(bin.join("pid"))
                .ok()
                .and_then(|p| p.trim().parse::<i32>().ok())
            {
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("extraction did not start");
    // SAFETY: signal 0 only probes whether the process exists.
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "extraction not running");
    h.stop(&id).await;
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "stalled extraction still exists after stop"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    let calls = h.calls();
    assert_eq!(
        calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["extract"],
        "{calls:?}"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !h.app.streams.lock().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!stream.dir.exists());
    h.app.shutdown().await;
}
