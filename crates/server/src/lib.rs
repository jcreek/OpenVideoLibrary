pub mod api;
pub mod auth;
pub mod db;
pub mod error;
pub mod hardware;
pub mod lists;
mod lyrics;
pub mod media;
mod music;
pub mod playback;
pub mod scan;
pub mod search;
pub mod transport;
use crate::{
    db::Db,
    error::{Error, Result},
};
use axum::{Router, middleware};
use fs2::FileExt;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
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
    /// Set while a leftover sweep or any blocking work it started is still
    /// running, including work abandoned at its time bound. Later sweeps skip
    /// rather than strand another thread on a stalled data directory.
    #[doc(hidden)]
    pub sweeping: Arc<std::sync::atomic::AtomicBool>,
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
        // Never follow cache/workspace/database symlinks out of application data.
        // Startup cleanup must not traverse a transcodes symlink into media and
        // remove a directory that happens to look like a generated session ID.
        for (name, directory) in [
            ("server.lock", false),
            ("library.db", false),
            ("library.db-wal", false),
            ("library.db-shm", false),
            ("transcodes", true),
            ("artwork", true),
        ] {
            match std::fs::symlink_metadata(config.data.join(name)) {
                Ok(metadata)
                    if metadata.file_type().is_symlink()
                        || (directory && !metadata.is_dir())
                        || (!directory && !metadata.is_file()) =>
                {
                    return Err(Error::new(
                        500,
                        "application data paths must be regular files or directories, never symlinks",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
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
        // No session exists yet, so every server-named workspace and artwork
        // download is a leftover. One that cannot be removed is logged and
        // retried by maintenance rather than preventing startup.
        let swept = remove_orphans(&config.data, &workspace_names(&config.data), None);
        if swept != Swept::default() {
            tracing::info!(
                removed = swept.removed,
                failed = swept.failed,
                "startup cleanup"
            );
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
            sweeping: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
                _=maintenance.tick()=>{
                    let _=app.db.call(|c| {c.execute("DELETE FROM sessions WHERE expires<=?",[auth::now()])?;c.execute("DELETE FROM ovl_progress_order WHERE updated<=?",[auth::now()-86400])?;Ok(())}).await;
                    // Its own task, so a slow sweep holds neither scans nor shutdown.
                    let sweeper=app.clone();
                    app.tasks.spawn(async move { tokio::select! {
                        _=sweeper.cancel.cancelled()=>{},
                        result=sweeper.sweep_orphans(ABANDONED_DOWNLOAD_AGE)=>if let Err(error)=result { tracing::warn!(reason=%error.1,"leftover cleanup failed"); },
                    }});
                },
                _=scan.tick()=>{ let _=scan::start(app.clone()).await; }
            }}
        });
    }
    /// Removes leftover server-owned files: workspaces in `transcodes/` that no
    /// live session owns, and artwork downloads in `artwork/` older than
    /// `download_age`. Only server-generated names are touched and symlinks are
    /// never followed. A file that cannot be removed is logged and counted, and
    /// the next sweep retries it. While an earlier sweep's blocking work is
    /// still running, this returns at once with nothing swept.
    pub async fn sweep_orphans(self: &Arc<Self>, download_age: Duration) -> Result<Swept> {
        self.sweep_with(download_age, SWEEP_TIMEOUT, workspace_names)
            .await
    }
    /// [`App::sweep_orphans`] with its time bound and workspace listing given,
    /// so tests can stall the listing past the bound.
    #[doc(hidden)]
    pub async fn sweep_with(
        self: &Arc<Self>,
        download_age: Duration,
        limit: Duration,
        list: fn(&Path) -> Vec<String>,
    ) -> Result<Swept> {
        use std::sync::atomic::Ordering;
        /// Clears the flag once the sweep and every blocking job it started end.
        struct Busy(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Busy {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        if self.sweeping.swap(true, Ordering::AcqRel) {
            tracing::warn!("leftover cleanup skipped: earlier cleanup work still running");
            return Ok(Swept::default());
        }
        let busy = Arc::new(Busy(self.sweeping.clone()));
        let data = self.config.data.clone();
        let held = busy.clone();
        let names = crate::media::bounded_blocking(
            &self.tasks,
            limit,
            ("leftover listing timed out", "leftover worker failed"),
            move || {
                let _held = held;
                Ok(list(&data))
            },
        )
        .await??;
        // A session creates its workspace and registers it under this lock with
        // no await in between, and removes its workspace before unregistering.
        // So a listed workspace that is not registered here has no session.
        let orphans: Vec<String> = {
            let streams = self.streams.lock().await;
            names
                .into_iter()
                .filter(|name| !streams.contains_key(name))
                .collect()
        };
        let data = self.config.data.clone();
        let held = busy.clone();
        let swept = crate::media::bounded_blocking(
            &self.tasks,
            limit,
            ("leftover cleanup timed out", "leftover worker failed"),
            move || {
                let _held = held;
                Ok(remove_orphans(&data, &orphans, Some(download_age)))
            },
        )
        .await??;
        drop(busy);
        if swept != Swept::default() {
            tracing::info!(
                removed = swept.removed,
                failed = swept.failed,
                "leftover cleanup"
            );
        }
        Ok(swept)
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

/// How long an artwork download may sit under its temporary name before
/// maintenance treats it as abandoned. A save writes and renames at once.
pub const ABANDONED_DOWNLOAD_AGE: Duration = Duration::from_secs(10 * 60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(60);

/// Files removed and left behind by one leftover sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub removed: usize,
    pub failed: usize,
}

/// Whether `name` is a server-generated token: 64 hex characters.
fn generated(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `path` is a real directory, not a symlink to one. Startup refuses
/// symlinked `transcodes` and `artwork`; this keeps a later swap from
/// redirecting a sweep.
fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// Server-named session workspaces: real directories, never symlinks.
#[doc(hidden)]
pub fn workspace_names(data: &Path) -> Vec<String> {
    let transcodes = data.join("transcodes");
    if !real_dir(&transcodes) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(transcodes) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| generated(name))
        .collect()
}

/// Removes the named workspaces and `artwork/poster-<token>` downloads: all of
/// them when `download_age` is `None` (startup, with no save in progress),
/// otherwise those last modified at least that long ago. A download whose
/// modification time is in the future (the clock stepped back) waits for the
/// next startup. Neither removal follows a symlink.
fn remove_orphans(data: &Path, workspaces: &[String], download_age: Option<Duration>) -> Swept {
    let mut swept = Swept::default();
    let mut outcome = |result: std::io::Result<()>, path: &Path| match result {
        Ok(()) => swept.removed += 1,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            swept.failed += 1;
            tracing::warn!(%error, path = %path.display(), "leftover not removed; will retry");
        }
    };
    if real_dir(&data.join("transcodes")) {
        // Only server-generated names, whatever listing supplied them.
        for name in workspaces.iter().filter(|name| generated(name)) {
            let path = data.join("transcodes").join(name);
            outcome(std::fs::remove_dir_all(&path), &path);
        }
    }
    let artwork = data.join("artwork");
    let entries = if real_dir(&artwork) {
        std::fs::read_dir(artwork).ok()
    } else {
        None
    };
    if let Some(entries) = entries {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.strip_prefix("poster-").is_some_and(generated) {
                continue;
            }
            // DirEntry metadata does not follow a symlink.
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let stale = match download_age {
                None => true,
                Some(limit) => metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age >= limit),
            };
            if metadata.is_file() && stale {
                outcome(std::fs::remove_file(entry.path()), &entry.path());
            }
        }
    }
    swept
}

impl Drop for App {
    fn drop(&mut self) {
        // Release the flock explicitly: a concurrent fork may briefly inherit
        // the open description before exec closes it, delaying close-only unlock.
        let _ = FileExt::unlock(&self._lock);
    }
}
