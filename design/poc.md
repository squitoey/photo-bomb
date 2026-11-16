# Proof of concept — status

The POC implements the design end to end on macOS so the workflow can be tried on real folders.
It is deliberately narrower than the design in places; those are listed here.

## Built

| Area | State |
| --- | --- |
| Core crate | Scan pipeline (walk → diff → size/partial/full hash funnel → decode → pairs), SQLite index, incremental rescans |
| Exact duplicates | BLAKE3 funnel as designed; hard links recognised and shown |
| Similar photos | pHash + dHash, pairs stored up to distance 16, grouping at query time with the chain guard |
| Similarity setting | Slider 0–16 with presets; same-shape and time-window filters |
| Actions | Trash (system Trash), move to folder, replace with hard links; journal; undo for all three |
| Safety rules | No permanent delete; "never empty a group" enforced in the core, and a copy that is not on disk right now does not count as the kept one; re-check size/mtime before acting; hard links re-verify content |
| Auto-select | Exact: cleanest name → shortest path → oldest. Similar: most pixels → largest file → has capture date |
| Ignore | Whole group ("None are duplicates") or one photo ("Not a duplicate" in its ⋯ menu). Stored as content-hash pairs; a byte-identical copy is flagged per file instead (`photo.not_dup`), since content cannot tell it apart |
| Photo info | Popup with every indexed field, hashes, and the EXIF read live from the file |
| Other photos | View of files in no exact group and no similar group at the current similarity level |
| Removable sources | Per-source flag. Files that cannot be found are kept and flagged (`photo.missing`) instead of dropped; they still appear in groups, tagged "offline". Offline sources behave the same way |
| Live results | Hashes, decoded photos and similar pairs are saved in small batches during a scan. Groups fill in while it runs (the app reloads lists every few seconds) and a stopped scan resumes where it left off. Pair search is incremental: only newly analysed photos are compared |
| Thumbnails | Kept in the local cache folder, never on the scanned drive. Made during the scan's analysis stage, and on demand when the app asks for one that does not exist yet |
| Desktop app | Sources, Exact, Similar, Other photos, Compare, Info, Review & apply, History |
| CLI | Same features, dry-run by default |
| Formats | JPEG, PNG, WebP, GIF, BMP, TIFF natively; HEIC/AVIF/RAW through the macOS `sips` tool |

## Different from the design

| Design | POC |
| --- | --- |
| Windows, macOS, Linux | Developed and run on macOS. The code builds and its tests run on Windows and Linux in CI, and releases include installers for them, but those builds have not been used by hand. HEIC/AVIF/RAW are only decoded on macOS (`sips`) |
| libheif / embedded RAW previews | Shells out to `sips`; slower per file, macOS only |
| Moved files matched by content hash | A moved file is treated as removed + new, and is re-analysed |
| Stacks (RAW+JPEG, Live Photos) | Not implemented; such pairs are treated as unrelated files |
| "Hide exact duplicates" inside similar groups | Not collapsed; byte-identical members are badged instead |
| Configurable rule order, preferred folder/format | Fixed rule order |
| Virtualised grids, All photos view, settings screen, export | Not built; group lists are paged ("Show more") |
| Thumbnails keyed by content hash, LRU budget | Keyed by photo id, no size cap |
| Security-scoped bookmarks, store sandbox | Unsandboxed direct build only |
| Rotation-invariant matching | Not implemented |
| Compare: synced zoom/pan, overlay | Plain side by side |

## Observations from testing

- Resized, recompressed and format-converted copies land at distance 0–2. Unrelated photos in the
  small test set were at 16 or beyond, so there is a wide usable gap.
- The hashes are greyscale, so the same image in a different colour (e.g. colour variants of one
  wallpaper) groups as similar at 3–10. A small colour signature per photo would fix this and is
  the most obvious next improvement to detection.
- The Apple Photos library package is skipped on purpose. Photos kept only inside Photos.app are
  therefore not scanned; that needs PhotoKit, which is the same work as the iOS source.
- macOS suspends the web view a few seconds after the window is hidden. Scans run on a backend
  thread, so they should carry on, but that has not been tested with a hidden window.

## Developer self-test

Starting the app with `PHOTOBOMB_DEBUG_DIR=<folder>` runs a scripted click-through and writes a
log plus HTML snapshots to that folder. It applies and then undoes one hard-link batch, so point
`PHOTOBOMB_DATA_DIR` at a throwaway index when using it.
