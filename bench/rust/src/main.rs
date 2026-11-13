use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::UNIX_EPOCH,
};
use tower_http::services::ServeFile;
#[derive(Clone)]
struct App {
    db: Arc<Mutex<Connection>>,
    scanning: Arc<AtomicBool>,
}
#[derive(Deserialize)]
struct Filter {
    q: Option<String>,
    offset: Option<u32>,
}
#[derive(Serialize)]
struct Item {
    id: i64,
    title: String,
}
async fn items(
    State(app): State<App>,
    Query(f): Query<Filter>,
) -> Result<Json<Vec<Item>>, StatusCode> {
    tokio::task::spawn_blocking(move||{let db=app.db.lock().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;let mut stmt=db.prepare("SELECT id,title FROM items WHERE title LIKE ? ORDER BY title COLLATE NOCASE,id LIMIT 100 OFFSET ?").map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;let rows=stmt.query_map(params![format!("{}%",f.q.unwrap_or_default()),f.offset.unwrap_or(0)],|r|Ok(Item{id:r.get(0)?,title:r.get(1)?})).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;rows.collect::<Result<Vec<_>,_>>().map(Json).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)}).await.map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?
}
fn scan_root(app: &App, root: &str) -> Result<(), Box<dyn std::error::Error>> {
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        };
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !["mkv", "mp4", "avi", "m4v", "mov", "ts", "webm"].contains(&ext.as_str()) {
            continue;
        };
        let metadata = entry.metadata()?;
        let mtime = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as i64;
        let size = metadata.len() as i64;
        let path = path.to_string_lossy();
        let db = app.db.lock().map_err(|_| "database poisoned")?;
        let old = db.query_row(
            "SELECT mtime,size FROM items WHERE path=?",
            [path.as_ref()],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        );
        if old.ok() == Some((mtime, size)) {
            continue;
        };
        let title = entry
            .path()
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .replace('.', " ");
        db.execute("INSERT INTO items(path,title,mtime,size) VALUES(?,?,?,?) ON CONFLICT(path) DO UPDATE SET mtime=excluded.mtime,size=excluded.size",params![path,title,mtime,size])?;
    }
    Ok(())
}
async fn scan(State(app): State<App>) -> StatusCode {
    let root = match std::env::var("BENCH_ROOT") {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    if app.scanning.swap(true, Ordering::AcqRel) {
        return StatusCode::CONFLICT;
    };
    tokio::task::spawn_blocking(move || {
        if let Err(e) = scan_root(&app, &root) {
            eprintln!("scan: {e}")
        };
        app.scanning.store(false, Ordering::Release);
    });
    StatusCode::ACCEPTED
}
#[tokio::main]
async fn main() {
    let db = Connection::open(std::env::var("BENCH_DB").expect("BENCH_DB required")).unwrap();
    let app = App {
        db: Arc::new(Mutex::new(db)),
        scanning: Arc::new(AtomicBool::new(false)),
    };
    let mut router = Router::new()
        .route("/items", get(items))
 .route("/info",get(|| async {Json(serde_json::json!({"query_version":"prefix-nocase-v2","sqlite_version":rusqlite::version(),"runtime":"rust","cpu_count":std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1)}))}))
        .route("/scan", post(scan));
    if let Ok(media) = std::env::var("BENCH_MEDIA") {
        router = router.route_service("/media", ServeFile::new(media))
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8097")
        .await
        .unwrap();
    axum::serve(listener, router.with_state(app)).await.unwrap();
}
