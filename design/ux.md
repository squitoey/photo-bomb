# UX

## Navigation

```
┌────────────┬──────────────────────────────────────────────┐
│ Sources    │                                              │
│ Exact (132)│              main view                       │
│ Similar(48)│                                              │
│ All photos │                                              │
│ History    │                                              │
│ Settings   │                                              │
├────────────┴──────────────────────────────────────────────┤
│ scan status · selection: 212 files, 1.8 GB · [Review & apply] │
└───────────────────────────────────────────────────────────┘
```

## Screens

### 1. Sources (first run lands here)

- Add folders by picker or drag-and-drop. Per source: path, photo count, last scan, online/offline, exclude patterns, priority (for the "preferred folder" rule).
- **Scan** button; progress shows stage, files/sec, counts found so far, pause/cancel.
- Results appear while scanning; the user does not have to wait for completion.

### 2. Exact duplicates

- List of groups, each a row of thumbnails with path, size, date. Sort by reclaimable space (default), count, date, folder.
- Proposed keeper has a "Keep" badge; the rest are pre-marked only after the user runs an auto-select rule or clicks.
- Group header: "3 copies · 12.4 MB reclaimable", actions: keep-only-this, ignore group.
- Hard links are badged and show 0 bytes reclaimable.

### 3. Similar photos

Same layout, plus:

- **Similarity slider** in the toolbar (Loose ↔ Strict) with live group/photo counts while dragging.
- Filter chips: same aspect ratio, time window, same folder, minimum size.
- Each member shows what differs from the keeper: resolution, file size, format, date, distance badge ("98% similar").
- Split/merge: drag a photo out of a group to split it; records an ignored pair.

### 4. Compare view (open a group)

```
┌─────────────────────────┬─────────────────────────┐
│                         │                         │
│        photo A          │        photo B          │
│                         │                         │
├─────────────────────────┴─────────────────────────┤
│ [filmstrip of group members]                      │
├───────────────────────────────────────────────────┤
│ A: 4032×3024 · 3.1 MB · HEIC · 2024-06-01 · /Phone │
│ B: 2016×1512 · 0.6 MB · JPEG · 2024-06-01 · /Export│
└───────────────────────────────────────────────────┘
```

- Side-by-side with synchronised zoom and pan; toggle to overlay/flicker or a swipe divider for near-identical shots.
- Metadata table highlights the differences and the better value.
- Keyboard-first: `←/→` member, `↑/↓` group, `K` keep, `X` mark for removal, `I` ignore group, `Space` zoom, `Z` undo.

### 5. All photos

- Virtualised grid of everything indexed, with filters (source, folder, date, format, "unreadable", "in a group").
- General management: multi-select, move, trash, reveal in file manager, open in default app.

### 6. Review & apply

Opens from the bottom bar. A dry run of the pending selection:

- Totals: files, space reclaimed, groups affected.
- Destination choice: **Move to trash** (default), **Move to folder…** (quarantine folder, structure preserved), or **Replace with hard links** (exact duplicates only).
- Warnings listed and blocking where needed (see Safety).
- Confirm → progress → summary with an **Undo** button.

### 7. History

- One entry per batch from the action journal: date, action, file count, size.
- Undo a whole batch or single files; shows which can no longer be restored (e.g. trash emptied).

### 8. Settings

- Default similarity level, grouping filters, chain-guard strictness.
- Auto-select rule order (drag to reorder) and format preference.
- File types, minimum file size, exclude patterns, follow symlinks.
- Performance: CPU threads, thumbnail cache size.
- Advanced: rotation-invariant matching, maximum stored distance, byte-verify before delete.

## Management actions

| Action | Scope | Notes |
| --- | --- | --- |
| Mark keep / remove | Photo | Pending selection only; nothing touches disk until Apply |
| Auto-select | Group, filtered list, or all | Applies the rule order; always leaves a keeper |
| Trash | Selection | OS trash; journaled |
| Move to folder | Selection | Preserves relative structure; collision-safe renames |
| Replace with hard links | Exact groups | Optional; same volume only; see below |
| Ignore group / pair | Group | Persisted by content hash; reversible in Settings → Ignored |
| Reveal / open | Photo | File manager or default viewer |
| Export report | Results | CSV/JSON of groups and decisions |
| Undo | Batch or file | Restores from trash or moves back |

Later: merge metadata into the keeper before removing the rest.

### Replace with hard links

An alternative to trashing, for **exact groups only**: every path stays, but all copies share one set of bytes on disk.

- Offered in Review & apply as a third destination: Trash / Move to folder / **Replace with hard links**. Off by default.
- Availability is checked per group; unavailable groups are listed with the reason and left untouched:
  - members on different volumes
  - filesystem without hard-link support (FAT/exFAT, many network shares)
  - mobile platforms
- Execution per duplicate: re-verify its content hash equals the keeper's → create the link under a temporary name next to the duplicate → atomically rename over the duplicate. A crash never leaves a path missing.
- The dialog states the trade-off plainly: linked files are one file — editing any path changes all of them. Tools that save by writing a new file break the link instead.
- The duplicate's own timestamps and permissions are replaced by the keeper's (a hard link shares them).
- Journaled as `hardlink`. Undo copies the bytes back into a separate file at that path.
- After linking, the group shows a "hard link" badge and 0 bytes reclaimable.

## Safety rules

Hard rules enforced in the core, not just the UI:

1. **No permanent delete.** Removal means OS trash or a move. There is no "delete forever" in v1.
2. **Never empty a group.** A plan that removes every member of a group is blocked until the user overrides it per group.
3. **Verify before acting.** Each file's size and mtime are rechecked at execution; if changed, it is skipped and reported. Exact-duplicate removal also confirms at least one other verified copy still exists at that moment.
4. **Everything is journaled** before it happens, so an interrupted batch can be resumed or rolled back.
5. **Offline sources are read-only.** No actions on files whose volume is not mounted.
6. **Protected locations** (Apple Photos library package, system folders) are scan-only.
7. **Stacks move together.** A RAW is never left without its JPEG, or a Live Photo without its video, unless the user asks.

Soft guards:

- Extra confirmation when a batch exceeds a threshold (e.g. 1,000 files or 10 GB).
- Similar-photo removals are never pre-selected without an explicit auto-select step.
- First-run tip explaining that similar ≠ identical.

## States to design for

- Empty (no sources), scanning with partial results, no duplicates found, source offline, unreadable files, permission denied, trash unavailable on the volume (offer move-to-folder instead).

## Accessibility and i18n

- Full keyboard operation, visible focus, screen-reader labels on thumbnails (filename, size, role in group).
- Keep/remove state shown with icon and text, not colour alone.
- Light and dark themes following the OS.
- All strings externalised; sizes and dates formatted per locale.
