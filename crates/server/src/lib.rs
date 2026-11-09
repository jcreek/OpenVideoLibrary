pub mod api;
pub mod auth;
pub mod db;
pub mod error;
pub mod hardware;
pub mod lists;
pub mod media;
pub mod playback;
pub mod scan;
use crate::{
    db::Db,
    error::{Error, Result},
};
use axum::{Router, middleware};
use fs2::FileExt;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone)]
pub struct Config {
    pub data: PathBuf,
    pub listen: String,
    pub web: PathBuf,
    pub secure_cookies: bool,
    pub max_streams: usize,
    pub transcode_bytes: u64,
    pub min_free_bytes: u64,
    pub scan_interval: Duration,
    pub stream_idle: Duration,
}
impl Config {
    pub fn from_env() -> Self {
        fn number(key: &str, fallback: u64) -> u64 {
            std::env::var(key)
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|v| *v > 0)
                .unwrap_or(fallback)
        }
        let mut listen = std::env::var("OVL_LISTEN").unwrap_or(":8096".into());
        if listen.starts_with(':') {
            listen = format!("0.0.0.0{listen}");
        }
        Self {
            data: std::env::var_os("OVL_DATA")
                .map(PathBuf::from)
                .unwrap_or("data".into()),
            web: std::env::var_os("OVL_WEB")
                .map(PathBuf::from)
                .unwrap_or("web/dist".into()),
            listen,
            secure_cookies: std::env::var("OVL_SECURE_COOKIES").as_deref() == Ok("true"),
            max_streams: number("OVL_MAX_STREAMS", 2).min(64) as usize,
            transcode_bytes: number("OVL_TRANSCODE_BYTES", 4 << 30),
            min_free_bytes: number("OVL_MIN_FREE_BYTES", 1 << 30),
            scan_interval: std::env::var("OVL_SCAN_INTERVAL")
                .ok()
                .and_then(|s| humantime::parse_duration(&s).ok())
                .filter(|d| *d >= Duration::from_secs(60))
                .unwrap_or(Duration::from_secs(6 * 3600)),
            stream_idle: Duration::from_secs(300),
        }
    }
}
pub struct App {
    pub config: Config,
    pub db: Db,
    pub cancel: CancellationToken,
    pub tasks: TaskTracker,
    pub hashing: Arc<Semaphore>,
    pub inspection: Arc<Semaphore>,
    pub metadata: Arc<Semaphore>,
    pub scans: Arc<Semaphore>,
    pub streams: Mutex<HashMap<String, Arc<playback::Stream>>>,
    pub setup: Mutex<()>,
    pub rates: Mutex<HashMap<std::net::IpAddr, (u32, std::time::Instant)>>,
    _lock: File,
}
impl App {
    pub fn open(mut config: Config) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&config.data)?;
        config.data = std::fs::canonicalize(&config.data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config.data, std::fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(config.data.join("server.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| Error::new(409, "data directory already in use"))?;
        let db = Db::open(&config.data.join("library.db"))?;
        // Only server-owned, randomly named workspaces may be cleaned up.
        if let Ok(entries) = std::fs::read_dir(config.data.join("transcodes")) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.len() == 64
                    && name.bytes().all(|b| b.is_ascii_hexdigit())
                    && e.file_type()?.is_dir()
                {
                    std::fs::remove_dir_all(e.path())?;
                }
            }
        }
        Ok(Arc::new(Self {
            config,
            db,
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
            hashing: Arc::new(Semaphore::new(2)),
            inspection: Arc::new(Semaphore::new(2)),
            metadata: Arc::new(Semaphore::new(2)),
            scans: Arc::new(Semaphore::new(1)),
            streams: Mutex::new(HashMap::new()),
            setup: Mutex::new(()),
            rates: Mutex::new(HashMap::new()),
            _lock: lock,
        }))
    }
    pub fn router(self: &Arc<Self>) -> Router {
        api::routes(self.clone()).layer(middleware::from_fn(api::security_headers))
    }
    pub fn background(self: &Arc<Self>) {
        let app = self.clone();
        self.tasks.spawn(async move {
            let mut maintenance=tokio::time::interval(Duration::from_secs(60));
            let mut scan=tokio::time::interval(app.config.scan_interval);
            maintenance.tick().await; scan.tick().await;
            loop { tokio::select! {
                _=app.cancel.cancelled()=>break,
                _=maintenance.tick()=>{ let _=app.db.call(|c| {c.execute("DELETE FROM sessions WHERE expires<=?",[auth::now()])?;Ok(())}).await; },
                _=scan.tick()=>{ let _=scan::start(app.clone()).await; }
            }}
        });
    }
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        if let Err(error) = self.db.shutdown().await {
            tracing::error!(%error,"database shutdown failed");
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Release the flock explicitly: a concurrent fork may briefly inherit
        // the open description before exec closes it, delaying close-only unlock.
        let _ = FileExt::unlock(&self._lock);
    }
}
