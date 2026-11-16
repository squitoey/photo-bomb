//! Auto-select rules: which member of a group to propose as the keeper.

use std::cmp::Ordering;
use std::path::Path;

use crate::Photo;

/// Lower is cleaner: penalises "copy", "(2)", " 2" style names.
fn name_noise(path: &str) -> u32 {
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut n = 0;
    if stem.contains("copy") || stem.contains("duplicate") {
        n += 2;
    }
    if stem.ends_with(')') && stem.contains('(') {
        n += 2;
    }
    if stem.split(' ').count() > 1
        && stem
            .rsplit(' ')
            .next()
            .is_some_and(|t| t.parse::<u32>().is_ok())
    {
        n += 1;
    }
    n
}

/// Exact groups: cleanest name, then shortest path, then oldest file.
pub fn cmp_exact(a: &Photo, b: &Photo) -> Ordering {
    name_noise(&a.path)
        .cmp(&name_noise(&b.path))
        .then(a.path.len().cmp(&b.path.len()))
        .then(a.mtime_ns.cmp(&b.mtime_ns))
        .then(a.id.cmp(&b.id))
}

/// Similar groups: most pixels, then largest file, then has a capture date.
pub fn cmp_similar(a: &Photo, b: &Photo) -> Ordering {
    let px = |p: &Photo| p.width.unwrap_or(0) * p.height.unwrap_or(0);
    px(b)
        .cmp(&px(a))
        .then(b.size.cmp(&a.size))
        .then(b.taken_at.is_some().cmp(&a.taken_at.is_some()))
        .then(cmp_exact(a, b))
}

#[cfg(test)]
mod tests {
    use super::name_noise;

    #[test]
    fn noise() {
        assert_eq!(name_noise("/a/IMG_1234.jpg"), 0);
        assert!(name_noise("/a/IMG_1234 copy.jpg") > 0);
        assert!(name_noise("/a/IMG_1234 (2).jpg") > 0);
        assert!(name_noise("/a/IMG_1234 2.jpg") > 0);
    }
}
