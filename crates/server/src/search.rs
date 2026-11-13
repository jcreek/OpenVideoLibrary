//! Exact broad-FTS membership caching. Main SQLite data and public schemas are
//! untouched. Current authorization, progress, filters and ordering stay in SQL.
use crate::error::Result;
use rusqlite::{Connection, OptionalExtension, params};
const MAX_TERMS: i64 = 8;
const MAX_ROWS: i64 = 200_000;
const MAX_QUERY_ROWS: usize = 100_000;
const BROAD_ROWS: i64 = 2048;

#[derive(Debug)]
pub enum Plan {
    Sparse,
    Broad,
    Cached(i64),
}

pub fn init(c: &Connection) -> Result<()> {
    c.execute_batch("ATTACH DATABASE ':memory:' AS ovl_query_cache;
        PRAGMA ovl_query_cache.page_size=4096;
        PRAGMA ovl_query_cache.max_page_count=4096;
        PRAGMA ovl_query_cache.journal_mode=MEMORY;
        CREATE TABLE ovl_query_cache.queries(id INTEGER PRIMARY KEY,term TEXT UNIQUE NOT NULL,status INTEGER NOT NULL DEFAULT 0,rows INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE ovl_query_cache.entries(query_id INTEGER NOT NULL,item_id INTEGER NOT NULL,PRIMARY KEY(query_id,item_id)) WITHOUT ROWID;
        CREATE TEMP TABLE ovl_search_state(generation INTEGER NOT NULL,cached_generation INTEGER NOT NULL,external_version INTEGER NOT NULL);
        INSERT INTO ovl_search_state VALUES(0,-1,-1);
        CREATE TEMP TRIGGER ovl_search_items_ai AFTER INSERT ON main.items BEGIN UPDATE ovl_search_state SET generation=generation+1; END;
        CREATE TEMP TRIGGER ovl_search_items_ad AFTER DELETE ON main.items BEGIN UPDATE ovl_search_state SET generation=generation+1; END;
        CREATE TEMP TRIGGER ovl_search_items_au AFTER UPDATE OF title,series ON main.items BEGIN UPDATE ovl_search_state SET generation=generation+1; END;")?;
    Ok(())
}
fn evict(c: &Connection, except: i64) -> Result<()> {
    let id: i64 = c.query_row(
        "SELECT id FROM ovl_query_cache.queries WHERE id<>? ORDER BY id LIMIT 1",
        [except],
        |r| r.get(0),
    )?;
    c.execute("DELETE FROM ovl_query_cache.entries WHERE query_id=?", [id])?;
    c.execute("DELETE FROM ovl_query_cache.queries WHERE id=?", [id])?;
    Ok(())
}

pub fn plan(c: &Connection, expression: &str) -> Result<Plan> {
    // Do not reserve cache space for abnormally long request terms. The normal
    // FTS statement still handles them with identical matching semantics.
    if expression.len() > 4096 {
        return Ok(Plan::Sparse);
    }
    let version: i64 = c.query_row("PRAGMA main.data_version", [], |r| r.get(0))?;
    let (generation, cached, external): (i64, i64, i64) = c.query_row(
        "SELECT generation,cached_generation,external_version FROM temp.ovl_search_state",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if generation != cached || version != external {
        c.execute_batch(
            "DELETE FROM ovl_query_cache.entries; DELETE FROM ovl_query_cache.queries;",
        )?;
        c.execute(
            "UPDATE temp.ovl_search_state SET cached_generation=?,external_version=?",
            params![generation, version],
        )?;
    }
    let known = c
        .query_row(
            "SELECT id,status FROM ovl_query_cache.queries WHERE term=?",
            [expression],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;
    if let Some((id, status)) = known {
        if status == 1 {
            return Ok(Plan::Cached(id));
        }
        if status == 2 {
            return Ok(Plan::Broad);
        }
        // Admit only a repeated broad query in an unchanged metadata generation.
        // Scans that add/update titles between requests therefore avoid a costly
        // cache fill that cannot be reused.
        loop {
            let rows: i64 = c.query_row(
                "SELECT coalesce(sum(rows),0) FROM ovl_query_cache.queries",
                [],
                |r| r.get(0),
            )?;
            if rows <= MAX_ROWS - MAX_QUERY_ROWS as i64 {
                break;
            }
            evict(c, id)?;
        }
        c.execute_batch("SAVEPOINT ovl_search_fill")?;
        let fill=c.execute("INSERT INTO ovl_query_cache.entries SELECT ?,rowid FROM main.search WHERE search MATCH ? LIMIT ?",params![id,expression,(MAX_QUERY_ROWS+1) as i64]);
        let rows = match fill {
            Ok(rows) if rows <= MAX_QUERY_ROWS => rows,
            result => {
                // SQLite FULL can roll back the surrounding transaction. In that
                // case the caller uses uncached, single-statement FTS and does
                // not commit an already rolled-back read transaction.
                let _ = c.execute_batch("ROLLBACK TO ovl_search_fill; RELEASE ovl_search_fill");
                if let Err(error) = result
                    && !matches!(
                        error.sqlite_error_code(),
                        Some(rusqlite::ErrorCode::DiskFull | rusqlite::ErrorCode::OutOfMemory)
                    )
                {
                    return Err(error.into());
                }
                c.execute("INSERT INTO ovl_query_cache.queries(id,term,status,rows) VALUES(?,?,2,0) ON CONFLICT(term) DO UPDATE SET status=2,rows=0",params![id,expression])?;
                return Ok(Plan::Broad);
            }
        };
        c.execute_batch("RELEASE ovl_search_fill")?;
        c.execute(
            "UPDATE ovl_query_cache.queries SET status=1,rows=? WHERE id=?",
            params![rows as i64, id],
        )?;
        return Ok(Plan::Cached(id));
    }
    let count: i64 = c.query_row(
        "SELECT count(*) FROM (SELECT rowid FROM main.search WHERE search MATCH ? LIMIT ?)",
        params![expression, BROAD_ROWS + 1],
        |r| r.get(0),
    )?;
    if count <= BROAD_ROWS {
        return Ok(Plan::Sparse);
    }
    while c.query_row("SELECT count(*) FROM ovl_query_cache.queries", [], |r| {
        r.get::<_, i64>(0)
    })? >= MAX_TERMS
    {
        evict(c, -1)?;
    }
    c.execute(
        "INSERT INTO ovl_query_cache.queries(term) VALUES(?)",
        [expression],
    )?;
    Ok(Plan::Broad)
}
