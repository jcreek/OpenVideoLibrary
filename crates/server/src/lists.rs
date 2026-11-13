//! Personal collections, ordered playlists and persistent queues. Every read
//! filters entries against current grants; writes validate every media path.
use crate::{
    App,
    auth::{self, User, parse},
    error::{Error, Result},
    media,
};
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Write {
    kind: String,
    name: String,
    #[serde(rename = "ItemIDs")]
    item_ids: Vec<i64>,
    #[serde(default)]
    revision: i64,
    #[serde(default)]
    cursor: usize,
}
async fn validate(app: &Arc<App>, u: User, b: &Write) -> Result<Vec<i64>> {
    if !["collection", "playlist", "queue"].contains(&b.kind.as_str())
        || b.name.trim().is_empty()
        || b.name.chars().count() > 200
        || b.item_ids.len() > 1000
        || b.cursor > b.item_ids.len()
    {
        return Err(Error::new(400, "invalid list"));
    }
    if b.kind == "collection"
        && b.item_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != b.item_ids.len()
    {
        return Err(Error::new(400, "duplicate collection item"));
    }
    let mut libraries = Vec::with_capacity(b.item_ids.len());
    for id in &b.item_ids {
        libraries.push(media::item(app, u, *id).await?.1);
    }
    Ok(libraries)
}
pub async fn all(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
) -> Result<Json<Value>> {
    Ok(Json(app.db.call(move|c|{let mut s=c.prepare("SELECT id,kind,name,revision,cursor FROM ovl_lists WHERE owner_id=? ORDER BY updated DESC,id DESC LIMIT 100")?;
        let out=s.query_map([u.id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,"revision":r.get::<_,i64>(3)?,"cursor":r.get::<_,i64>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;Ok(Value::Array(out))}).await?))
}
pub async fn one(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    let (mut out,entries)=app.db.call(move|c|{
        let out=c.query_row("SELECT id,kind,name,revision,cursor,(SELECT count(*) FROM ovl_list_entries WHERE list_id=ovl_lists.id) FROM ovl_lists WHERE id=? AND owner_id=?",params![id,u.id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,"revision":r.get::<_,i64>(3)?,"cursor":r.get::<_,i64>(4)?,"entry_count":r.get::<_,i64>(5)?}))).optional()?.ok_or_else(||Error::new(404,"list not found"))?;
        let mut s=c.prepare("SELECT i.id,f.library_id,i.title,i.series,i.season,i.episode,i.overview,coalesce(p.position,0),coalesce(p.watched,0),e.position FROM ovl_list_entries e JOIN items i ON i.id=e.item_id JOIN media_files f ON f.id=(SELECT min(f2.id) FROM media_files f2 WHERE f2.item_id=i.id AND (? OR EXISTS(SELECT 1 FROM access a WHERE a.library_id=f2.library_id AND a.user_id=?))) LEFT JOIN progress p ON p.item_id=i.id AND p.user_id=? WHERE e.list_id=? ORDER BY e.position LIMIT 1000")?;
        let entries=s.query_map(params![u.admin,u.id,u.id,id],|r|Ok(json!({"position":r.get::<_,i64>(9)?,"item":crate::api::item_row(r)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;Ok((out,entries))
    }).await?;
    // Re-resolve symlinks, and omit unavailable mounts without changing saved order.
    let mut visible = Vec::new();
    for e in entries {
        if media::item(&app, u, e["item"]["id"].as_i64().expect("integer item ID"))
            .await
            .is_ok()
        {
            visible.push(e);
        }
    }
    out["entries"] = Value::Array(visible);
    Ok(Json(out))
}
pub async fn create(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    let b: Write = parse(&body)?;
    let libraries = validate(&app, u, &b).await?;
    let id = app
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            check_grants(&tx, u, &libraries)?;
            if tx.query_row(
                "SELECT count(*) FROM ovl_lists WHERE owner_id=?",
                [u.id],
                |r| r.get::<_, i64>(0),
            )? >= 100
            {
                return Err(Error::new(409, "list limit reached"));
            }
            tx.execute(
                "INSERT INTO ovl_lists(owner_id,kind,name,cursor,updated) VALUES(?,?,?,?,?)",
                params![u.id, b.kind, b.name, b.cursor as i64, auth::now()],
            )?;
            let id = tx.last_insert_rowid();
            for (pos, item) in b.item_ids.into_iter().enumerate() {
                tx.execute(
                    "INSERT INTO ovl_list_entries VALUES(?,?,?)",
                    params![id, pos as i64, item],
                )?;
            }
            tx.commit()?;
            Ok(id)
        })
        .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"revision":1}))))
}
pub async fn update(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    let b: Write = parse(&body)?;
    let libraries = validate(&app, u, &b).await?;
    let revision=app.db.call(move|c|{let tx=c.transaction()?;
        check_grants(&tx,u,&libraries)?;
        let rev=tx.query_row("SELECT revision FROM ovl_lists WHERE id=? AND owner_id=?",params![id,u.id],|r|r.get::<_,i64>(0)).optional()?.ok_or_else(||Error::new(404,"list not found"))?;
        if b.revision!=rev{return Err(Error::new(409,"list changed; reload before saving"));}
        tx.execute("UPDATE ovl_lists SET kind=?,name=?,cursor=?,revision=revision+1,updated=? WHERE id=?",params![b.kind,b.name,b.cursor as i64,auth::now(),id])?;
        tx.execute("DELETE FROM ovl_list_entries WHERE list_id=?",[id])?;
        for (pos,item) in b.item_ids.into_iter().enumerate(){tx.execute("INSERT INTO ovl_list_entries VALUES(?,?,?)",params![id,pos as i64,item])?;}tx.commit()?;Ok(rev+1)
    }).await?;
    Ok(Json(json!({"id":id,"revision":revision})))
}
pub async fn delete(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    app.db
        .call(move |c| {
            if c.execute(
                "DELETE FROM ovl_lists WHERE id=? AND owner_id=?",
                params![id, u.id],
            )? == 0
            {
                return Err(Error::new(404, "list not found"));
            }
            Ok(())
        })
        .await?;
    Ok(Json(json!({"ok":true})))
}

// Recheck grants in the same serialized transaction as a replacement. A grant
// change may have completed while filesystem validation was in progress.
fn check_grants(c: &rusqlite::Connection, u: User, libraries: &[i64]) -> Result<()> {
    for library in libraries {
        if !auth::allowed(c, u, *library)? {
            return Err(Error::new(404, "media unavailable"));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Cursor {
    revision: i64,
    cursor: usize,
}

/// Cursor changes never replace entries, including entries hidden by offline
/// mounts or revoked permissions. Positions refer to the persisted ordering.
pub async fn cursor(
    State(app): State<Arc<App>>,
    Extension(u): Extension<User>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>> {
    let b: Cursor = parse(&body)?;
    let revision = app.db.call(move |c| {
        let tx = c.transaction()?;
        let (kind, revision, end): (String, i64, i64) = tx.query_row(
            "SELECT kind,revision,(SELECT coalesce(max(position)+1,0) FROM ovl_list_entries WHERE list_id=ovl_lists.id) FROM ovl_lists WHERE id=? AND owner_id=?",
            params![id,u.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional()?.ok_or_else(|| Error::new(404,"list not found"))?;
        if kind != "queue" || b.cursor > end as usize {
            return Err(Error::new(400,"invalid queue cursor"));
        }
        if b.revision != revision {
            return Err(Error::new(409,"list changed; reload before saving"));
        }
        tx.execute("UPDATE ovl_lists SET cursor=?,revision=revision+1,updated=? WHERE id=?",params![b.cursor as i64,auth::now(),id])?;
        tx.commit()?;
        Ok(revision+1)
    }).await?;
    Ok(Json(json!({"id":id,"revision":revision})))
}
