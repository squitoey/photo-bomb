//! photobomb-core: scanning, duplicate detection, grouping and file actions.
//! No UI code lives here; the CLI and the desktop app are thin layers on top.

pub mod action;
pub mod db;
pub mod decode;
pub mod fsmeta;
pub mod group;
pub mod hash;
pub mod scan;
pub mod select;
mod trash;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;

pub use action::{BatchResult, HistoryEntry, Op, Plan, PlanItem};
pub use group::{Group, GroupPage, Member, PhotoPage, SimilarOpts};
pub use scan::{ScanProgress, ScanSummary};

/// Pairs further apart than this are never stored.
pub const D_MAX: u32 = 16;

pub struct Library {
    conn: Mutex<Connection>,
    pub data_dir: PathBuf,
    pub thumbs_dir: PathBuf,
}

#[derive(Serialize, Clone, Debug)]
pub struct Source {
    pub id: i64,
    pub root: String,
    pub last_scan_at: Option<i64>,
    pub photo_count: i64,
    pub online: bool,
    /// On a drive that comes and goes: files that cannot be found are kept in the index.
    pub removable: bool,
    /// Indexed files that were not found at the last scan.
    pub missing_count: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Photo {
    pub id: i64,
    pub source_id: i64,
    pub path: String,
    pub size: i64,
    pub mtime_ns: i64,
    pub dev: i64,
    pub ino: i64,
    pub content_hash: Option<String>,
    pub format: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub taken_at: Option<i64>,
    /// 0 = not attempted, 1 = decoded, 2 = failed
    pub decoded: i64,
    pub decode_error: Option<String>,
    /// Not found at the last scan (offline or removable source). Kept in the index.
    pub missing: bool,
    /// The user marked this file as "not a duplicate"; it is left out of every group.
    pub not_dup: bool,
}

/// Everything known about one photo, for the info panel.
#[derive(Serialize, Clone, Debug)]
pub struct PhotoDetails {
    #[serde(flatten)]
    pub photo: Photo,
    pub state: String,
    pub source_root: String,
    pub source_removable: bool,
    /// The file is on disk right now.
    pub exists: bool,
    pub partial_hash: Option<String>,
    pub phash: Option<String>,
    pub dhash: Option<String>,
    pub thumb_path: String,
    /// EXIF fields read from the file just now, as (name, value).
    pub exif: Vec<(String, String)>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Status {
    pub sources: i64,
    pub photos: i64,
    pub bytes: i64,
    pub decoded: i64,
    pub unreadable: i64,
    pub pairs: i64,
    /// Files kept in the index although they were not found at the last scan.
    pub missing: i64,
}

impl Library {
    /// Opens the default library. `PHOTOBOMB_DATA_DIR` overrides the location.
    pub fn open_default() -> Result<Self> {
        if let Ok(dir) = std::env::var("PHOTOBOMB_DATA_DIR") {
            return Self::open(Path::new(&dir));
        }
        // macOS: ~/Library/Application Support and ~/Library/Caches.
        // Linux: ~/.local/share and ~/.cache. Windows: %APPDATA% and %LOCALAPPDATA%.
        let data = dirs::data_dir()
            .context("cannot find the user data folder")?
            .join("photobomb");
        let thumbs = dirs::cache_dir()
            .context("cannot find the user cache folder")?
            .join("photobomb")
            .join("thumbs");
        Self::open_with(&data, &thumbs)
    }

    /// Opens a library that keeps everything, thumbnails included, under `data_dir`.
    pub fn open(data_dir: &Path) -> Result<Self> {
        Self::open_with(data_dir, &data_dir.join("thumbs"))
    }

    pub fn open_with(data_dir: &Path, thumbs_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        std::fs::create_dir_all(thumbs_dir)?;
        let conn = Connection::open(data_dir.join("index.db"))?;
        db::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            data_dir: data_dir.to_path_buf(),
            thumbs_dir: thumbs_dir.to_path_buf(),
        })
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn thumb_path(&self, photo_id: i64) -> PathBuf {
        self.thumbs_dir.join(format!("{photo_id}.jpg"))
    }

    pub fn add_source(&self, dir: &Path) -> Result<Source> {
        let root =
            dunce::canonicalize(dir).with_context(|| format!("cannot open {}", dir.display()))?;
        anyhow::ensure!(root.is_dir(), "{} is not a folder", root.display());
        anyhow::ensure!(
            !scan::is_protected(&root),
            "{} is a protected library and cannot be scanned",
            root.display()
        );
        let root_s = root.to_string_lossy().to_string();
        let conn = self.db();
        conn.execute("INSERT OR IGNORE INTO source (root) VALUES (?1)", [&root_s])?;
        drop(conn);
        self.sources()?
            .into_iter()
            .find(|s| s.root == root_s)
            .context("source was not saved")
    }

    pub fn remove_source(&self, id: i64) -> Result<()> {
        let conn = self.db();
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM photo WHERE source_id = ?1")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        conn.execute("DELETE FROM source WHERE id = ?1", [id])?;
        drop(conn);
        for pid in ids {
            let _ = std::fs::remove_file(self.thumb_path(pid));
        }
        Ok(())
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let conn = self.db();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.root, s.last_scan_at,
                    (SELECT COUNT(*) FROM photo p WHERE p.source_id = s.id AND p.state = 'present'),
                    s.removable,
                    (SELECT COUNT(*) FROM photo p WHERE p.source_id = s.id AND p.state = 'present' AND p.missing = 1)
             FROM source s ORDER BY s.root",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let root: String = r.get(1)?;
                Ok(Source {
                    id: r.get(0)?,
                    online: Path::new(&root).is_dir(),
                    root,
                    last_scan_at: r.get(2)?,
                    photo_count: r.get(3)?,
                    removable: r.get(4)?,
                    missing_count: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn status(&self) -> Result<Status> {
        let conn = self.db();
        let one = |sql: &str| -> rusqlite::Result<i64> { conn.query_row(sql, [], |r| r.get(0)) };
        Ok(Status {
            sources: one("SELECT COUNT(*) FROM source")?,
            photos: one("SELECT COUNT(*) FROM photo WHERE state = 'present'")?,
            bytes: one("SELECT COALESCE(SUM(size), 0) FROM photo WHERE state = 'present'")?,
            decoded: one("SELECT COUNT(*) FROM photo WHERE state = 'present' AND decoded = 1")?,
            unreadable: one("SELECT COUNT(*) FROM photo WHERE state = 'present' AND decoded = 2")?,
            pairs: one("SELECT COUNT(*) FROM similar_pair")?,
            missing: one("SELECT COUNT(*) FROM photo WHERE state = 'present' AND missing = 1")?,
        })
    }

    /// Marks a source as living on a removable drive (or not).
    pub fn set_removable(&self, id: i64, removable: bool) -> Result<()> {
        let n = self.db().execute(
            "UPDATE source SET removable = ?2 WHERE id = ?1",
            rusqlite::params![id, removable],
        )?;
        anyhow::ensure!(n == 1, "unknown source");
        Ok(())
    }

    pub fn photo_details(&self, id: i64) -> Result<PhotoDetails> {
        let photo = self.photo(id)?.context("unknown photo")?;
        let hex64 = |v: Option<i64>| v.map(|v| format!("{:016x}", v as u64));
        let (state, partial, phash, dhash, source_root, source_removable) = self.db().query_row(
            "SELECT p.state, p.partial_hash, p.phash, p.dhash, s.root, s.removable
             FROM photo p JOIN source s ON s.id = p.source_id WHERE p.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<Vec<u8>>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, bool>(5)?,
                ))
            },
        )?;
        let path = Path::new(&photo.path);
        Ok(PhotoDetails {
            state,
            source_root,
            source_removable,
            exists: path.is_file(),
            partial_hash: partial.map(hex::encode),
            phash: hex64(phash),
            dhash: hex64(dhash),
            thumb_path: self.thumb_path(id).to_string_lossy().to_string(),
            exif: decode::exif_fields(path),
            photo,
        })
    }

    pub fn photo(&self, id: i64) -> Result<Option<Photo>> {
        Ok(db::photos_by_ids(&self.db(), &[id])?.into_iter().next())
    }

    /// The photo's thumbnail, made now if it does not exist yet. Scans only create
    /// thumbnails in their last stage, and the cache folder can be emptied by the
    /// system, so the UI asks for them on demand.
    pub fn ensure_thumb(&self, id: i64) -> Result<PathBuf> {
        let thumb = self.thumb_path(id);
        if thumb.exists() {
            return Ok(thumb);
        }
        let photo = self.photo(id)?.context("unknown photo")?;
        let original = Path::new(&photo.path);
        anyhow::ensure!(decode::is_image(original), "not an image");
        anyhow::ensure!(photo.decoded != 2, "the file could not be read as an image");

        // A few at a time: each one decodes a full-size photo.
        let _slot = ThumbSlot::take();
        if thumb.exists() {
            return Ok(thumb);
        }
        let scratch = self.data_dir.join("scratch-thumbs");
        std::fs::create_dir_all(&scratch)?;
        // Written under another name first, so a half-written file is never served.
        let tmp = self.thumbs_dir.join(format!(
            "{id}.tmp-{}-{:?}.jpg",
            std::process::id(),
            std::thread::current().id()
        ));
        let made = std::panic::catch_unwind(|| decode::process(original, &tmp, &scratch));
        let ok = matches!(made, Ok(Ok(_))) && std::fs::rename(&tmp, &thumb).is_ok();
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            anyhow::bail!("could not make a thumbnail");
        }
        Ok(thumb)
    }

    /// A file the UI can display at full size: the original when a browser can show it,
    /// otherwise a converted copy kept in the cache.
    pub fn preview_path(&self, id: i64) -> Result<PathBuf> {
        let photo = self.photo(id)?.context("unknown photo")?;
        let original = PathBuf::from(&photo.path);
        if !decode::needs_conversion(&original) {
            return Ok(original);
        }
        let cached = self
            .thumbs_dir
            .join(format!("full-{}-{}.jpg", id, photo.mtime_ns));
        if !cached.exists() {
            decode::convert_to_jpeg(&original, &cached, 2400)?;
        }
        Ok(cached)
    }

    /// Photos that could not be decoded (they still take part in exact matching).
    pub fn unreadable(&self) -> Result<Vec<Photo>> {
        db::photos_where(
            &self.db(),
            "state = 'present' AND decoded = 2 ORDER BY path",
        )
    }
}

/// Limits how many thumbnails are generated on demand at once.
struct ThumbSlot;

static THUMB_SLOTS: (Mutex<usize>, std::sync::Condvar) = (Mutex::new(4), std::sync::Condvar::new());

impl ThumbSlot {
    fn take() -> Self {
        let (lock, cv) = &THUMB_SLOTS;
        let mut free = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *free == 0 {
            free = cv.wait(free).unwrap_or_else(|e| e.into_inner());
        }
        *free -= 1;
        ThumbSlot
    }
}

impl Drop for ThumbSlot {
    fn drop(&mut self) {
        let (lock, cv) = &THUMB_SLOTS;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        cv.notify_one();
    }
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
