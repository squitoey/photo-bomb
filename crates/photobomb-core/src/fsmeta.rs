//! The few file facts that differ between operating systems.

use std::fs::Metadata;
use std::path::Path;

/// Modification time in nanoseconds since the Unix epoch.
#[cfg(unix)]
pub fn mtime_ns(md: &Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    md.mtime() * 1_000_000_000 + md.mtime_nsec()
}

/// Modification time in nanoseconds since the Unix epoch.
#[cfg(not(unix))]
pub fn mtime_ns(md: &Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// (volume, file) identity. Two paths with the same identity are hard links to one file.
#[cfg(unix)]
pub fn ident(_path: &Path, md: &Metadata) -> (i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (md.dev() as i64, md.ino() as i64)
}

/// (volume, file) identity. Two paths with the same identity are hard links to one file.
#[cfg(windows)]
pub fn ident(path: &Path, _md: &Metadata) -> (i64, i64) {
    use file_id::FileId;
    match file_id::get_file_id(path) {
        Ok(FileId::LowRes {
            volume_serial_number,
            file_index,
        }) => (volume_serial_number as i64, file_index as i64),
        Ok(FileId::HighRes {
            volume_serial_number,
            file_id,
        }) => (volume_serial_number as i64, file_id as i64),
        Ok(FileId::Inode {
            device_id,
            inode_number,
        }) => (device_id as i64, inode_number as i64),
        // Unknown identity must never look like a hard link, so make it unique per path.
        Err(_) => (0, path_hash(path)),
    }
}

#[cfg(not(any(unix, windows)))]
pub fn ident(path: &Path, _md: &Metadata) -> (i64, i64) {
    (0, path_hash(path))
}

#[cfg(not(unix))]
fn path_hash(path: &Path) -> i64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish() as i64
}

/// Identity of the file at `path` right now, if it can be read.
pub fn ident_of(path: &Path) -> Option<(i64, i64)> {
    std::fs::metadata(path).ok().map(|md| ident(path, &md))
}
