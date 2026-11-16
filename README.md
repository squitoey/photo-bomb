# photo-bomb

A utility for managing photos: finds byte-identical files, groups similar-looking photos at an
adjustable similarity level, and lets you trash, move or hard-link the extras, with undo.

Status: proof of concept, developed on macOS; Windows and Linux builds are produced by CI but untested by hand. Design docs are in [design/](design/README.md); what the
POC does and does not cover is in [design/poc.md](design/poc.md).

## Layout

| Path | What |
| --- | --- |
| `crates/photobomb-core` | All logic: scan, hashing, grouping, actions, SQLite index |
| `crates/photobomb-cli` | `photobomb` command-line tool |
| `apps/desktop` | Tauri 2 + React desktop app |
| `design/` | Design documents |

## Build

Needs Rust, Node and pnpm. If `cargo` is not on your PATH: `. "$HOME/.cargo/env"`.

```sh
cargo build --release -p photobomb-cli            # -> target/release/photobomb
cd apps/desktop && pnpm install && pnpm tauri build   # -> target/release/bundle/macos/Photo Bomb.app
```

During development: `cd apps/desktop && pnpm tauri dev`. Tests: `cargo test -p photobomb-core`.

## Desktop app

```sh
open "target/release/bundle/macos/Photo Bomb.app"
```

1. **Sources** — add a folder; it is scanned straight away.
2. **Exact duplicates / Similar photos** — click photos to mark them for removal, or use
   *Auto-select all*. On Similar, drag the slider to change how alike photos must be.
3. **Compare** — side by side; `←`/`→` change the right photo, `X` marks it, `K` keeps only it.
4. **Review & apply** — choose Trash, Move to a folder, or Replace with hard links.
5. **History** — undo any past action.

macOS will ask for permission the first time you scan Desktop, Documents, Downloads or Pictures.

## CLI

The CLI and the app share one index, so results from either show up in both.

```sh
photobomb scan ~/Pictures/Imports          # add the folder as a source and scan
photobomb exact                            # byte-identical groups
photobomb similar --threshold 8            # 0 identical image ... 16 very loose
photobomb similar --same-aspect --time-window 60
photobomb clean exact --action trash       # dry run: keep the suggested file per group
photobomb clean exact --action trash --yes # do it
photobomb clean exact --action hardlink --yes
photobomb clean similar --threshold 4 --action move --dest ~/Quarantine --yes
photobomb apply --ids 12,40 --action trash --yes
photobomb ignore --ids 12,40               # "these are not duplicates"
photobomb ignore --ids 12,40,41 --only 41  # take just photo 41 out of that group
photobomb others                           # files in no exact or similar group
photobomb info 12                          # everything known about one photo, with EXIF
photobomb sources add /Volumes/Backup/Photos --removable
photobomb sources removable /Volumes/Backup/Photos on   # or off
photobomb history
photobomb undo 3
photobomb status
```

Every destructive command is a dry run unless `--yes` is given. Add `--json` to `exact` and
`similar` for scripting. Use `--data-dir <dir>` (or `PHOTOBOMB_DATA_DIR`) to work on a separate
index, which is handy for experiments.

## Where data lives

| Data | Location |
| --- | --- |
| Index | `~/Library/Application Support/photobomb/index.db` |
| Thumbnails | `~/Library/Caches/photobomb/thumbs` |

Nothing is written into your photo folders. Deleting both locations resets the app.

## Development

- CI (`.github/workflows/ci.yml`) runs on every push to every branch: frontend type-check and
  build, `cargo fmt --check`, `cargo clippy -D warnings`, and the tests on Linux, macOS and Windows.
- Before pushing: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- Releases: see [RELEASING.md](RELEASING.md).
