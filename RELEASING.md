# Releasing

Releases are cut from `main` by the **Release** workflow (`.github/workflows/release.yml`).

## Cut a release

1. Make sure `main` is green in CI and holds what you want to ship.
2. GitHub → Actions → **Release** → **Run workflow** (branch `main`) and pick the kind:

   | Kind | Use for | Example |
   | --- | --- | --- |
   | `patch` | Bug fixes only | 1.4.2 → 1.4.3 |
   | `minor` | New features; nothing existing breaks | 1.4.2 → 1.5.0 |
   | `major` | Breaking changes; not always backwards compatible | 1.4.2 → 2.0.0 |

   Or from a terminal: `gh workflow run release.yml -f bump=minor`

The workflow then:

1. Runs the tests.
2. Bumps the version, commits `Release vX.Y.Z` to `main` and tags it `vX.Y.Z`.
3. Creates a draft GitHub release with generated notes.
4. Builds one installer per platform and attaches it:
   - macOS: `.dmg` (universal: Apple Silicon and Intel)
   - Windows: `-setup.exe`
   - Linux: `.AppImage`
5. Publishes the release once all three builds have succeeded.

Downloads appear on the repository's Releases page.

## Where the version lives

`[workspace.package] version` in the root `Cargo.toml`. The core, CLI and desktop app
inherit it. `scripts/bump_version.py major|minor|patch` changes it (and
`apps/desktop/package.json`); the workflow runs it for you.

## If a build fails

The version commit and tag already exist and the release stays a draft. Fix forward:
re-run the failed jobs from the workflow run page, or delete the draft release and tag
and start again.

## Notes

- The workflow pushes to `main` with the built-in token. If `main` gets branch
  protection that requires pull requests, allow GitHub Actions to bypass it or the
  bump step will fail.
- Installers are not signed with a paid developer certificate. macOS and Windows show
  an "unidentified developer" warning on first launch (macOS: System Settings →
  Privacy & Security → Open Anyway). Signing and notarisation are a later step.
- HEIC, AVIF and RAW decoding currently relies on a macOS system tool, so those formats
  are only analysed on macOS. They still take part in exact-duplicate matching everywhere.
