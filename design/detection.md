# Detection

Two independent detectors. A photo can be in one exact group and one similar group.

## 1. Exact duplicates (identical bytes)

Definition: two files are exact duplicates when their full-content BLAKE3 hashes are equal.

### Funnel

Each step only runs on files that still collide after the previous one.

| Step | Cost | Eliminates |
| --- | --- | --- |
| 1. Group by file size | Free (from the directory walk) | Any file with a unique size |
| 2. Partial hash: first 64 KB + last 64 KB + size | Two small reads | Most same-size, different-content files |
| 3. Full BLAKE3 of the whole file | Full read | Everything that is not byte-identical |

- Files ≤ 128 KB skip step 2 and go straight to a full hash.
- Zero-byte files are ignored.
- Hashes are cached against `(size, mtime, file identity)`; unchanged files are never re-read.
- Optional paranoid mode: byte-by-byte comparison before any delete.

### Edge cases

- **Hard links** — same file identity means one file with two names, not two copies. Shown with a "hard link" badge; deleting one frees no space, and the UI says so.
- **Symlinks** — not followed by default; the target is indexed only if it is inside a source.
- **Overlapping sources** — paths are canonicalised so a file is never indexed twice.
- **Same pixels, different bytes** (metadata edited, re-saved) — not an exact duplicate by definition. These land in the similar detector at distance 0 and are labelled "identical image, different file".

## 2. Similar photos

### Perceptual hashes

Computed once per photo from a single reduced-size decode, after applying EXIF orientation.

| Hash | Method | Strength |
| --- | --- | --- |
| pHash (64-bit) | Greyscale → 32×32 → DCT → low-frequency 8×8 vs median | Robust to resize, recompression, mild colour/brightness edits |
| dHash (64-bit) | Greyscale → 9×8 → adjacent-pixel gradients | Cheap; errors largely uncorrelated with pHash |

Pair distance:

```
distance(a, b) = max(hamming(pHash_a, pHash_b), hamming(dHash_a, dHash_b))
```

- Requiring both hashes to agree cuts false positives sharply compared to either alone.
- Range 0–64. Useful range is roughly 0–16.

### Pair search

- Find all pairs with `distance ≤ D_MAX` (default 16) and store them with their distance.
- Up to a few hundred thousand photos: parallel brute-force XOR + popcount over packed hash arrays. Simple, and the full comparison is a sequential memory scan.
- Beyond that: multi-index hashing (split the 64 bits into bands, bucket by band) to generate candidates.
- Incremental: a rescan compares only new hashes against the full set.
- Cap stored pairs per photo (e.g. 200 nearest) so flat or blank images cannot explode the pair table.

### The similarity setting

The user control is a slider, "Similarity", from **Loose** to **Strict**, mapped to a Hamming threshold `T`:

| Preset | T | Typically catches |
| --- | --- | --- |
| Identical image | 0 | Same pixels in a different file |
| Strict | 4 | Resized, recompressed, format-converted copies |
| Balanced (default) | 8 | Light edits, filters, small exposure changes |
| Loose | 12 | Burst shots, minor reframing |
| Very loose | 16 | Same scene; expect false positives |

- The slider is continuous (0–16); presets are tick marks.
- Grouping is a query over stored pairs filtered by `distance ≤ T`. Changing the slider regroups in milliseconds with no rescan.
- The UI shows live counts while dragging ("214 groups, 688 photos").
- Raising `D_MAX` above 16 is an advanced setting and requires re-running the pair stage only (hashes are reused).
- Thresholds above are starting values, to be tuned against the test corpus ([roadmap.md](roadmap.md)).

### Grouping

Turning pairs into groups:

1. Take pairs with `distance ≤ T`, minus ignored pairs.
2. Union-find to get connected components.
3. **Chain guard**: single-linkage lets A~B~C~D chain into a group where A and D look nothing alike. For each component, pick the medoid; any member further than `T + slack` from the medoid is split off and regrouped. Slack is small (default 2).
4. Drop singletons.

Optional filters (settings, all applied at query time):

| Filter | Effect |
| --- | --- |
| Same aspect ratio only | Stops crops and unrelated shapes matching |
| Time window | Only group photos taken within N seconds/minutes — isolates burst shots |
| Same folder / across folders only | Focus the review |
| Minimum dimensions | Ignore icons and thumbnails |
| Hide exact duplicates | Collapse byte-identical files to one representative inside similar groups (default on) |

### Stacks are not duplicates

- RAW+JPEG pairs (same basename, same folder) and Live Photo image+video pairs are detected and treated as one unit.
- A stack appears as a single member of a group; actions apply to the whole stack.

### Known limits

| Case | Behaviour |
| --- | --- |
| Rotated 90°/180° without EXIF, mirrored | Missed by default. Optional setting hashes all 8 orientations (8× hash cost, same decode) |
| Heavy crops, collages, large overlays | Often missed — global hashes are not crop-invariant |
| Flat images (sky, blank scans, screenshots of text) | Prone to false matches; mitigated by the dual-hash rule and pair cap |
| Animated GIFs | Hashed on the first frame |

### Future: semantic similarity

- An optional on-device embedding model (ONNX Runtime, small vision model) stores a vector per photo; cosine similarity finds crops, re-edits and "same subject" shots.
- Fits the same shape: per-photo feature → stored pairs with a score → threshold at query time.
- Kept out of v1: model size, slower scans, and harder-to-explain matches.

## Auto-select rules

Rules propose a keeper for each group; the user always confirms. Rules are an ordered list; the first rule that separates candidates wins.

| Rule | Keeps |
| --- | --- |
| Highest resolution | Most pixels |
| Largest file | Least compressed |
| Preferred format | e.g. RAW > HEIC > JPEG > PNG |
| Preferred folder | Files under a chosen source/folder; others are the duplicates |
| Oldest / newest | By capture date, falling back to file date |
| Has metadata | Files with EXIF date/GPS over stripped copies |
| Shortest path / cleanest name | Prefers `IMG_1234.jpg` over `IMG_1234 (2) copy.jpg` |

- Defaults — exact groups: preferred folder → cleanest name → oldest. Similar groups: highest resolution → largest file → has metadata.
- Auto-select never marks every member of a group for removal.
