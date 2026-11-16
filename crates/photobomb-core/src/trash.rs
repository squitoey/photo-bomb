//! Move a file to the system trash and report where it ended up, so it can be restored.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[cfg(target_os = "macos")]
pub fn trash(path: &Path) -> Result<PathBuf> {
    use anyhow::{anyhow, Context};
    use objc2::rc::Retained;
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    let ns_path = NSString::from_str(&path.to_string_lossy());
    let url = NSURL::fileURLWithPath(&ns_path);
    let fm = NSFileManager::defaultManager();
    let mut out: Option<Retained<NSURL>> = None;
    fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut out))
        .map_err(|e| anyhow!("{}", e.localizedDescription()))?;
    let out = out.context("the system did not report where the file went")?;
    let p = out.path().context("trash location has no path")?;
    Ok(PathBuf::from(p.to_string()))
}

#[cfg(not(target_os = "macos"))]
pub fn trash(_path: &Path) -> Result<PathBuf> {
    anyhow::bail!("trash is only implemented for macOS in this build; use move-to-folder")
}
