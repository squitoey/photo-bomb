#!/usr/bin/env python3
"""Bump the project version: major, minor or patch.

The version lives in [workspace.package] in the root Cargo.toml (the app, CLI and core
all inherit it). apps/desktop/package.json is kept in step. Prints the new version.
Run `cargo update --workspace` afterwards so Cargo.lock follows.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CARGO = ROOT / "Cargo.toml"
PACKAGE = ROOT / "apps" / "desktop" / "package.json"
PATTERN = re.compile(r'(\[workspace\.package\][^\[]*?\bversion\s*=\s*")(\d+)\.(\d+)\.(\d+)(")', re.S)


def main() -> int:
    if len(sys.argv) != 2 or sys.argv[1] not in ("major", "minor", "patch"):
        print("usage: bump_version.py major|minor|patch", file=sys.stderr)
        return 2
    kind = sys.argv[1]
    text = CARGO.read_text()
    match = PATTERN.search(text)
    if not match:
        print("no [workspace.package] version found in Cargo.toml", file=sys.stderr)
        return 1
    major, minor, patch = (int(match.group(i)) for i in (2, 3, 4))
    if kind == "major":
        major, minor, patch = major + 1, 0, 0
    elif kind == "minor":
        minor, patch = minor + 1, 0
    else:
        patch += 1
    new = f"{major}.{minor}.{patch}"
    CARGO.write_text(text[: match.start()] + match.group(1) + new + match.group(5) + text[match.end() :])

    package = json.loads(PACKAGE.read_text())
    package["version"] = new
    PACKAGE.write_text(json.dumps(package, indent=2) + "\n")
    print(new)
    return 0


if __name__ == "__main__":
    sys.exit(main())
