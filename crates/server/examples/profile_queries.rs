//! Profiles the actual production SQL with bundled SQLite and disposable data.
//! Run: cargo run --locked --release --example profile_queries -- 100000
use ovl_server::{api, auth::User};
use rusqlite::{Connection, params_from_iter};
use serde_json::json;
use std::{collections::HashMap, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = std::env::args()
        .nth(1)
        .unwrap_or("100000".into())
        .parse::<i64>()?;
    assert!((1000..=100000).contains(&count));
    let mut c = Connection::open_in_memory()?;
    c.execute_batch(include_str!("../src/schema.sql"))?;
    c.execute_batch(include_str!("../src/migrations/002_media_files.sql"))?;
    c.execute(
        "INSERT INTO libraries VALUES(1,'Synthetic','/synthetic','movie',0)",
        [],
    )?;
    let tx = c.transaction()?;
    for i in 1..=count {
        tx.execute(
            "INSERT INTO items(id,title) VALUES(?,?)",
            rusqlite::params![i, format!("Movie {i:08}")],
        )?;
        tx.execute(
            "INSERT INTO media_files(item_id,library_id,path,mtime,size) VALUES(?,1,?,0,0)",
            rusqlite::params![i, format!("/synthetic/{i}.mp4")],
        )?;
    }
    tx.commit()?;
    let mut results = vec![];
    for query in ["", "Movie", "00099999", "NoMatches"] {
        let q = HashMap::from([
            ("q".to_owned(), query.to_owned()),
            ("offset".into(), "900".into()),
        ]);
        let (sql, args) = api::item_query(User { id: 1, admin: true }, &q);
        let variants = [
            ("production", sql.clone()),
            (
                "title-index",
                sql.replace(
                    "FROM items i JOIN",
                    "FROM items i INDEXED BY items_title JOIN",
                ),
            ),
        ];
        for (label, sql) in variants {
            let plan = c
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
                .query_map(params_from_iter(&args), |r| r.get::<_, String>(3))?
                .collect::<Result<Vec<_>, _>>()?;
            let mut timings = vec![];
            let mut ids = vec![];
            for _ in 0..7 {
                let start = Instant::now();
                ids = c
                    .prepare(&sql)?
                    .query_map(params_from_iter(&args), |r| r.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                timings.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            results.push(json!({"variant":label,"query":query,"offset":900,"plan":plan,"milliseconds":timings,"ids":ids}));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"sqlite_version":rusqlite::version(),"items":count,"kind":"In-memory production SQL profile; not HTTP or N100 acceptance","results":results})
        )?
    );
    Ok(())
}
