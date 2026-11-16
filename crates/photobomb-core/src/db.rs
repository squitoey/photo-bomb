use anyhow::Result;
use rusqlite::{Connection, Row};

use crate::Photo;

pub fn init(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;

         CREATE TABLE IF NOT EXISTS source (
           id            INTEGER PRIMARY KEY,
           root          TEXT NOT NULL UNIQUE,
           last_scan_at  INTEGER
         );

         CREATE TABLE IF NOT EXISTS photo (
           id            INTEGER PRIMARY KEY,
           source_id     INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
           path          TEXT NOT NULL UNIQUE,
           size          INTEGER NOT NULL,
           mtime_ns      INTEGER NOT NULL,
           dev           INTEGER NOT NULL,
           ino           INTEGER NOT NULL,
           state         TEXT NOT NULL DEFAULT 'present',
           partial_hash  BLOB,
           content_hash  BLOB,
           format        TEXT,
           width         INTEGER,
           height        INTEGER,
           taken_at      INTEGER,
           phash         INTEGER,
           dhash         INTEGER,
           decoded       INTEGER NOT NULL DEFAULT 0,
           decode_error  TEXT
         );
         CREATE INDEX IF NOT EXISTS photo_size    ON photo(size);
         CREATE INDEX IF NOT EXISTS photo_content ON photo(content_hash) WHERE content_hash IS NOT NULL;

         CREATE TABLE IF NOT EXISTS similar_pair (
           a_id      INTEGER NOT NULL REFERENCES photo(id) ON DELETE CASCADE,
           b_id      INTEGER NOT NULL REFERENCES photo(id) ON DELETE CASCADE,
           distance  INTEGER NOT NULL,
           PRIMARY KEY (a_id, b_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS similar_pair_b ON similar_pair(b_id);

         CREATE TABLE IF NOT EXISTS ignored_pair (
           hash_a      BLOB NOT NULL,
           hash_b      BLOB NOT NULL,
           created_at  INTEGER NOT NULL,
           PRIMARY KEY (hash_a, hash_b)
         ) WITHOUT ROWID;

         CREATE TABLE IF NOT EXISTS journal (
           id          INTEGER PRIMARY KEY,
           batch_id    INTEGER NOT NULL,
           created_at  INTEGER NOT NULL,
           op          TEXT NOT NULL,
           photo_id    INTEGER,
           from_path   TEXT NOT NULL,
           to_path     TEXT,
           size        INTEGER NOT NULL DEFAULT 0,
           status      TEXT NOT NULL,
           error       TEXT
         );
         CREATE INDEX IF NOT EXISTS journal_batch ON journal(batch_id);",
    )?;
    // Columns added after the first release of the schema.
    add_column(conn, "source", "removable", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(conn, "photo", "missing", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(conn, "photo", "not_dup", "INTEGER NOT NULL DEFAULT 0")?;
    // 1 once the photo has been compared against the rest for similar pairs.
    add_column(conn, "photo", "paired", "INTEGER NOT NULL DEFAULT 0")?;
    Ok(())
}

fn add_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let exists = conn
        .prepare(&format!(
            "SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"
        ))?
        .exists([column])?;
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

pub const PHOTO_COLS: &str =
    "id, source_id, path, size, mtime_ns, dev, ino, content_hash, format, \
                              width, height, taken_at, decoded, decode_error, missing, not_dup";

pub fn photo_from_row(r: &Row) -> rusqlite::Result<Photo> {
    let hash: Option<Vec<u8>> = r.get(7)?;
    Ok(Photo {
        id: r.get(0)?,
        source_id: r.get(1)?,
        path: r.get(2)?,
        size: r.get(3)?,
        mtime_ns: r.get(4)?,
        dev: r.get(5)?,
        ino: r.get(6)?,
        content_hash: hash.map(hex::encode),
        format: r.get(8)?,
        width: r.get(9)?,
        height: r.get(10)?,
        taken_at: r.get(11)?,
        decoded: r.get(12)?,
        decode_error: r.get(13)?,
        missing: r.get(14)?,
        not_dup: r.get(15)?,
    })
}

/// `clause` is trusted, internal SQL (never user input).
pub fn photos_where(conn: &Connection, clause: &str) -> Result<Vec<Photo>> {
    let mut stmt = conn.prepare(&format!("SELECT {PHOTO_COLS} FROM photo WHERE {clause}"))?;
    let rows = stmt
        .query_map([], photo_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn photos_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<Photo>> {
    let mut out = Vec::with_capacity(ids.len());
    let mut stmt = conn.prepare(&format!("SELECT {PHOTO_COLS} FROM photo WHERE id = ?1"))?;
    for id in ids {
        let mut rows = stmt.query_map([id], photo_from_row)?;
        if let Some(p) = rows.next() {
            out.push(p?);
        }
    }
    Ok(out)
}
