//! File operations: trash, move, replace-with-hard-link, and undo.
//! Safety rules live here, not in the UI:
//!  - nothing is ever permanently deleted
//!  - a plan that would remove every copy in a group is blocked unless explicitly allowed
//!  - every file is re-checked against the index right before it is touched
//!  - every operation is journaled before it happens

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{db, fsmeta, hash, now, select, trash, Library, Photo, SimilarOpts};

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Op {
    Trash,
    Move { dest: String },
    Hardlink,
}

impl Op {
    fn name(&self) -> &'static str {
        match self {
            Op::Trash => "trash",
            Op::Move { .. } => "move",
            Op::Hardlink => "hardlink",
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct PlanItem {
    pub photo_id: i64,
    pub path: String,
    pub size: i64,
    /// "ok" | "skip" | "blocked"
    pub status: &'static str,
    pub note: Option<String>,
    /// Hard link: the keeper this file will point at.
    pub target: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Plan {
    pub op: Op,
    pub items: Vec<PlanItem>,
    pub ok: usize,
    pub skipped: usize,
    pub blocked: usize,
    /// Bytes freed (trash, hard link) or relocated (move).
    pub bytes: i64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct BatchResult {
    pub batch_id: i64,
    pub done: usize,
    pub failed: usize,
    pub skipped: usize,
    pub bytes: i64,
    pub errors: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct HistoryEntry {
    pub batch_id: i64,
    pub created_at: i64,
    pub op: String,
    pub files: i64,
    pub done: i64,
    pub failed: i64,
    pub undone: i64,
    pub bytes: i64,
}

/// A journal entry: id, op, photo id, original path, where it went, size.
type JournalRow = (i64, String, Option<i64>, String, Option<String>, i64);

fn stat_matches(p: &Photo) -> Result<(), String> {
    match std::fs::symlink_metadata(&p.path) {
        Err(_) => Err("file is missing; rescan".into()),
        Ok(md) => {
            if md.len() as i64 != p.size || fsmeta::mtime_ns(&md) != p.mtime_ns {
                Err("file changed since the last scan; rescan".into())
            } else {
                Ok(())
            }
        }
    }
}

fn unique_path(wanted: &Path) -> PathBuf {
    if !wanted.exists() {
        return wanted.to_path_buf();
    }
    let stem = wanted
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = wanted
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let dir = wanted.parent().unwrap_or(Path::new("."));
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

/// rename, falling back to copy + remove across volumes.
pub(crate) fn move_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).with_context(|| format!("copy to {}", to.display()))?;
    let (a, b) = (std::fs::metadata(from)?.len(), std::fs::metadata(to)?.len());
    if a != b {
        let _ = std::fs::remove_file(to);
        bail!("copy was incomplete");
    }
    std::fs::remove_file(from)?;
    Ok(())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.photobomb-tmp-{}", std::process::id()))
}

impl Library {
    /// Dry run. Nothing on disk changes.
    pub fn plan(
        &self,
        ids: &[i64],
        op: &Op,
        similar: Option<&SimilarOpts>,
        allow_empty_groups: bool,
    ) -> Result<Plan> {
        let selected: HashSet<i64> = ids.iter().copied().collect();
        let photos = db::photos_by_ids(&self.db(), ids)?;

        // Photos that would leave their group with no survivor.
        let mut emptied: HashSet<i64> = HashSet::new();
        if !matches!(op, Op::Hardlink) && !allow_empty_groups {
            let mut groups = self.exact_groups()?;
            if let Some(opts) = similar {
                groups.extend(self.similar_groups(opts)?);
            }
            for g in groups {
                if !g.members.iter().any(|m| selected.contains(&m.photo.id)) {
                    continue;
                }
                // A survivor only counts if its file is on disk right now: a copy on an
                // unplugged drive cannot be checked.
                let survivor = g.members.iter().any(|m| {
                    !selected.contains(&m.photo.id)
                        && !m.photo.missing
                        && Path::new(&m.photo.path).is_file()
                });
                if !survivor {
                    emptied.extend(g.members.iter().map(|m| m.photo.id));
                }
            }
        }

        // Hard links: siblings by content hash.
        let mut siblings: HashMap<String, Vec<Photo>> = HashMap::new();
        if matches!(op, Op::Hardlink) {
            let conn = self.db();
            for p in db::photos_where(
                &conn,
                "state = 'present' AND missing = 0 AND content_hash IS NOT NULL",
            )? {
                siblings
                    .entry(p.content_hash.clone().unwrap())
                    .or_default()
                    .push(p);
            }
        }

        let present: HashSet<i64> = {
            let conn = self.db();
            let mut stmt = conn.prepare("SELECT state FROM photo WHERE id = ?1")?;
            photos
                .iter()
                .filter(|p| {
                    stmt.query_row([p.id], |r| r.get::<_, String>(0))
                        .map(|s| s == "present")
                        .unwrap_or(false)
                })
                .map(|p| p.id)
                .collect()
        };

        let mut items = Vec::new();
        for p in &photos {
            let mut item = PlanItem {
                photo_id: p.id,
                path: p.path.clone(),
                size: p.size,
                status: "ok",
                note: None,
                target: None,
            };
            let skip = |item: &mut PlanItem, why: &str| {
                item.status = "skip";
                item.note = Some(why.to_string());
            };
            if !present.contains(&p.id) {
                skip(&mut item, "no longer in the library");
            } else if let Err(why) = stat_matches(p) {
                skip(&mut item, &why);
            } else if emptied.contains(&p.id) {
                item.status = "blocked";
                item.note =
                    Some("no copy that is available right now would be kept in this group".into());
            } else if matches!(op, Op::Hardlink) {
                match &p.content_hash {
                    None => skip(&mut item, "not an exact duplicate"),
                    Some(h) => {
                        let mut keepers: Vec<&Photo> = siblings
                            .get(h)
                            .map(|v| v.iter().filter(|s| !selected.contains(&s.id)).collect())
                            .unwrap_or_default();
                        keepers.sort_by(|a, b| select::cmp_exact(a, b));
                        if keepers.is_empty() {
                            skip(&mut item, "no copy left unselected to link to");
                        } else if keepers.iter().any(|k| k.dev == p.dev && k.ino == p.ino) {
                            skip(&mut item, "already hard-linked");
                        } else if let Some(k) = keepers.iter().find(|k| k.dev == p.dev) {
                            item.target = Some(k.path.clone());
                        } else {
                            skip(&mut item, "the kept copy is on a different volume");
                        }
                    }
                }
            }
            items.push(item);
        }
        let ok = items.iter().filter(|i| i.status == "ok").count();
        Ok(Plan {
            op: op.clone(),
            ok,
            skipped: items.iter().filter(|i| i.status == "skip").count(),
            blocked: items.iter().filter(|i| i.status == "blocked").count(),
            bytes: items
                .iter()
                .filter(|i| i.status == "ok")
                .map(|i| i.size)
                .sum(),
            items,
        })
    }

    /// Plans and then performs the operation. Every step is journaled and can be undone.
    pub fn execute(
        &self,
        ids: &[i64],
        op: &Op,
        similar: Option<&SimilarOpts>,
        allow_empty_groups: bool,
    ) -> Result<BatchResult> {
        let plan = self.plan(ids, op, similar, allow_empty_groups)?;
        if plan.blocked > 0 {
            bail!(
                "{} file(s) would leave a group with nothing kept. Unselect one per group, or allow it explicitly.",
                plan.blocked
            );
        }
        if let Op::Move { dest } = op {
            std::fs::create_dir_all(dest).with_context(|| format!("cannot create {dest}"))?;
        }
        let source_roots: HashMap<i64, String> = self
            .sources()?
            .into_iter()
            .map(|s| (s.id, s.root))
            .collect();
        let photos: HashMap<i64, Photo> = db::photos_by_ids(&self.db(), ids)?
            .into_iter()
            .map(|p| (p.id, p))
            .collect();

        let batch_id: i64 = self.db().query_row(
            "SELECT COALESCE(MAX(batch_id), 0) + 1 FROM journal",
            [],
            |r| r.get(0),
        )?;
        let mut result = BatchResult {
            batch_id,
            skipped: plan.skipped,
            ..Default::default()
        };
        let mut verified_keepers: HashMap<String, [u8; 32]> = HashMap::new();

        for item in plan.items.iter().filter(|i| i.status == "ok") {
            let photo = &photos[&item.photo_id];
            let from = Path::new(&item.path);
            // Journal first, so an interrupted batch leaves a trace.
            let journal_id: i64 = {
                let conn = self.db();
                conn.execute(
                    "INSERT INTO journal (batch_id, created_at, op, photo_id, from_path, to_path, size, status)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,'planned')",
                    params![batch_id, now(), op.name(), item.photo_id, item.path, item.target, item.size],
                )?;
                conn.last_insert_rowid()
            };

            let outcome: Result<Option<PathBuf>> = match op {
                Op::Trash => trash::trash(from),
                Op::Move { dest } => {
                    let root = source_roots
                        .get(&photo.source_id)
                        .map(String::as_str)
                        .unwrap_or("/");
                    let rel = from
                        .strip_prefix(root)
                        .unwrap_or(Path::new(from.file_name().unwrap()));
                    let root_name = Path::new(root)
                        .file_name()
                        .map(|n| n.to_os_string())
                        .unwrap_or_default();
                    let to = unique_path(&Path::new(dest).join(root_name).join(rel));
                    move_file(from, &to).map(|_| Some(to))
                }
                Op::Hardlink => self
                    .replace_with_link(
                        from,
                        Path::new(item.target.as_deref().unwrap()),
                        &mut verified_keepers,
                    )
                    .map(|_| None),
            };

            let conn = self.db();
            match outcome {
                Ok(to) => {
                    match op {
                        Op::Trash => {
                            conn.execute(
                                "UPDATE photo SET state='trashed' WHERE id=?1",
                                [item.photo_id],
                            )?;
                        }
                        Op::Move { .. } => {
                            conn.execute(
                                "UPDATE photo SET state='moved' WHERE id=?1",
                                [item.photo_id],
                            )?;
                        }
                        Op::Hardlink => refresh_stat(&conn, item.photo_id, from)?,
                    }
                    if let Some(to) = to {
                        conn.execute(
                            "UPDATE journal SET status='done', to_path=?2 WHERE id=?1",
                            params![journal_id, to.to_string_lossy()],
                        )?;
                    } else {
                        conn.execute("UPDATE journal SET status='done' WHERE id=?1", [journal_id])?;
                    }
                    result.done += 1;
                    result.bytes += item.size;
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    conn.execute(
                        "UPDATE journal SET status='failed', error=?2 WHERE id=?1",
                        params![journal_id, msg],
                    )?;
                    result.failed += 1;
                    result.errors.push(format!("{}: {msg}", item.path));
                }
            }
        }
        Ok(result)
    }

    /// Replaces `dup` with a hard link to `keeper`, after proving the bytes are identical now.
    /// The link is created under a temporary name and renamed into place, so `dup` never vanishes.
    fn replace_with_link(
        &self,
        dup: &Path,
        keeper: &Path,
        verified: &mut HashMap<String, [u8; 32]>,
    ) -> Result<()> {
        let key = keeper.to_string_lossy().to_string();
        let keeper_hash = match verified.get(&key) {
            Some(h) => *h,
            None => {
                let h = hash::full_hash(keeper).context("cannot read the kept copy")?;
                verified.insert(key, h);
                h
            }
        };
        if hash::full_hash(dup)? != keeper_hash {
            bail!("contents no longer match the kept copy; rescan");
        }
        let tmp = temp_sibling(dup);
        let _ = std::fs::remove_file(&tmp);
        std::fs::hard_link(keeper, &tmp).context("this volume does not support hard links")?;
        if let Err(e) = std::fs::rename(&tmp, dup) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    /// Reverses a batch: restores trashed/moved files, turns hard links back into separate copies.
    pub fn undo(&self, batch_id: i64) -> Result<BatchResult> {
        let entries: Vec<JournalRow> = {
            let conn = self.db();
            let mut stmt = conn.prepare(
                "SELECT id, op, photo_id, from_path, to_path, size FROM journal
                 WHERE batch_id = ?1 AND status = 'done' ORDER BY id DESC",
            )?;
            let v = stmt
                .query_map([batch_id], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            v
        };
        if entries.is_empty() {
            bail!("nothing to undo in batch {batch_id}");
        }
        let mut result = BatchResult {
            batch_id,
            ..Default::default()
        };
        for (jid, op, photo_id, from_path, to_path, size) in entries {
            let from = Path::new(&from_path);
            let outcome: Result<()> = (|| match op.as_str() {
                "hardlink" => {
                    if !from.exists() {
                        bail!("file is gone");
                    }
                    let tmp = temp_sibling(from);
                    let _ = std::fs::remove_file(&tmp);
                    std::fs::copy(from, &tmp)?;
                    if let Err(e) = std::fs::rename(&tmp, from) {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(e.into());
                    }
                    Ok(())
                }
                "trash" => {
                    if from.exists() {
                        bail!("something else now exists at the original location");
                    }
                    trash::restore(from, to_path.as_deref().map(Path::new))
                }
                _ => {
                    let to = to_path
                        .as_deref()
                        .context("no record of where the file went")?;
                    if from.exists() {
                        bail!("something else now exists at the original location");
                    }
                    if !Path::new(to).exists() {
                        bail!("file is no longer at {to}");
                    }
                    move_file(Path::new(to), from)
                }
            })();
            let conn = self.db();
            match outcome {
                Ok(()) => {
                    conn.execute("UPDATE journal SET status='undone' WHERE id=?1", [jid])?;
                    if let Some(pid) = photo_id {
                        conn.execute("UPDATE photo SET state='present' WHERE id=?1", [pid])?;
                        refresh_stat(&conn, pid, from)?;
                    }
                    result.done += 1;
                    result.bytes += size;
                }
                Err(e) => {
                    result.failed += 1;
                    result.errors.push(format!("{from_path}: {e:#}"));
                }
            }
        }
        Ok(result)
    }

    pub fn history(&self) -> Result<Vec<HistoryEntry>> {
        let conn = self.db();
        let mut stmt = conn.prepare(
            "SELECT batch_id, MIN(created_at), MIN(op), COUNT(*),
                    SUM(status='done'), SUM(status='failed'), SUM(status='undone'),
                    COALESCE(SUM(CASE WHEN status IN ('done','undone') THEN size ELSE 0 END), 0)
             FROM journal GROUP BY batch_id ORDER BY batch_id DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(HistoryEntry {
                    batch_id: r.get(0)?,
                    created_at: r.get(1)?,
                    op: r.get(2)?,
                    files: r.get(3)?,
                    done: r.get(4)?,
                    failed: r.get(5)?,
                    undone: r.get(6)?,
                    bytes: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}

/// Keeps the index in step with the file after it was relinked or restored.
fn refresh_stat(conn: &rusqlite::Connection, photo_id: i64, path: &Path) -> Result<()> {
    if let Ok(md) = std::fs::symlink_metadata(path) {
        let (dev, ino) = fsmeta::ident(path, &md);
        conn.execute(
            "UPDATE photo SET size=?2, mtime_ns=?3, dev=?4, ino=?5 WHERE id=?1",
            params![photo_id, md.len() as i64, fsmeta::mtime_ns(&md), dev, ino],
        )?;
    }
    Ok(())
}
