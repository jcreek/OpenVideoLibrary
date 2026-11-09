use crate::{
    App,
    error::{Error, Result},
};
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct User {
    #[serde(rename = "ID")]
    pub id: i64,
    pub admin: bool,
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn digest(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
pub fn token() -> Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|_| Error::new(500, "randomness unavailable"))?;
    Ok(b.iter().map(|v| format!("{v:02x}")).collect())
}
pub fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| Error::new(400, "invalid request"))
}
pub fn request_header(headers: &HeaderMap) -> Result<()> {
    if headers.get("X-OVL-Request").and_then(|h| h.to_str().ok()) != Some("1") {
        return Err(Error::new(403, "missing request header"));
    }
    Ok(())
}
pub fn session_token(headers: &HeaderMap) -> String {
    if let Some(b) = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    {
        return b.to_owned();
    }
    headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .find_map(|v| v.trim().strip_prefix("ovl_session="))
        .unwrap_or("")
        .to_owned()
}
pub async fn authenticate(
    State(app): State<Arc<App>>,
    mut req: Request,
    next: Next,
) -> Result<Response> {
    let t = digest(&session_token(req.headers()));
    let user=app.db.call(move|c| Ok(c.query_row("SELECT u.id,u.admin FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token=? AND s.expires>?",params![t,now()],|r|Ok(User{id:r.get(0)?,admin:r.get(1)?})).optional()?)).await?.ok_or_else(||Error::new(401,"authentication required"))?;
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let bearer = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("Bearer "));
        if !bearer {
            request_header(req.headers())?;
        }
    }
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}
pub fn admin(u: User) -> Result<()> {
    if !u.admin {
        return Err(Error::new(403, "administrator required"));
    }
    Ok(())
}
pub fn allowed(c: &rusqlite::Connection, u: User, library: i64) -> Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM libraries l WHERE l.id=? AND (? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=l.id AND a.user_id=?)))",params![library,u.admin,u.id],|r|r.get(0))?)
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Credentials {
    pub name: String,
    pub password: String,
    #[serde(default)]
    pub libraries: Vec<i64>,
}
fn validate(name: &str, password: &str) -> Result<()> {
    if name.is_empty() || name.len() > 100 || !(12..=72).contains(&password.len()) {
        return Err(Error::new(
            400,
            "name required; password must be 12–72 bytes",
        ));
    }
    Ok(())
}
pub async fn create(app: Arc<App>, b: Credentials, is_admin: bool) -> Result<Value> {
    validate(&b.name, &b.password)?;
    if b.libraries.len() > 1000 {
        return Err(Error::new(400, "too many libraries"));
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
            if is_admin
                && tx.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))? != 0
            {
                return Err(Error::new(409, "already configured"));
            }
            tx.execute(
                "INSERT INTO users(name,password,admin) VALUES(?,?,?)",
                params![b.name, hash.as_bytes(), is_admin],
            )
            .map_err(|_| Error::new(409, "account could not be created"))?;
            let id = tx.last_insert_rowid();
            for l in b.libraries {
                tx.execute("INSERT INTO access VALUES(?,?)", params![id, l])
                    .map_err(|_| Error::new(400, "invalid or duplicate library"))?;
            }
            tx.commit()?;
            Ok(json!({"id":id}))
        })
        .await
}
pub async fn setup(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    request_header(&headers)?;
    let _guard = app.setup.lock().await;
    let b = parse(&body)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(create(app.clone(), b, true).await?),
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct LoginCredentials {
    name: String,
    password: String,
}
pub async fn login(
    State(app): State<Arc<App>>,
    connection: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    request_header(&headers)?;
    let ip = connection
        .map(|Extension(ConnectInfo(s))| s.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    {
        let mut rates = app.rates.lock().await;
        let now = Instant::now();
        rates.retain(|_, (_, until)| *until > now);
        if !rates.contains_key(&ip) && rates.len() >= 4096 {
            return Err(Error::new(429, "authentication busy"));
        }
        let b = rates
            .entry(ip)
            .or_insert((0, now + Duration::from_secs(60)));
        if b.0 >= 20 {
            return Err(Error::new(429, "too many authentication attempts"));
        }
        b.0 += 1;
    }
    let b: LoginCredentials = parse(&body)?;
    if b.password.len() > 72 {
        return Err(Error::new(401, "invalid credentials"));
    }
    let permit = app
        .hashing
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(429, "authentication busy"))?;
    let account = app
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT id,password FROM users WHERE name=?",
                [b.name],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?)
        })
        .await?;
    let (id, hash) = account.ok_or_else(|| Error::new(401, "invalid credentials"))?;
    let valid = app
        .tasks
        .spawn_blocking(move || {
            let _permit = permit;
            bcrypt::verify(b.password, &String::from_utf8_lossy(&hash)).unwrap_or(false)
        })
        .await
        .map_err(|_| Error::new(500, "authentication failed"))?;
    if !valid {
        return Err(Error::new(401, "invalid credentials"));
    }
    let token = token()?;
    let hashed = digest(&token);
    let expires = now() + 86400;
    app.db
        .call(move |c| {
            c.execute(
                "INSERT INTO sessions VALUES(?,?,?)",
                params![hashed, id, expires],
            )?;
            Ok(())
        })
        .await?;
    let secure = if app.config.secure_cookies {
        "; Secure"
    } else {
        ""
    };
    let cookie =
        format!("ovl_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400{secure}");
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({"token":token}))).into_response())
}
pub async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Result<impl IntoResponse> {
    let hashed = digest(&session_token(&headers));
    app.db
        .call(move |c| {
            c.execute("DELETE FROM sessions WHERE token=?", [hashed])?;
            Ok(())
        })
        .await?;
    Ok((
        [(
            header::SET_COOKIE,
            "ovl_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict",
        )],
        Json(json!({"ok":true})),
    ))
}
