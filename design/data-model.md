# Data model

SQLite, WAL mode, one writer thread. Schema versioned with forward-only migrations.

## Schema

```sql
-- Folders (or device libraries) the user scans
CREATE TABLE source (
  id            INTEGER PRIMARY KEY,
  kind          TEXT NOT NULL,          -- 'fs' | 'photokit' | 'mediastore'
  root          TEXT NOT NULL,          -- canonical path or library id
  volume_id     TEXT,                   -- detects unmounted / swapped drives
  bookmark      BLOB,                   -- macOS security-scoped bookmark
  excludes      TEXT NOT NULL DEFAULT '[]',  -- JSON array of globs
  priority      INTEGER NOT NULL DEFAULT 0,  -- for the "preferred folder" rule
  last_scan_at  INTEGER
);

-- One row per indexed file
CREATE TABLE photo (
  id            INTEGER PRIMARY KEY,
  source_id     INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  rel_path      TEXT NOT NULL,          -- relative to source root
  size          INTEGER NOT NULL,
  mtime_ns      INTEGER NOT NULL,
  file_identity TEXT,                   -- dev+inode / volume+file id (hard links)
  state         TEXT NOT NULL DEFAULT 'present', -- present | missing | trashed | moved
  -- content hashes (NULL until that funnel step was needed)
  partial_hash  BLOB,
  content_hash  BLOB,                   -- BLAKE3, 32 bytes
  -- image info (NULL if not decodable)
  format        TEXT,
  width         INTEGER,
  height        INTEGER,
  taken_at      INTEGER,                -- EXIF capture time, UTC
  camera        TEXT,
  has_gps       INTEGER,
  orientation   INTEGER,
  phash         INTEGER,                -- 64-bit
  dhash         INTEGER,                -- 64-bit
  hash_version  INTEGER,                -- perceptual algorithm version
  decode_error  TEXT,
  stack_id      INTEGER REFERENCES stack(id),
  UNIQUE (source_id, rel_path)
);
CREATE INDEX photo_size    ON photo(size);
CREATE INDEX photo_content ON photo(content_hash) WHERE content_hash IS NOT NULL;
CREATE INDEX photo_taken   ON photo(taken_at);

-- RAW+JPEG pairs, Live Photos
CREATE TABLE stack (
  id    INTEGER PRIMARY KEY,
  kind  TEXT NOT NULL                   -- 'raw_jpeg' | 'live_photo'
);

-- Perceptual pairs within D_MAX. a_id < b_id.
CREATE TABLE similar_pair (
  a_id      INTEGER NOT NULL REFERENCES photo(id) ON DELETE CASCADE,
  b_id      INTEGER NOT NULL REFERENCES photo(id) ON DELETE CASCADE,
  distance  INTEGER NOT NULL,
  PRIMARY KEY (a_id, b_id)
) WITHOUT ROWID;
CREATE INDEX similar_pair_dist ON similar_pair(distance);
CREATE INDEX similar_pair_b    ON similar_pair(b_id);

-- "These are not duplicates" decisions, keyed by content so they survive moves
CREATE TABLE ignored_pair (
  hash_a  BLOB NOT NULL,
  hash_b  BLOB NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (hash_a, hash_b)
) WITHOUT ROWID;

-- Every file operation, for undo and audit
CREATE TABLE journal (
  id          INTEGER PRIMARY KEY,
  batch_id    INTEGER NOT NULL,         -- one user action = one batch
  created_at  INTEGER NOT NULL,
  op          TEXT NOT NULL,            -- 'trash' | 'move' | 'hardlink'
  photo_id    INTEGER,
  from_path   TEXT NOT NULL,
  to_path     TEXT,                     -- move destination, trash location, or hard-link target (keeper)
  content_hash BLOB,
  status      TEXT NOT NULL,            -- planned | done | failed | undone
  error       TEXT
);
CREATE INDEX journal_batch ON journal(batch_id);

CREATE TABLE setting (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL                  -- JSON
);
```

## Derived, not stored

- **Exact groups**: `GROUP BY content_hash HAVING COUNT(*) > 1` over present photos.
- **Similar groups**: built in memory from `similar_pair WHERE distance <= T` (see [detection.md](detection.md)). Cached per `(T, filters, index revision)` and dropped when the index changes.
- Group IDs handed to the UI are stable for a given cache entry: the smallest member photo ID.

Why not store groups: they depend on the slider and filters, and recomputing from pairs is cheap.

## Invalidation

| Change | Effect |
| --- | --- |
| File size or mtime changed | Clear its hashes and pairs; reprocess |
| File missing | `state = missing`; purged after the next successful full scan of that source |
| New file matches a missing file's size + content hash | Treated as a move: row updated, perceptual data kept |
| `hash_version` bumped by an app update | Perceptual hashes and pairs recomputed in the background; content hashes untouched |
| Source offline | Rows kept and flagged missing; shown in groups as "offline"; never chosen as keeper, never count as the surviving copy, no actions allowed |
| Source marked removable | A file that cannot be found is flagged missing instead of purged, however long it stays away; cleared when the file is seen again |

## Thumbnail cache

- Path: `{cache}/thumbs/{first 2 hex of key}/{key}.webp`, key = content hash (or partial hash + size when no full hash exists).
- Two sizes: grid (256 px) and compare (1024 px, generated lazily).
- LRU-evicted against a size budget (default 2 GB, configurable).
- Keyed by content, so duplicates share one thumbnail and moves need no regeneration.

## Size estimate

Rough per-photo footprint: under 500 bytes for the row and indexes, plus pairs. 100k photos is on the order of tens of MB of index; thumbnails dominate disk use.
