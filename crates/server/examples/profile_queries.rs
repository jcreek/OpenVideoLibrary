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
    ovl_server::search::init(&c)?;
    let mut results = vec![];
    for query in ["", "Movie", "00099999", "NoMatches"] {
        let q = HashMap::from([
            ("q".to_owned(), query.to_owned()),
            ("offset".into(), "900".into()),
        ]);
        let planning = Instant::now();
        let (sql, args) = api::item_query(&c, User { id: 1, admin: true }, &q)?;
        let planning_ms = planning.elapsed().as_secs_f64() * 1000.0;
        let indexed = sql.replace(
            "FROM items i JOIN",
            "FROM items i INDEXED BY items_title JOIN",
        );
        c.execute_batch("CREATE TEMP TABLE IF NOT EXISTS profile_search_cache(item_id INTEGER PRIMARY KEY); DELETE FROM profile_search_cache;")?;
        let filling = Instant::now();
        if !query.is_empty() {
            let expression = format!("\"{}\"*", query.replace('"', "\"\""));
            c.execute("INSERT INTO profile_search_cache SELECT rowid FROM search WHERE search MATCH ? LIMIT 100001",[expression])?;
        }
        let cache_fill_ms = filling.elapsed().as_secs_f64() * 1000.0;
        let mut variants=vec![
            ("production",sql.clone()),
            ("bounded-cache-prototype",indexed.replace("i.id IN(SELECT rowid FROM search WHERE search MATCH ?)","EXISTS(SELECT 1 FROM temp.profile_search_cache pc WHERE pc.item_id=i.id AND ? IS NOT NULL)")),
        ];
        if std::env::args().any(|v| v == "--experiments") {
            variants.push((
                "title-index-correlated-fts",
                indexed.replace(
                    "i.id IN(SELECT rowid FROM search WHERE search MATCH ?)",
                    "EXISTS(SELECT 1 FROM search WHERE rowid=i.id AND search MATCH ?)",
                ),
            ));
        }
        let mut expected = None;
        for (label, sql) in variants {
            let mut timings = vec![];
            let mut planning_times = vec![];
            let mut total_times = vec![];
            let mut ids = vec![];
            let mut executed_sql = sql.clone();
            let mut executed_args = args.clone();
            for _ in 0..7 {
                let total = Instant::now();
                let planning = Instant::now();
                if label == "production" {
                    (executed_sql, executed_args) =
                        api::item_query(&c, User { id: 1, admin: true }, &q)?;
                }
                planning_times.push(planning.elapsed().as_secs_f64() * 1000.0);
                let start = Instant::now();
                ids = c
                    .prepare(&executed_sql)?
                    .query_map(params_from_iter(&executed_args), |r| r.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                timings.push(start.elapsed().as_secs_f64() * 1000.0);
                total_times.push(total.elapsed().as_secs_f64() * 1000.0);
            }
            let plan = c
                .prepare(&format!("EXPLAIN QUERY PLAN {executed_sql}"))?
                .query_map(params_from_iter(&executed_args), |r| r.get::<_, String>(3))?
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(expected) = &expected {
                assert_eq!(&ids, expected, "profile variant changed results");
            } else {
                expected = Some(ids.clone());
            }
            results.push(json!({"variant":label,"query":query,"offset":900,"planning_ms":planning_ms,"cache_fill_ms":cache_fill_ms,"plan":plan,"milliseconds":timings,"planning_milliseconds":planning_times,"total_milliseconds":total_times,"ids":ids}));
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
