# Roadmap

## Milestones

### M1 — Core + CLI (exact duplicates)

- `photobomb-core`: filesystem source, walk, diff, size → partial → full hash funnel, SQLite index.
- CLI: `scan <dir>`, `dupes --json`.
- Exit: correct exact groups on a test tree; rescan of an unchanged tree does no file reads.

### M2 — Desktop shell

- Tauri app: Sources, scan progress, Exact duplicates list, thumbnails via `photo://`.
- Trash + move with the action journal, Review & apply, History with undo.
- Exit: end-to-end clean-up of exact duplicates on Windows, macOS and Linux.

### M3 — Similar photos

- Decode stage (JPEG, PNG, WebP, TIFF, GIF, HEIC, AVIF), pHash + dHash, pair search, grouping with chain guard.
- Similarity slider with live counts, filters, compare view, ignore pairs, auto-select rules.
- Exit: meets the accuracy targets below on the test corpus; slider regroup feels instant at 100k photos.

### M4 — Polish

- RAW via embedded previews, stacks (RAW+JPEG, Live Photos).
- All photos view, export report, settings, offline sources, keyboard shortcuts, accessibility pass.
- Replace exact duplicates with hard links (optional action, with undo).
- Direct builds: signed installers with auto-update.
- Store builds: Mac App Store, Microsoft Store, Flathub; sandbox test pass on each.

### M5 — Later

- Filesystem watching for live updates.
- Metadata merge.
- Semantic similarity via on-device embeddings.
- Video similarity (keyframe hashes) — not committed; exact-only until then.
- iOS and Android: PhotoKit / MediaStore sources, mobile layout.

## Testing

| Layer | Approach |
| --- | --- |
| Hash funnel | Property tests: funnel result equals naive full-hash grouping on generated trees (same-size files, shared prefixes/suffixes, hard links, empty files) |
| Perceptual accuracy | Labelled corpus: base images × transforms (resize, recompress, format change, brightness, crop, rotate, watermark) plus known-different lookalikes. Report precision/recall per threshold; use it to set the slider presets |
| Grouping | Unit tests for union-find, chain guard, filters, ignored pairs |
| Actions | Integration tests on temp dirs: trash, move, hard link (cross-volume refusal, unsupported filesystem, crash mid-replace), collision, file changed mid-batch, crash mid-batch then resume, undo |
| Safety | Tests that the core rejects plans that empty a group or touch offline/protected sources |
| Performance | Benchmarks on synthetic libraries (10k / 100k / 500k): scan throughput, rescan time, regroup latency, memory ceiling |
| Cross-platform | CI matrix on all three desktop OSes; path edge cases (unicode, long paths, case-insensitivity) |
| UI | Component tests plus a small end-to-end suite against a fixture library |

Initial targets (to be validated, not promises):

- Exact detection: zero false positives, zero false negatives.
- Similar at "Balanced": catches resized/recompressed copies essentially always; false-positive groups rare enough that review stays pleasant.
- Rescan of an unchanged 100k library: seconds.
- Slider regroup at 100k photos: under 100 ms.

## Risks

| Risk | Mitigation |
| --- | --- |
| User loses photos | Trash-only, never-empty-group rule, verify-before-act, journal + undo, conservative defaults |
| False positives erode trust | Dual-hash rule, chain guard, no pre-selection for similar groups, ignore list |
| HEIC/RAW decoding differs per platform | Bundle libheif; RAW via embedded preview only; unreadable files degrade to exact-only |
| Slow on HDD / NAS | Per-device IO limits, hash funnel minimises reads, resumable scans |
| Pair table blow-up on huge or uniform libraries | `D_MAX` cap, per-photo pair cap, multi-index hashing |
| Mobile platform limits | `PhotoSource` abstraction from day one; exact hashing optional where originals are remote |

## Open questions

1. Should "move to folder" be offered as a first-class organise feature (e.g. sort by date), or only as a quarantine for duplicates?
2. Is a CLI a shipped product feature or only a development tool? (A CLI cannot ship through the sandboxed stores.)
3. Free or paid, and the same on both channels?

## Decided

| Question | Decision |
| --- | --- |
| Video similarity | Exact-only for now; may be added later |
| Hard links | Yes, as an optional action for exact duplicates |
| Distribution | Both direct downloads and app stores |
| UI framework | React (recommended; see README key decisions) |
