//! The scan pipeline: walk -> diff -> exact funnel -> decode -> pairs.
//! Every stage saves its results in small batches, so groups can be reviewed while a scan
//! is running and a stopped scan picks up where it left off.
//! The database lock is only held for short reads and batched writes, so the UI can
//! keep querying while a scan runs.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::Result;
use rayon::prelude::*;
use rusqlite::params;
use serde::Serialize;

use crate::{decode, hash, now, Library, D_MAX};

/// Most pairs kept per photo, so flat or blank images cannot flood the pair table.
const MAX_PAIRS_PER_PHOTO: usize = 200;

#[derive(Serialize, Clone, Debug)]
pub struct ScanProgress {
    pub stage: &'static str,
    pub done: u64,
    pub total: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ScanSummary {
    pub files_seen: u64,
    pub added: u64,
    pub changed: u64,
    pub removed: u64,
    /// Not found, but kept because the source is offline or removable.
    pub missing: u64,
    pub hashed: u64,
    pub decoded: u64,
    pub decode_failed: u64,
    pub pairs: u64,
    pub offline_sources: Vec<String>,
    pub cancelled: bool,
    pub seconds: f64,
}

pub type ProgressFn<'a> = &'a (dyn Fn(ScanProgress) + Sync);

/// Folders that are never descended into or modified.
pub fn is_protected(path: &Path) -> bool {
    path.extension()
        .map(|e| {
            let e = e.to_string_lossy().to_lowercase();
            e == "photoslibrary" || e == "aplibrary" || e == "lrdata"
        })
        .unwrap_or(false)
}

struct Found {
    source_id: i64,
    path: String,
    size: i64,
    mtime_ns: i64,
    dev: i64,
    ino: i64,
}

struct Existing {
    id: i64,
    size: i64,
    mtime_ns: i64,
    state: String,
    source_id: i64,
    missing: bool,
}

pub fn scan(lib: &Library, progress: ProgressFn, cancel: &AtomicBool) -> Result<ScanSummary> {
    let started = std::time::Instant::now();
    let mut sum = ScanSummary::default();
    let cancelled = || cancel.load(Ordering::Relaxed);

    // 1. Walk
    let sources = lib.sources()?;
    let mut found: Vec<Found> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut scanned_sources: Vec<i64> = Vec::new();
    for src in &sources {
        if !src.online {
            sum.offline_sources.push(src.root.clone());
            continue;
        }
        scanned_sources.push(src.id);
        let walker = walkdir::WalkDir::new(&src.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let hidden = e.file_name().to_string_lossy().starts_with('.');
                !hidden && !(e.file_type().is_dir() && is_protected(e.path()))
            });
        for entry in walker.flatten() {
            if cancelled() {
                break;
            }
            if !entry.file_type().is_file() || !decode::is_media(entry.path()) {
                continue;
            }
            let Ok(md) = entry.metadata() else { continue };
            if md.len() == 0 {
                continue;
            }
            let path = entry.path().to_string_lossy().to_string();
            if !seen.insert(path.clone()) {
                continue; // overlapping sources
            }
            found.push(Found {
                source_id: src.id,
                path,
                size: md.len() as i64,
                mtime_ns: md.mtime() * 1_000_000_000 + md.mtime_nsec(),
                dev: md.dev() as i64,
                ino: md.ino() as i64,
            });
            if found.len().is_multiple_of(500) {
                progress(ScanProgress {
                    stage: "Finding files",
                    done: found.len() as u64,
                    total: 0,
                });
            }
        }
    }
    sum.files_seen = found.len() as u64;
    progress(ScanProgress {
        stage: "Finding files",
        done: sum.files_seen,
        total: sum.files_seen,
    });
    if cancelled() {
        return finish(sum, true, started);
    }

    // 2. Diff against the index
    let mut dead_thumbs: Vec<i64> = Vec::new();
    {
        let mut conn = lib.db();
        let mut existing: HashMap<String, Existing> = HashMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT id, path, size, mtime_ns, state, source_id, missing FROM photo")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    Existing {
                        id: r.get(0)?,
                        size: r.get(2)?,
                        mtime_ns: r.get(3)?,
                        state: r.get(4)?,
                        source_id: r.get(5)?,
                        missing: r.get(6)?,
                    },
                ))
            })?;
            for row in rows {
                let (path, ex) = row?;
                existing.insert(path, ex);
            }
        }
        let removable: HashSet<i64> = sources
            .iter()
            .filter(|s| s.removable)
            .map(|s| s.id)
            .collect();
        let tx = conn.transaction()?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO photo (source_id, path, size, mtime_ns, dev, ino) VALUES (?1,?2,?3,?4,?5,?6)",
            )?;
            let mut reset = tx.prepare(
                "UPDATE photo SET source_id=?2, size=?3, mtime_ns=?4, dev=?5, ino=?6, state='present',
                        partial_hash=NULL, content_hash=NULL, format=NULL, width=NULL, height=NULL,
                        taken_at=NULL, phash=NULL, dhash=NULL, decoded=0, decode_error=NULL, missing=0, paired=0
                 WHERE id=?1",
            )?;
            let mut touch = tx.prepare(
                "UPDATE photo SET state='present', missing=0, dev=?2, ino=?3 WHERE id=?1",
            )?;
            let mut drop_pairs = tx.prepare("DELETE FROM similar_pair WHERE a_id=?1 OR b_id=?1")?;
            for f in &found {
                match existing.remove(&f.path) {
                    None => {
                        insert.execute(params![
                            f.source_id,
                            f.path,
                            f.size,
                            f.mtime_ns,
                            f.dev,
                            f.ino
                        ])?;
                        sum.added += 1;
                    }
                    Some(ex) if ex.size == f.size && ex.mtime_ns == f.mtime_ns => {
                        touch.execute(params![ex.id, f.dev, f.ino])?;
                    }
                    Some(ex) => {
                        reset.execute(params![
                            ex.id,
                            f.source_id,
                            f.size,
                            f.mtime_ns,
                            f.dev,
                            f.ino
                        ])?;
                        drop_pairs.execute([ex.id])?;
                        sum.changed += 1;
                    }
                }
            }
            // Whatever is left was not found on disk. Trashed/moved rows stay for undo.
            // Files of an offline or removable source may come back, so they are only flagged.
            let mut delete = tx.prepare("DELETE FROM photo WHERE id=?1")?;
            let mut flag = tx.prepare("UPDATE photo SET missing=1 WHERE id=?1")?;
            for ex in existing.values() {
                if ex.state != "present" {
                    continue;
                }
                if !scanned_sources.contains(&ex.source_id) || removable.contains(&ex.source_id) {
                    if !ex.missing {
                        flag.execute([ex.id])?;
                    }
                    sum.missing += 1;
                } else {
                    delete.execute([ex.id])?;
                    dead_thumbs.push(ex.id);
                    sum.removed += 1;
                }
            }
        }
        tx.commit()?;
    }
    for id in dead_thumbs {
        let _ = std::fs::remove_file(lib.thumb_path(id));
    }
    drop(found);

    // 3. Exact funnel
    sum.hashed = exact_funnel(lib, progress, cancel)?;
    if cancelled() {
        return finish(sum, true, started);
    }

    // 4. Decode
    let (ok, failed) = decode_stage(lib, progress, cancel)?;
    sum.decoded = ok;
    sum.decode_failed = failed;
    if cancelled() {
        return finish(sum, true, started);
    }

    // 5. Pairs
    match pair_stage(lib, progress, cancel)? {
        Some(n) => sum.pairs = n,
        None => return finish(sum, true, started),
    }

    {
        let conn = lib.db();
        for id in &scanned_sources {
            conn.execute(
                "UPDATE source SET last_scan_at=?2 WHERE id=?1",
                params![id, now()],
            )?;
        }
    }
    finish(sum, false, started)
}

fn finish(
    mut sum: ScanSummary,
    cancelled: bool,
    started: std::time::Instant,
) -> Result<ScanSummary> {
    sum.cancelled = cancelled;
    sum.seconds = started.elapsed().as_secs_f64();
    Ok(sum)
}

struct HashRow {
    id: i64,
    path: String,
    size: i64,
    partial: Option<Vec<u8>>,
    content: Option<Vec<u8>>,
    missing: bool,
}

/// Files examined per database write in the exact funnel. Small enough that results
/// show up, and survive a stopped scan, within seconds.
const FUNNEL_BATCH: usize = 256;

/// size -> partial hash -> full hash. Each step only runs on files that still collide.
/// Files can only match others of the same size, so same-size sets are worked through
/// in batches and saved as they finish: exact groups appear while the scan is still
/// running, and a scan that is stopped resumes where it left off.
fn exact_funnel(lib: &Library, progress: ProgressFn, cancel: &AtomicBool) -> Result<u64> {
    let mut rows: Vec<HashRow> = {
        let conn = lib.db();
        let mut stmt = conn.prepare(
            "SELECT id, path, size, partial_hash, content_hash, missing FROM photo WHERE state='present'",
        )?;
        let v = stmt
            .query_map([], |r| {
                Ok(HashRow {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    size: r.get(2)?,
                    partial: r.get(3)?,
                    content: r.get(4)?,
                    missing: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        v
    };

    let mut by_size: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        by_size.entry(r.size).or_default().push(i);
    }
    // Smallest files first: they are the quickest to settle.
    let mut sets: Vec<Vec<usize>> = by_size.into_values().filter(|g| g.len() > 1).collect();
    sets.sort_by_key(|g| (rows[g[0]].size, rows[g[0]].id));
    // Sets with nothing left to read cost nothing and are not counted as work.
    sets.retain(|g| {
        g.iter()
            .any(|&i| !rows[i].missing && rows[i].content.is_none())
    });
    let total: u64 = sets.iter().map(|g| g.len() as u64).sum();
    let small_limit = (2 * hash::PARTIAL_CHUNK) as i64;
    let mut done = 0u64;
    let mut changed = 0u64;
    if total > 0 {
        progress(ScanProgress {
            stage: "Comparing files",
            done: 0,
            total,
        });
    }

    let mut next = 0;
    while next < sets.len() && !cancel.load(Ordering::Relaxed) {
        let mut batch: Vec<usize> = Vec::new();
        while next < sets.len()
            && (batch.is_empty() || batch.len() + sets[next].len() <= FUNNEL_BATCH)
        {
            batch.extend(&sets[next]);
            next += 1;
        }

        // Partial hashes for every file in the batch that lacks one.
        let partials: Vec<(usize, Option<[u8; 32]>)> = batch
            .par_iter()
            .filter(|&&i| rows[i].partial.is_none() && !rows[i].missing)
            .map(|&i| {
                if cancel.load(Ordering::Relaxed) {
                    return (i, None);
                }
                let r = &rows[i];
                let p = Path::new(&r.path);
                let h = if r.size <= small_limit {
                    hash::full_hash(p).ok()
                } else {
                    hash::partial_hash(p, r.size as u64).ok()
                };
                (i, h)
            })
            .collect();
        let mut dirty: HashSet<usize> = HashSet::new();
        for (i, h) in partials {
            if let Some(h) = h {
                rows[i].partial = Some(h.to_vec());
                if rows[i].size <= small_limit {
                    rows[i].content = Some(h.to_vec()); // small files: the partial hash is the full hash
                }
                dirty.insert(i);
            }
        }

        // Full hashes where size and partial hash both collide.
        let mut by_partial: HashMap<(i64, Vec<u8>), Vec<usize>> = HashMap::new();
        for &i in &batch {
            if let Some(p) = &rows[i].partial {
                by_partial
                    .entry((rows[i].size, p.clone()))
                    .or_default()
                    .push(i);
            }
        }
        let need_full: Vec<usize> = by_partial
            .into_values()
            .filter(|g| g.len() > 1)
            .flatten()
            .filter(|&i| rows[i].content.is_none() && !rows[i].missing)
            .collect();
        let fulls: Vec<(usize, Option<[u8; 32]>)> = need_full
            .par_iter()
            .map(|&i| {
                if cancel.load(Ordering::Relaxed) {
                    return (i, None);
                }
                (i, hash::full_hash(Path::new(&rows[i].path)).ok())
            })
            .collect();
        for (i, h) in fulls {
            if let Some(h) = h {
                rows[i].content = Some(h.to_vec());
                dirty.insert(i);
            }
        }

        if !dirty.is_empty() {
            let mut conn = lib.db();
            let tx = conn.transaction()?;
            {
                let mut up =
                    tx.prepare("UPDATE photo SET partial_hash=?2, content_hash=?3 WHERE id=?1")?;
                for &i in &dirty {
                    let r = &rows[i];
                    up.execute(params![r.id, r.partial, r.content])?;
                }
            }
            tx.commit()?;
            changed += dirty.len() as u64;
        }
        done += batch.len() as u64;
        progress(ScanProgress {
            stage: "Comparing files",
            done,
            total,
        });
    }
    Ok(changed)
}

/// Similar pairs are refreshed after this many decode chunks (64 photos each).
const PAIR_EVERY_CHUNKS: u32 = 8;

enum DecodeOutcome {
    Ok(decode::Decoded),
    Failed(String),
}

fn decode_stage(lib: &Library, progress: ProgressFn, cancel: &AtomicBool) -> Result<(u64, u64)> {
    let todo: Vec<(i64, String)> = {
        let conn = lib.db();
        let mut stmt = conn.prepare(
            "SELECT id, path FROM photo WHERE state='present' AND decoded=0 AND missing=0",
        )?;
        let v: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        v.into_iter()
            .filter(|(_, p)| decode::is_image(Path::new(p)))
            .collect()
    };
    let total = todo.len() as u64;
    let scratch: PathBuf = lib.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch)?;
    let (mut ok, mut failed, mut done) = (0u64, 0u64, 0u64);
    let mut chunks_done = 0u32;
    progress(ScanProgress {
        stage: "Analysing photos",
        done: 0,
        total,
    });

    for chunk in todo.chunks(64) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let results: Vec<(i64, DecodeOutcome)> = chunk
            .par_iter()
            .map(|(id, path)| {
                let thumb = lib.thumb_path(*id);
                // A broken file must never take the scan down with it.
                let res =
                    std::panic::catch_unwind(|| decode::process(Path::new(path), &thumb, &scratch));
                let outcome = match res {
                    Ok(Ok(d)) => DecodeOutcome::Ok(d),
                    Ok(Err(e)) => DecodeOutcome::Failed(format!("{e:#}")),
                    Err(_) => DecodeOutcome::Failed("decoder crashed".into()),
                };
                (*id, outcome)
            })
            .collect();

        // Scoped: the lock must be released before pairing and before progress is reported.
        {
            let mut conn = lib.db();
            let tx = conn.transaction()?;
            {
                let mut good = tx.prepare(
                    "UPDATE photo SET decoded=1, format=?2, width=?3, height=?4, taken_at=?5,
                        phash=?6, dhash=?7, decode_error=NULL WHERE id=?1",
                )?;
                let mut bad =
                    tx.prepare("UPDATE photo SET decoded=2, decode_error=?2 WHERE id=?1")?;
                for (id, outcome) in &results {
                    match outcome {
                        DecodeOutcome::Ok(d) => {
                            good.execute(params![
                                id,
                                d.format,
                                d.width,
                                d.height,
                                d.taken_at,
                                d.phash as i64,
                                d.dhash as i64
                            ])?;
                            ok += 1;
                        }
                        DecodeOutcome::Failed(msg) => {
                            bad.execute(params![id, msg])?;
                            failed += 1;
                        }
                    }
                }
            }
            tx.commit()?;
        }
        done += chunk.len() as u64;
        // Compare what has been analysed so far, so similar groups fill in during the scan.
        chunks_done += 1;
        if chunks_done.is_multiple_of(PAIR_EVERY_CHUNKS) {
            pair_stage(lib, &|_| {}, cancel)?;
        }
        progress(ScanProgress {
            stage: "Analysing photos",
            done,
            total,
        });
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok((ok, failed))
}

/// Finds pairs within `D_MAX` for photos that have not been compared yet, against every
/// analysed photo. Photos compared in an earlier scan are not compared with each other
/// again. Returns the number of stored pairs, or `None` if cancelled.
fn pair_stage(lib: &Library, progress: ProgressFn, cancel: &AtomicBool) -> Result<Option<u64>> {
    let hashes: Vec<(i64, u64, u64, bool)> = {
        let conn = lib.db();
        let mut stmt = conn.prepare(
            "SELECT id, phash, dhash, paired FROM photo WHERE state='present' AND decoded=1 ORDER BY id",
        )?;
        let v = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, i64>(2)? as u64,
                    r.get(3)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        v
    };
    let fresh: Vec<usize> = (0..hashes.len()).filter(|&i| !hashes[i].3).collect();
    let total = fresh.len() as u64;
    let counter = AtomicU64::new(0);
    if total > 0 {
        progress(ScanProgress {
            stage: "Finding similar photos",
            done: 0,
            total,
        });
    }
    let pairs: Vec<(i64, i64, u32)> = fresh
        .par_iter()
        .flat_map_iter(|&i| {
            let mut near: Vec<(i64, i64, u32)> = Vec::new();
            if !cancel.load(Ordering::Relaxed) {
                let (a, pa, da, _) = hashes[i];
                for (j, &(b, pb, db, seen)) in hashes.iter().enumerate() {
                    // Two fresh photos meet once, from the lower id.
                    if j == i || (!seen && j < i) {
                        continue;
                    }
                    let d = hash::distance(pa, da, pb, db);
                    if d <= D_MAX {
                        near.push((a.min(b), a.max(b), d));
                    }
                }
                if near.len() > MAX_PAIRS_PER_PHOTO {
                    near.sort_by_key(|p| p.2);
                    near.truncate(MAX_PAIRS_PER_PHOTO);
                }
            }
            let c = counter.fetch_add(1, Ordering::Relaxed) + 1;
            if c.is_multiple_of(2000) {
                progress(ScanProgress {
                    stage: "Finding similar photos",
                    done: c,
                    total,
                });
            }
            near
        })
        .collect();
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }

    let mut conn = lib.db();
    let tx = conn.transaction()?;
    {
        let mut ins = tx.prepare(
            "INSERT OR REPLACE INTO similar_pair (a_id, b_id, distance) VALUES (?1,?2,?3)",
        )?;
        for (a, b, d) in &pairs {
            ins.execute(params![a, b, d])?;
        }
        let mut mark = tx.prepare("UPDATE photo SET paired=1 WHERE id=?1")?;
        for &i in &fresh {
            mark.execute([hashes[i].0])?;
        }
    }
    let stored: i64 = tx.query_row("SELECT COUNT(*) FROM similar_pair", [], |r| r.get(0))?;
    tx.commit()?;
    drop(conn);
    if total > 0 {
        progress(ScanProgress {
            stage: "Finding similar photos",
            done: total,
            total,
        });
    }
    Ok(Some(stored as u64))
}
