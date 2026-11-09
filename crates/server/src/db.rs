//! A single SQLite owner thread and a bounded admission queue. No SQLite call runs
//! on Tokio's reactor threads; overload is rejected rather than queued forever.
use crate::error::{Error, Result};
use rusqlite::Connection;
use std::{path::Path, time::Instant};
use tokio::sync::{mpsc, oneshot};
type Job = Box<dyn FnOnce(&mut Connection) + Send>;
#[derive(Clone)]
pub struct Db {
    tx: mpsc::Sender<Job>,
}
impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL;",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if !(0..=2).contains(&version) {
            return Err(Error::new(500, "unsupported database schema"));
        }
        if version < 2 {
            let tx = conn.transaction()?;
            if version == 0 {
                tx.execute_batch(include_str!("schema.sql"))?;
            }
            tx.execute_batch(include_str!("migrations/002_media_files.sql"))?;
            tx.commit()?;
        }
        // Additive tables leave user_version=2 for legacy backup/restore and rollback.
        // Existing tables and values are never rewritten by the expansion.
        conn.execute_batch(include_str!("migrations/expansion.sql"))?;
        let (tx, mut rx) = mpsc::channel::<Job>(64);
        std::thread::Builder::new()
            .name("ovl-sqlite".into())
            .spawn(move || {
                while let Some(job) = rx.blocking_recv() {
                    job(&mut conn);
                }
            })?;
        Ok(Self { tx })
    }
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let queued = Instant::now();
        self.tx
            .try_send(Box::new(move |conn| {
                tracing::debug!(
                    wait_us = queued.elapsed().as_micros() as u64,
                    "database admitted"
                );
                let _ = tx.send(f(conn));
            }))
            .map_err(|_| Error::new(503, "database busy; retry shortly"))?;
        rx.await
            .map_err(|_| Error::new(503, "database worker unavailable"))?
    }
}
