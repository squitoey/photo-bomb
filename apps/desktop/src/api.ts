import { invoke } from "@tauri-apps/api/core";

export interface Source {
  id: number;
  root: string;
  last_scan_at: number | null;
  photo_count: number;
  online: boolean;
  removable: boolean;
  missing_count: number;
}

export interface Photo {
  id: number;
  path: string;
  size: number;
  mtime_ns: number;
  content_hash: string | null;
  format: string | null;
  width: number | null;
  height: number | null;
  taken_at: number | null;
  decoded: number;
  decode_error: string | null;
  /** Not found at the last scan (offline or removable source). */
  missing: boolean;
  /** Marked "not a duplicate"; left out of every group. */
  not_dup: boolean;
}

export interface PhotoDetails extends Photo {
  dev: number;
  ino: number;
  source_id: number;
  state: string;
  source_root: string;
  source_removable: boolean;
  exists: boolean;
  partial_hash: string | null;
  phash: string | null;
  dhash: string | null;
  thumb_path: string;
  exif: [string, string][];
}

export interface PhotoPage {
  total: number;
  total_bytes: number;
  photos: Photo[];
}

export interface Member extends Photo {
  distance: number | null;
  hardlinked: boolean;
  exact_copy: boolean;
}

export interface Group {
  id: number;
  kind: "exact" | "similar";
  keeper: number;
  members: Member[];
  reclaimable: number;
}

export interface GroupPage {
  total_groups: number;
  total_photos: number;
  total_reclaimable: number;
  groups: Group[];
}

export interface SimilarOpts {
  threshold: number;
  same_aspect: boolean;
  time_window_secs: number | null;
}

export type Op = { type: "trash" } | { type: "move"; dest: string } | { type: "hardlink" };

export interface PlanItem {
  photo_id: number;
  path: string;
  size: number;
  status: "ok" | "skip" | "blocked";
  note: string | null;
  target: string | null;
}

export interface Plan {
  items: PlanItem[];
  ok: number;
  skipped: number;
  blocked: number;
  bytes: number;
}

export interface BatchResult {
  batch_id: number;
  done: number;
  failed: number;
  skipped: number;
  bytes: number;
  errors: string[];
}

export interface HistoryEntry {
  batch_id: number;
  created_at: number;
  op: string;
  files: number;
  done: number;
  failed: number;
  undone: number;
  bytes: number;
}

export interface AppStatus {
  sources: number;
  photos: number;
  bytes: number;
  decoded: number;
  unreadable: number;
  pairs: number;
  missing: number;
  scanning: boolean;
  ignored: number;
  library_path: string;
}

export interface ScanProgress {
  stage: string;
  done: number;
  total: number;
}

export interface ScanSummary {
  files_seen: number;
  added: number;
  changed: number;
  removed: number;
  missing: number;
  decoded: number;
  decode_failed: number;
  pairs: number;
  offline_sources: string[];
  cancelled: boolean;
  seconds: number;
}

export interface ScanFinished {
  summary: ScanSummary | null;
  error: string | null;
}

export interface Mark {
  id: number;
  size: number;
  path: string;
}

export const api = {
  status: () => invoke<AppStatus>("status"),
  sources: () => invoke<Source[]>("sources"),
  addSource: (path: string) => invoke<Source>("add_source", { path }),
  removeSource: (id: number) => invoke<void>("remove_source", { id }),
  startScan: () => invoke<void>("start_scan"),
  cancelScan: () => invoke<void>("cancel_scan"),
  exactGroups: (page: number, pageSize: number) => invoke<GroupPage>("exact_groups", { page, pageSize }),
  similarGroups: (opts: SimilarOpts, page: number, pageSize: number) =>
    invoke<GroupPage>("similar_groups", { opts, page, pageSize }),
  autoSelect: (kind: "exact" | "similar", opts: SimilarOpts | null) => invoke<Mark[]>("auto_select", { kind, opts }),
  ignoreGroup: (ids: number[]) => invoke<void>("ignore_group", { ids }),
  ignorePhoto: (id: number, others: number[]) => invoke<void>("ignore_photo", { id, others }),
  unignorePhoto: (id: number) => invoke<void>("unignore_photo", { id }),
  setRemovable: (id: number, removable: boolean) => invoke<void>("set_removable", { id, removable }),
  otherPhotos: (opts: SimilarOpts, page: number, pageSize: number) =>
    invoke<PhotoPage>("other_photos", { opts, page, pageSize }),
  photoDetails: (id: number) => invoke<PhotoDetails>("photo_details", { id }),
  clearIgnored: () => invoke<void>("clear_ignored"),
  plan: (ids: number[], op: Op, similar: SimilarOpts | null, allowEmpty: boolean) =>
    invoke<Plan>("plan", { ids, op, similar, allowEmpty }),
  execute: (ids: number[], op: Op, similar: SimilarOpts | null, allowEmpty: boolean) =>
    invoke<BatchResult>("execute", { ids, op, similar, allowEmpty }),
  history: () => invoke<HistoryEntry[]>("history"),
  undo: (batchId: number) => invoke<BatchResult>("undo", { batchId }),
  unreadable: () => invoke<Photo[]>("unreadable"),
  reveal: (id: number) => invoke<void>("reveal", { id }),
};

const UA = navigator.userAgent;
/** "Show in Finder", named for the file manager of this operating system. */
export const REVEAL_LABEL = UA.includes("Mac")
  ? "Show in Finder"
  : UA.includes("Windows")
    ? "Show in Explorer"
    : "Open folder";

// Custom protocols are served as http://<scheme>.localhost on Windows.
const PHOTO_BASE = UA.includes("Windows") ? "http://photo.localhost" : "photo://localhost";
export const thumbUrl = (p: Photo) => `${PHOTO_BASE}/thumb/${p.id}?v=${p.mtime_ns}`;
export const fullUrl = (p: Photo) => `${PHOTO_BASE}/full/${p.id}?v=${p.mtime_ns}`;

export function human(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return u === 0 ? `${bytes} B` : `${v.toFixed(1)} ${units[u]}`;
}

/** Index of the last path separator, on any operating system. */
const lastSep = (path: string) => Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));

export function fileName(path: string): string {
  return path.slice(lastSep(path) + 1);
}

export function dirName(path: string): string {
  return path.slice(0, lastSep(path));
}

/** Hash distance (0-64) as a rough "percent similar". */
export function similarity(distance: number): number {
  return Math.round(100 - (distance / 64) * 100);
}

export function dateTime(secs: number | null): string {
  if (!secs) return "—";
  // Capture times are stored as wall-clock time, so show them without shifting zones.
  return new Date(secs * 1000).toISOString().slice(0, 16).replace("T", " ");
}
