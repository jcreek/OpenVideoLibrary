#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn stalled_request_and_direct_reader_do_not_hold_sigterm_open() {
    for stall_response in [false, true] {
        let root = TempDir::new().unwrap();
        let media = root.path().join("media");
        std::fs::create_dir(&media).unwrap();
        std::fs::File::create(media.join("Large.mp4"))
            .unwrap()
            .set_len(64 << 20)
            .unwrap();
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let addr = format!("127.0.0.1:{port}");
        let log = std::fs::File::create(root.path().join("server.log")).unwrap();
        let mut server = Server(
            Command::new(env!("CARGO_BIN_EXE_ovl-server"))
                .env("OVL_LISTEN", &addr)
                .env("OVL_DATA", root.path().join("data"))
                .env("OVL_SCAN_INTERVAL", "24h")
                .env("RUST_LOG", "error")
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let base = format!("http://{addr}/api/v1");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if client.get(format!("{base}/status")).send().await.is_ok() {
                break;
            }
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "server exited before readiness"
            );
            assert!(Instant::now() < deadline, "server startup timed out");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut socket = tokio::net::TcpStream::connect(&addr).await.unwrap();
        if stall_response {
            let credentials = json!({"Name":"shutdown-test","Password":"test-password-123"});
            assert_eq!(
                client
                    .post(format!("{base}/setup"))
                    .header("X-OVL-Request", "1")
                    .json(&credentials)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                201
            );
            let login: Value = client
                .post(format!("{base}/login"))
                .header("X-OVL-Request", "1")
                .json(&credentials)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let token = login["token"].as_str().unwrap();
            assert_eq!(
                client
                    .post(format!("{base}/libraries"))
                    .bearer_auth(token)
                    .json(&json!({"Name":"Fixture","Path":media,"Kind":"movie"}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                201
            );
            assert_eq!(
                client
                    .post(format!("{base}/scan"))
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                202
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let state: Value = client
                    .get(format!("{base}/scan"))
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                if state["running"] == false {
                    break;
                }
                assert!(Instant::now() < deadline, "scan timeout");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let items: Value = client
                .get(format!("{base}/items"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let id = items[0]["id"].as_i64().unwrap();
            socket.write_all(format!("GET /api/v1/items/{id}/file HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n\r\n").as_bytes()).await.unwrap();
            let mut header = [0u8; 1024];
            let n = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut header))
                .await
                .unwrap()
                .unwrap();
            assert!(
                std::str::from_utf8(&header[..n])
                    .unwrap_or("")
                    .starts_with("HTTP/1.1 200")
            );
            // Leave the remainder of the 64MiB sparse fixture unread.
        } else {
            socket.write_all(b"POST /api/v1/setup HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 64000\r\n\r\n{").await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
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
                assert!(status.success(), "server shutdown failed: {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "stalled peer held shutdown open; response={stall_response}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(socket);
    }
}
