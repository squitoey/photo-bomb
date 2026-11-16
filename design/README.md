# Photo Bomb — Design

A local-first, cross-platform app for finding and cleaning up duplicate and similar photos.

| Doc | Contents |
| --- | --- |
| [architecture.md](architecture.md) | Tech stack, components, scan pipeline, IPC, platform notes |
| [detection.md](detection.md) | Exact-duplicate and similar-photo algorithms, the similarity setting |
| [data-model.md](data-model.md) | SQLite schema, caches, invalidation |
| [ux.md](ux.md) | Screens, review workflow, management actions, safety rules |
| [roadmap.md](roadmap.md) | Milestones, testing strategy, open questions |
| [poc.md](poc.md) | What the macOS proof of concept covers and where it differs from the design |

## Goals

- Runs on Windows, macOS and Linux from one codebase; iOS and Android reachable later without a rewrite.
- **Exact duplicates**: files with identical bytes, proven by a cryptographic hash.
- **Similar photos**: visually alike images (resized, recompressed, edited, burst shots), grouped with a user-adjustable similarity level.
- **Manage**: review groups, pick keepers, then trash / move / ignore, with undo.
- Fast on large libraries (target: 100k+ photos), with incremental rescans.
- Fully offline. No account, no upload, no telemetry.

## Non-goals (v1)

- Being a photo editor, a cloud sync tool, or a full library manager (albums, face tagging).
- Video similarity (videos still take part in exact-duplicate detection).
- Importing or reorganising files on its own. The app indexes photos where they live.

## Key decisions

| Decision | Choice | Why |
| --- | --- | --- |
| App shell | Tauri 2 | One codebase for desktop, with iOS/Android targets; small binaries; native core |
| Core language | Rust, as a UI-free library crate | Hashing and decoding are CPU/IO bound; the same crate backs the GUI, a CLI and mobile |
| UI | TypeScript + React, virtualised grids | Mature libraries for virtualisation, accessibility and drag-and-drop; heavy work is in Rust, so framework speed is not the bottleneck |
| Storage | SQLite index + on-disk thumbnail cache | Embedded, transactional, works on every target |
| Exact match | Size → partial hash → full BLAKE3 | Skips full reads for most files; BLAKE3 is fast and collision-resistant |
| Similar match | 64-bit perceptual hashes (pHash + dHash), Hamming distance | Cheap to compute and store, robust to resize/recompress |
| Similarity setting | Applied at query time over precomputed pair distances | Moving the slider regroups instantly, with no rescan |
| Deletion | OS trash only, journaled, undoable | Photo loss is unrecoverable; safety beats convenience |
| Hard links | Optional action for exact duplicates: replace copies with hard links to the keeper | Frees space while keeping every path in place |
| Video | Exact-duplicate detection only for now | Similarity may follow; the per-photo feature → pairs → threshold shape already allows it |
| Distribution | Direct downloads and app stores | Core must work inside store sandboxes from the start |
| File ownership | Index in place, never move without an explicit action | The app must be safe to try on a real library |

## Vocabulary

- **Source** — a folder (or, on mobile, the device photo library) the user has added for scanning.
- **Photo** — one indexed image file.
- **Exact group** — two or more photos with the same content hash.
- **Similar group** — two or more photos linked by perceptual distance at the current similarity level.
- **Keeper** — the photo in a group the user (or an auto-select rule) chooses to keep.
- **Stack** — files that belong together and are not duplicates of each other (RAW+JPEG pair, Live Photo image+video).
- **Action journal** — the persistent log of every file operation, used for undo.
