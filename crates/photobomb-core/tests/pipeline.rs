//! End-to-end: scan a generated folder, check grouping, then exercise actions and undo.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use image::{imageops::FilterType, DynamicImage, Rgb, RgbImage};
use photobomb_core::{fsmeta, scan, Library, Op, SimilarOpts};

/// A blocky random image: an 8x8 grid of colours scaled up, so each seed looks distinct.
fn picture(seed: u64, size: u32) -> DynamicImage {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 24) as u8
    };
    let mut small = RgbImage::new(8, 8);
    for p in small.pixels_mut() {
        *p = Rgb([next(), next(), next()]);
    }
    DynamicImage::ImageRgb8(small).resize_exact(size, size, FilterType::CatmullRom)
}

fn save(img: &DynamicImage, path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    img.to_rgb8().save(path).unwrap();
}

fn rescan(lib: &Library) -> scan::ScanSummary {
    scan::scan(lib, &|_| {}, &AtomicBool::new(false)).unwrap()
}

#[test]
fn scan_group_act_undo() {
    let tmp = tempfile::tempdir().unwrap();
    let photos = tmp.path().join("photos");
    let a = photos.join("a/one.jpg");
    let a_copy = photos.join("b/one copy.jpg");
    let a_small = photos.join("b/one_small.png");
    let other = photos.join("a/two.jpg");
    save(&picture(1, 512), &a);
    fs::create_dir_all(a_copy.parent().unwrap()).unwrap();
    fs::copy(&a, &a_copy).unwrap();
    save(
        &picture(1, 512).resize_exact(200, 200, FilterType::Triangle),
        &a_small,
    );
    save(&picture(2, 512), &other);
    fs::write(photos.join("a/broken.jpg"), b"not a picture").unwrap();

    let lib = Library::open(&tmp.path().join("lib")).unwrap();
    lib.add_source(&photos).unwrap();
    let first = rescan(&lib);
    assert_eq!(first.files_seen, 5);
    assert_eq!(first.added, 5);
    assert_eq!(first.decoded, 4);
    assert_eq!(first.decode_failed, 1);

    // A second scan of an unchanged folder does no work.
    let second = rescan(&lib);
    assert_eq!(
        (second.added, second.changed, second.removed, second.decoded),
        (0, 0, 0, 0)
    );

    // Exact: the byte copy, and only that.
    let exact = lib.exact_groups().unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].members.len(), 2);
    let keeper = exact[0]
        .members
        .iter()
        .find(|m| m.photo.id == exact[0].keeper)
        .unwrap();
    assert!(
        Path::new(&keeper.photo.path).ends_with("a/one.jpg"),
        "clean name is preferred as keeper"
    );
    let dup = exact[0]
        .members
        .iter()
        .find(|m| m.photo.id != exact[0].keeper)
        .unwrap()
        .photo
        .clone();

    // Similar: the resized copy joins; the unrelated picture does not, even when loose.
    let similar = lib
        .similar_groups(&SimilarOpts {
            threshold: 16,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(similar.len(), 1);
    let names: Vec<&str> = similar[0]
        .members
        .iter()
        .map(|m| m.photo.path.as_str())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("one_small.png")),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.ends_with("two.jpg")), "{names:?}");

    // Safety: selecting every copy is blocked.
    let all: Vec<i64> = exact[0].members.iter().map(|m| m.photo.id).collect();
    let dest = tmp.path().join("quarantine").to_string_lossy().to_string();
    let mv = Op::Move { dest };
    assert_eq!(lib.plan(&all, &mv, None, false).unwrap().blocked, 2);
    assert!(lib.execute(&all, &mv, None, false).is_err());
    assert!(a.exists() && a_copy.exists());

    // Hard link, then undo.
    let res = lib.execute(&[dup.id], &Op::Hardlink, None, false).unwrap();
    assert_eq!((res.done, res.failed), (1, 0));
    assert_eq!(fsmeta::ident_of(&a), fsmeta::ident_of(&a_copy));
    assert_eq!(lib.exact_groups().unwrap()[0].reclaimable, 0);
    lib.undo(res.batch_id).unwrap();
    assert_ne!(fsmeta::ident_of(&a), fsmeta::ident_of(&a_copy));
    assert_eq!(fs::read(&a).unwrap(), fs::read(&a_copy).unwrap());

    // Move, then undo.
    let res = lib.execute(&[dup.id], &mv, None, false).unwrap();
    assert_eq!(res.done, 1);
    assert!(!a_copy.exists());
    assert!(lib.exact_groups().unwrap().is_empty());
    lib.undo(res.batch_id).unwrap();
    assert!(a_copy.exists());
    assert_eq!(lib.exact_groups().unwrap().len(), 1);

    // A file that changed since the scan is skipped, not touched.
    fs::write(&a_copy, b"changed").unwrap();
    let plan = lib.plan(&[dup.id], &mv, None, false).unwrap();
    assert_eq!((plan.ok, plan.skipped), (0, 1));

    // "Not duplicates" hides a group.
    rescan(&lib);
    let similar = lib
        .similar_groups(&SimilarOpts {
            threshold: 16,
            ..Default::default()
        })
        .unwrap();
    let ids: Vec<i64> = similar[0].members.iter().map(|m| m.photo.id).collect();
    lib.ignore_group(&ids).unwrap();
    assert!(lib
        .similar_groups(&SimilarOpts {
            threshold: 16,
            ..Default::default()
        })
        .unwrap()
        .is_empty());
}

#[test]
fn single_photo_ignore_others_and_removable_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let photos = tmp.path().join("photos");
    let a = photos.join("a/one.jpg");
    let a_copy = photos.join("b/one copy.jpg");
    let a_small = photos.join("b/one_small.png");
    let a_mid = photos.join("b/one_mid.png");
    let other = photos.join("a/two.jpg");
    save(&picture(1, 512), &a);
    fs::create_dir_all(a_copy.parent().unwrap()).unwrap();
    fs::copy(&a, &a_copy).unwrap();
    save(
        &picture(1, 512).resize_exact(200, 200, FilterType::Triangle),
        &a_small,
    );
    save(
        &picture(1, 512).resize_exact(300, 300, FilterType::Triangle),
        &a_mid,
    );
    save(&picture(2, 512), &other);

    let lib = Library::open(&tmp.path().join("lib")).unwrap();
    let src = lib.add_source(&photos).unwrap();
    rescan(&lib);
    let loose = SimilarOpts {
        threshold: 16,
        ..Default::default()
    };
    let id_of = |suffix: &str| -> i64 {
        let all = lib
            .other_photos(&SimilarOpts {
                threshold: 0,
                ..Default::default()
            })
            .unwrap();
        let exact = lib.exact_groups().unwrap();
        all.iter()
            .chain(
                exact
                    .iter()
                    .flat_map(|g| g.members.iter().map(|m| &m.photo)),
            )
            .find(|p| Path::new(&p.path).ends_with(suffix))
            .map(|p| p.id)
            .unwrap_or_else(|| panic!("no photo {suffix}"))
    };

    // Other photos: only the unrelated picture is in no group.
    let others = lib.other_photos(&loose).unwrap();
    assert_eq!(others.len(), 1, "{others:?}");
    assert!(others[0].path.ends_with("two.jpg"));

    // Taking one photo out of a similar group leaves the rest grouped.
    let small = id_of("one_small.png");
    let group: Vec<i64> = lib.similar_groups(&loose).unwrap()[0]
        .members
        .iter()
        .map(|m| m.photo.id)
        .collect();
    assert_eq!(group.len(), 4);
    lib.ignore_photo(small, &group).unwrap();
    let after = lib.similar_groups(&loose).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].members.len(), 3);
    assert!(!after[0].members.iter().any(|m| m.photo.id == small));
    assert!(lib
        .other_photos(&loose)
        .unwrap()
        .iter()
        .any(|p| p.id == small));

    // A byte-identical copy is flagged itself; its twin stays in the similar group.
    let copy = id_of("one copy.jpg");
    let exact: Vec<i64> = lib.exact_groups().unwrap()[0]
        .members
        .iter()
        .map(|m| m.photo.id)
        .collect();
    lib.ignore_photo(copy, &exact).unwrap();
    assert!(lib.exact_groups().unwrap().is_empty());
    assert!(lib.photo(copy).unwrap().unwrap().not_dup);
    assert_eq!(lib.similar_groups(&loose).unwrap()[0].members.len(), 2);
    lib.clear_ignored().unwrap();
    assert_eq!(lib.exact_groups().unwrap().len(), 1);
    assert_eq!(lib.similar_groups(&loose).unwrap()[0].members.len(), 4);

    // Removable source: a file that cannot be found stays in the index.
    lib.set_removable(src.id, true).unwrap();
    let stash = tmp.path().join("stash.jpg");
    fs::rename(&a, &stash).unwrap();
    let s = rescan(&lib);
    assert_eq!((s.removed, s.missing), (0, 1));
    let one = id_of("a/one.jpg");
    assert!(lib.photo(one).unwrap().unwrap().missing);
    let g = &lib.exact_groups().unwrap()[0];
    assert_eq!(g.keeper, copy, "an available file is preferred as keeper");
    assert_eq!(g.reclaimable, 0);
    // The only available copy cannot be removed on the strength of an offline one.
    assert_eq!(
        lib.plan(&[copy], &Op::Trash, None, false).unwrap().blocked,
        1
    );
    assert!(photobomb_core::group::auto_select(&lib.exact_groups().unwrap()).is_empty());

    // It comes back: same row, no longer missing.
    fs::rename(&stash, &a).unwrap();
    let s = rescan(&lib);
    assert_eq!((s.added, s.removed, s.missing), (0, 0, 0));
    assert!(!lib.photo(one).unwrap().unwrap().missing);

    // Not removable: a file that is gone is dropped.
    lib.set_removable(src.id, false).unwrap();
    fs::remove_file(&other).unwrap();
    let s = rescan(&lib);
    assert_eq!((s.removed, s.missing), (1, 0));

    // An offline source keeps its rows, flagged as missing.
    fs::rename(&photos, tmp.path().join("unplugged")).unwrap();
    let s = rescan(&lib);
    assert_eq!((s.removed, s.missing), (0, 4));
    assert_eq!(lib.status().unwrap().missing, 4);
}

#[test]
fn results_are_saved_during_a_scan_and_later_scans_are_incremental() {
    let tmp = tempfile::tempdir().unwrap();
    let photos = tmp.path().join("photos");
    let a = photos.join("one.jpg");
    save(&picture(1, 512), &a);
    fs::copy(&a, photos.join("one copy.jpg")).unwrap();
    save(&picture(2, 512), &photos.join("two.jpg"));
    let lib = Library::open(&tmp.path().join("lib")).unwrap();
    lib.add_source(&photos).unwrap();

    // By the time the decode stage reports, the exact stage has already saved its group.
    let seen_early = std::sync::atomic::AtomicBool::new(false);
    scan::scan(
        &lib,
        &|p| {
            if p.stage == "Analysing photos" && lib.exact_groups().unwrap().len() == 1 {
                seen_early.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(seen_early.load(std::sync::atomic::Ordering::Relaxed));
    let loose = SimilarOpts {
        threshold: 16,
        ..Default::default()
    };
    assert!(lib.similar_groups(&loose).unwrap().is_empty());

    // A scan stopped before it starts changes nothing and loses nothing.
    let stopped = scan::scan(&lib, &|_| {}, &AtomicBool::new(true)).unwrap();
    assert!(stopped.cancelled);
    assert_eq!(lib.exact_groups().unwrap().len(), 1);

    // Files added later are matched against what is already there: one new exact copy,
    // one new resized version.
    fs::copy(&a, photos.join("one again.jpg")).unwrap();
    save(
        &picture(2, 512).resize_exact(240, 240, FilterType::Triangle),
        &photos.join("two_small.png"),
    );
    let s = rescan(&lib);
    assert_eq!((s.added, s.decoded), (2, 2));
    assert_eq!(lib.exact_groups().unwrap()[0].members.len(), 3);
    let similar = lib.similar_groups(&loose).unwrap();
    assert_eq!(similar.len(), 1, "{similar:?}");
    assert!(similar[0]
        .members
        .iter()
        .any(|m| m.photo.path.ends_with("two_small.png")));

    // Nothing new: no work, same pairs.
    let again = rescan(&lib);
    assert_eq!((again.hashed, again.decoded, again.pairs), (0, 0, s.pairs));
}
