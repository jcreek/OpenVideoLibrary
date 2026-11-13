use crate::{
    App,
    auth::{self, User, admin, parse},
    error::{Error, Result},
    lists, media, playback, scan,
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, post, put},
};
use rusqlite::{OptionalExtension, params, params_from_iter, types::Value as Sql};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
pub fn routes(app: Arc<App>) -> Router {
    let protected = Router::new()
        .route(
            "/me",
            get(|Extension(u): Extension<User>| async move { Json(u) }),
        )
        .route("/logout", post(auth::logout))
        .route("/libraries", get(libraries).post(add_library))
        .route("/users", get(users).post(add_user))
        .route("/users/{id}/access", put(grants))
        .route("/users/{id}/password", post(password))
        .route("/items", get(items))
        .route("/series", get(series))
        .route("/music/albums", get(crate::music::albums))
        .route("/music/artists", get(crate::music::artists))
        .route("/music/tracks", get(crate::music::tracks))
        .route("/scan", get(scan_status).post(scan_start))
        .route("/items/{id}", axum::routing::patch(edit_item))
        .route("/items/{id}/progress", put(progress))
        .route("/items/{id}/file", get(media::file))
        .route("/items/{id}/probe", get(media::probe))
        .route("/items/{id}/telemetry", post(telemetry))
        .route("/items/{id}/artwork", get(media::artwork))
        .route("/items/{id}/subtitles", post(media::subtitle))
        .route("/items/{id}/lyrics", get(crate::lyrics::get))
        .route("/items/{id}/match", post(media::metadata_match))
        .route("/items/{id}/playback", post(playback::start))
        .route("/playback/{session}/{file}", get(playback::segment))
        .route("/playback/{session}", axum::routing::delete(playback::stop))
        .route("/capabilities", get(media::capabilities))
        .route("/lists", get(lists::all).post(lists::create))
        .route("/lists/{id}/cursor", put(lists::cursor))
        .route(
            "/lists/{id}",
            get(lists::one).put(lists::update).delete(lists::delete),
        )
        .route_layer(middleware::from_fn_with_state(
            app.clone(),
            auth::authenticate,
        ));
    Router::new()
        .nest(
            "/api/v1",
            protected
                .route("/status", get(status))
                .route("/setup", post(auth::setup))
                .route("/login", post(auth::login)),
        )
        .fallback_service(
            tower_http::services::ServeDir::new(app.config.web.clone())
                .append_index_html_on_directories(true),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn(body_deadline))
        .layer(
            tower_http::trace::TraceLayer::new_for_http().on_response(
                tower_http::trace::DefaultOnResponse::new().level(tracing::Level::INFO),
            ),
        )
        .with_state(app)
}
const MAX_BODY: usize = 64 * 1024;
/// Total time a client may take to send a request body, on every route
/// (including unauthenticated setup and login). Bodies are small and buffered,
/// so a stalled or trickling upload cannot hold a handler open.
pub const BODY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);
async fn body_deadline(req: Request, next: Next) -> Result<Response> {
    let (parts, body) = req.into_parts();
    let limited = http_body_util::Limited::new(body, MAX_BODY);
    let bytes = match tokio::time::timeout(BODY_DEADLINE, http_body_util::BodyExt::collect(limited))
        .await
    {
        Err(_) => return Err(Error::new(408, "request body timed out")),
        Ok(Err(error)) if error.is::<http_body_util::LengthLimitError>() => {
            return Err(Error::new(413, "request body too large"));
        }
        Ok(Err(_)) => return Err(Error::new(400, "invalid request body")),
        Ok(Ok(collected)) => collected.to_bytes(),
    };
    Ok(next
        .run(Request::from_parts(parts, axum::body::Body::from(bytes)))
        .await)
}
pub async fn security_headers(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let mut res = next.run(req).await;
    let success = res.status().is_success();
    let h = res.headers_mut();
    h.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("Referrer-Policy", HeaderValue::from_static("same-origin"));
    h.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    h.insert("Content-Security-Policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self' blob:; worker-src 'self' blob:; connect-src 'self'"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if path.starts_with("/assets/") && success {
            "public, max-age=31536000, immutable"
        } else {
            "no-store"
        }),
    );
    res
}
async fn status(State(app): State<Arc<App>>) -> Result<Json<Value>> {
    Ok(Json(app.db.call(|c|Ok(json!({"setup":c.query_row("SELECT count(*) FROM users",[],|r|r.get::<_,i64>(0))?==0}))).await?))
}
async fn libraries(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
) -> Result<Json<Value>> {
    Ok(Json(app.db.call(move|c|{
        let mut s=c.prepare("SELECT id,name,CASE WHEN EXISTS(SELECT 1 FROM ovl_music_libraries m WHERE m.library_id=l.id) THEN 'music' ELSE kind END,offline FROM libraries l WHERE ? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=l.id AND a.user_id=?) ORDER BY id")?;
        let rows=s.query_map(params![u.admin,u.id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"offline":r.get::<_,bool>(3)?})))?;
        Ok(Value::Array(rows.collect::<std::result::Result<Vec<_>,_>>()?))
    }).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Library {
    name: String,
    path: String,
    kind: String,
}
async fn add_library(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    body: Bytes,
) -> Result<impl axum::response::IntoResponse> {
    admin(u)?;
    let b: Library = parse(&body)?;
    if b.name.is_empty() || !["movie", "tv", "music"].contains(&b.kind.as_str()) {
        return Err(Error::new(400, "invalid library"));
    }
    let path = tokio::fs::canonicalize(b.path)
        .await
        .map_err(|_| Error::new(400, "library directory unavailable"))?;
    if path.starts_with(&app.config.data) || app.config.data.starts_with(&path) {
        return Err(Error::new(
            400,
            "media and application data directories must be separate",
        ));
    }
    if !tokio::fs::metadata(&path).await?.is_dir() {
        return Err(Error::new(400, "library directory unavailable"));
    }
    let out = app
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            // Nested roots would index one file under two libraries' grants.
            let roots = tx
                .prepare("SELECT path FROM libraries")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if roots
                .iter()
                .map(std::path::Path::new)
                .any(|root| root.starts_with(&path) || path.starts_with(root))
            {
                return Err(Error::new(409, "library roots must not overlap"));
            }
            let music = b.kind == "music";
            tx.execute(
                "INSERT INTO libraries(name,path,kind) VALUES(?,?,?)",
                params![
                    b.name,
                    path.to_string_lossy(),
                    if music { "movie" } else { &b.kind }
                ],
            )
            .map_err(|_| Error::new(409, "library exists"))?;
            let id = tx.last_insert_rowid();
            if music {
                tx.execute(
                    "INSERT INTO ovl_music_libraries(library_id) VALUES(?)",
                    [id],
                )?;
            }
            tx.commit()?;
            Ok(json!({"id":id}))
        })
        .await?;
    Ok((axum::http::StatusCode::CREATED, Json(out)))
}
async fn add_user(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    body: Bytes,
) -> Result<impl axum::response::IntoResponse> {
    admin(u)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(auth::create(app, parse(&body)?, false).await?),
    ))
}
async fn users(State(app): State<Arc<App>>, Extension(u): Extension<User>) -> Result<Json<Value>> {
    admin(u)?;
    Ok(Json(
        app.db
            .call(|c| {
                let mut s = c.prepare("SELECT id,name,admin FROM users ORDER BY id LIMIT 10000")?;
                let users = s
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, bool>(2)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let mut out = vec![];
                for (id, name, admin) in users {
                    let mut s = c.prepare(
                        "SELECT library_id FROM access WHERE user_id=? ORDER BY library_id",
                    )?;
                    let libs = s
                        .query_map([id], |r| r.get::<_, i64>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    out.push(json!({"id":id,"name":name,"admin":admin,"libraries":libs}));
                }
                Ok(Value::Array(out))
            })
            .await?,
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Access {
    libraries: Vec<i64>,
}
async fn grants(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    admin(u)?;
    let b: Access = parse(&body)?;
    if b.libraries.len() > 1000 {
        return Err(Error::new(400, "too many libraries"));
    }
    app.db
        .call(move |c| {
            let tx = c.transaction()?;
            let is_admin: bool = tx
                .query_row("SELECT admin FROM users WHERE id=?", [id], |r| r.get(0))
                .optional()?
                .ok_or_else(|| Error::new(404, "account not found"))?;
            if is_admin {
                return Err(Error::new(400, "administrator already has all libraries"));
            }
            tx.execute("DELETE FROM access WHERE user_id=?", [id])?;
            for l in b.libraries {
                tx.execute("INSERT INTO access VALUES(?,?)", params![id, l])
                    .map_err(|_| Error::new(400, "invalid or duplicate library"))?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Password {
    password: String,
}
async fn password(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    admin(u)?;
    let b: Password = parse(&body)?;
    if !(12..=72).contains(&b.password.len()) {
        return Err(Error::new(400, "password must be 12–72 bytes"));
    }
    let permit = app
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(429, "password hashing busy"))?;
    let hash = app
        .tasks
        .spawn_blocking(move || {
            let _permit = permit;
            bcrypt::hash(b.password, bcrypt::DEFAULT_COST)
        })
        .await
        .map_err(|_| Error::new(500, "password hashing failed"))?
        .map_err(|_| Error::new(500, "password hashing failed"))?;
    app.db
        .call(move |c| {
            let tx = c.transaction()?;
            if tx.execute(
                "UPDATE users SET password=? WHERE id=?",
                params![hash.as_bytes(), id],
            )? == 0
            {
                return Err(Error::new(404, "account not found"));
            }
            tx.execute("DELETE FROM sessions WHERE user_id=?", [id])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"ok":true})))
}
pub fn item_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,i64>(0)?,"library_id":r.get::<_,i64>(1)?,"title":r.get::<_,String>(2)?,"series":r.get::<_,String>(3)?,"season":r.get::<_,i64>(4)?,"episode":r.get::<_,i64>(5)?,"overview":r.get::<_,String>(6)?,"position":r.get::<_,f64>(7)?,"watched":r.get::<_,bool>(8)?}),
    )
}
async fn items(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    Ok(Json(
        app.db
            .call(move |c| {
                let tx = c.transaction()?;
                // Pin the main read snapshot before checking external data_version
                // or using cached IDs. External commits cannot mix old matches
                // with newer item metadata within one response.
                tx.query_row("SELECT 1 FROM main.items LIMIT 1", [], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?;
                let (sql, args) = item_query(&tx, u, &q)?;
                let mut s = tx.prepare(&sql)?;
                let rows = s
                    .query_map(params_from_iter(args), item_row)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                drop(s);
                if !tx.is_autocommit() {
                    tx.commit()?;
                }
                Ok(Value::Array(rows))
            })
            .await?,
    ))
}
/// Shared production query builder, also used by the reproducible SQL profiler.
pub fn item_query(
    c: &rusqlite::Connection,
    u: User,
    q: &HashMap<String, String>,
) -> Result<(String, Vec<Sql>)> {
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0 && *v <= 200)
        .unwrap_or(100);
    let offset = q
        .get("offset")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    let continuing = q.get("continue").is_some_and(|v| v == "true");
    if !continuing
        && ["q", "series", "season"]
            .iter()
            .all(|key| q.get(*key).is_none_or(String::is_empty))
        && let Some(library) = q.get("library").and_then(|id| id.parse::<i64>().ok())
    {
        // Page the covering library/title index before fetching progress or metadata.
        let sql = "WITH page AS MATERIALIZED (
            SELECT item_id AS id,library_id,title,file_id
            FROM ovl_browse_order
            WHERE library_id=? AND (? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=? AND a.user_id=?))
            ORDER BY title,item_id,file_id LIMIT ? OFFSET ?
        )
        SELECT i.id,page.library_id,i.title,i.series,i.season,i.episode,i.overview,
            coalesce(p.position,0),coalesce(p.watched,0)
        FROM page CROSS JOIN items i ON i.id=page.id
        LEFT JOIN progress p ON p.item_id=i.id AND p.user_id=?
        ORDER BY page.title,page.id,page.file_id";
        return Ok((
            sql.into(),
            vec![
                library.into(),
                (u.admin as i64).into(),
                library.into(),
                u.id.into(),
                limit.into(),
                offset.into(),
                u.id.into(),
            ],
        ));
    }
    let mut sql = String::from(
        "SELECT i.id,f.library_id,i.title,i.series,i.season,i.episode,i.overview,coalesce(p.position,0),coalesce(p.watched,0) FROM items i JOIN media_files f ON f.item_id=i.id LEFT JOIN progress p ON p.item_id=i.id AND p.user_id=? WHERE (? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=f.library_id AND a.user_id=?))",
    );
    let mut args: Vec<Sql> = vec![u.id.into(), (u.admin as i64).into(), u.id.into()];
    if let Some(q) = q.get("q").filter(|s| !s.is_empty()) {
        sql.push_str(" AND i.id IN(SELECT rowid FROM search WHERE search MATCH ?)");
        args.push(format!("\"{}\"*", q.replace('"', "\"\"")).into());
    }
    for (param, col) in [
        ("library", "f.library_id"),
        ("series", "i.series"),
        ("season", "i.season"),
    ] {
        if let Some(v) = q.get(param).filter(|s| !s.is_empty()) {
            sql.push_str(&format!(" AND {col}=?"));
            args.push(v.clone().into());
        }
    }
    if continuing {
        sql.push_str(" AND p.position>0 AND p.watched=0");
    }
    sql.push_str(if q.get("series").is_some_and(|s| !s.is_empty()) {
        " ORDER BY i.season,i.episode,i.id"
    } else if continuing {
        " ORDER BY p.updated DESC,i.id"
    } else {
        " ORDER BY i.title,i.id"
    });
    sql.push_str(" LIMIT ? OFFSET ?");
    args.extend([limit.into(), offset.into()]);
    // The measured unfiltered browse plan otherwise scans every media file and
    // sorts the whole library. The title index can stop after offset + limit.
    // A bounded census distinguishes broad FTS matches from selective searches.
    // Only broad, unfiltered title ordering uses the title index; sparse matches
    // keep SQLite's rowid plan. The MATCH expression and result ordering stay
    // identical. Never use correlated FTS: the profile showed a large regression.
    let plan = if ["library", "series", "season"]
        .iter()
        .all(|key| q.get(*key).is_none_or(|v| v.is_empty()))
        && !continuing
    {
        if let Some(term) = q.get("q").filter(|v| !v.is_empty()) {
            crate::search::plan(c, &format!("\"{}\"*", term.replace('"', "\"\"")))?
        } else {
            crate::search::Plan::Sparse
        }
    } else {
        crate::search::Plan::Sparse
    };
    if let crate::search::Plan::Cached(id) = plan {
        sql=sql.replace("i.id IN(SELECT rowid FROM search WHERE search MATCH ?)","EXISTS(SELECT 1 FROM ovl_query_cache.entries ce WHERE ce.query_id=? AND ce.item_id=i.id)");
        args[3] = id.into();
    }
    let broad_search = matches!(
        plan,
        crate::search::Plan::Broad | crate::search::Plan::Cached(_)
    );
    if ["library", "series", "season"]
        .iter()
        .all(|key| q.get(*key).is_none_or(|v| v.is_empty()))
        && !continuing
        && (q.get("q").is_none_or(|v| v.is_empty()) || broad_search)
    {
        sql = sql.replace(
            "FROM items i JOIN",
            "FROM items i INDEXED BY items_title JOIN",
        );
    }
    Ok((sql, args))
}
async fn series(State(app): State<Arc<App>>, Extension(u): Extension<User>) -> Result<Json<Value>> {
    Ok(Json(app.db.call(move|c|{let mut s=c.prepare("SELECT f.library_id,i.series,count(*),count(DISTINCT i.season),sum(coalesce(p.watched,0)) FROM items i JOIN media_files f ON f.item_id=i.id LEFT JOIN progress p ON p.item_id=i.id AND p.user_id=? WHERE i.series<>'' AND (? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=f.library_id AND a.user_id=?)) GROUP BY f.library_id,i.series ORDER BY i.series,f.library_id LIMIT 1000")?;
        let out=s.query_map(params![u.id,u.admin,u.id],|r|Ok(json!({"library_id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"episodes":r.get::<_,i64>(2)?,"seasons":r.get::<_,i64>(3)?,"watched":r.get::<_,i64>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;Ok(Value::Array(out))}).await?))
}
async fn scan_status(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
) -> Result<Json<Value>> {
    admin(u)?;
    Ok(Json(json!({"running":app.scans.available_permits()==0})))
}
async fn scan_start(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
) -> Result<impl axum::response::IntoResponse> {
    admin(u)?;
    scan::start(app).await?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({"status":"started"})),
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Metadata {
    title: String,
    overview: String,
}
async fn edit_item(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    admin(u)?;
    media::item(&app, u, id).await?;
    let b: Metadata = parse(&body)?;
    if b.title.is_empty() {
        return Err(Error::new(400, "title required"));
    }
    app.db
        .call(move |c| {
            c.execute(
                "UPDATE items SET title=?,overview=? WHERE id=?",
                params![b.title, b.overview, id],
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Progress {
    position: f64,
    watched: bool,
}
async fn progress(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    media::item(&app, u, id).await?;
    let b: Progress = parse(&body)?;
    if !b.position.is_finite() || b.position < 0.0 {
        return Err(Error::new(400, "invalid position"));
    }
    app.db.call(move|c|{c.execute("INSERT INTO progress VALUES(?,?,?,?,?) ON CONFLICT(user_id,item_id) DO UPDATE SET position=excluded.position,watched=excluded.watched,updated=excluded.updated",params![u.id,id,b.position,b.watched,auth::now()])?;Ok(())}).await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct PlaybackTelemetry {
    event: String,
    duration_ms: f64,
    mode: String,
}
async fn telemetry(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    media::item(&app, u, id).await?;
    let b: PlaybackTelemetry = parse(&body)?;
    if !["first_frame", "seek", "stall"].contains(&b.event.as_str())
        || !["direct", "remux", "transcode"].contains(&b.mode.as_str())
        || !b.duration_ms.is_finite()
        || !(0.0..=86_400_000.0).contains(&b.duration_ms)
    {
        return Err(Error::new(400, "invalid playback measurement"));
    }
    tracing::info!(event=%b.event,duration_ms=b.duration_ms,mode=%b.mode,"client playback measurement (untrusted observation)");
    Ok(Json(json!({"ok":true})))
}
