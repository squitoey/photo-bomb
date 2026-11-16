import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, AppStatus, human, Photo, ScanProgress, ScanSummary, Source } from "./api";

interface Props {
  status: AppStatus | null;
  scanning: boolean;
  progress: ScanProgress | null;
  lastScan: ScanSummary | null;
  revision: number;
  onScan: () => void;
  onChanged: () => void;
  onError: (e: string) => void;
}

export function Sources({ status, scanning, progress, lastScan, revision, onScan, onChanged, onError }: Props) {
  const [sources, setSources] = useState<Source[]>([]);
  const [unreadable, setUnreadable] = useState<Photo[] | null>(null);

  useEffect(() => {
    api.sources().then(setSources).catch((e) => onError(String(e)));
    setUnreadable(null);
  }, [revision, onError]);

  const addFolder = async () => {
    try {
      const picked = await open({ directory: true, multiple: true, title: "Choose folders to scan" });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      for (const p of paths) await api.addSource(p);
      onChanged();
      onScan();
    } catch (e) {
      onError(String(e));
    }
  };

  const setRemovable = async (s: Source, removable: boolean) => {
    try {
      await api.setRemovable(s.id, removable);
      onChanged();
    } catch (e) {
      onError(String(e));
    }
  };

  const remove = async (s: Source) => {
    try {
      await api.removeSource(s.id);
      onChanged();
    } catch (e) {
      onError(String(e));
    }
  };

  const pct = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : null;

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h2>Sources</h2>
          <p className="muted">
            Folders are indexed where they are. Nothing is moved or changed until you apply an action.
          </p>
        </div>
        <div className="row">
          <button onClick={addFolder} disabled={scanning}>
            Add folder…
          </button>
          {scanning ? (
            <button onClick={() => api.cancelScan()}>Stop scan</button>
          ) : (
            <button className="primary" onClick={onScan} disabled={sources.length === 0}>
              Scan now
            </button>
          )}
        </div>
      </header>

      {scanning && progress && (
        <div className="card">
          <div className="row spread">
            <strong>{progress.stage}</strong>
            <span className="muted">
              {progress.total > 0
                ? `${progress.done.toLocaleString()} of ${progress.total.toLocaleString()}`
                : progress.done.toLocaleString()}
            </span>
          </div>
          <div className="progress">
            <div className={pct === null ? "bar indeterminate" : "bar"} style={pct === null ? undefined : { width: `${pct}%` }} />
          </div>
        </div>
      )}

      {sources.length === 0 ? (
        <div className="empty">
          <h3>No folders yet</h3>
          <p>Add a folder of photos to look for duplicates and similar shots.</p>
          <button className="primary" onClick={addFolder}>
            Add folder…
          </button>
        </div>
      ) : (
        <div className="card list">
          {sources.map((s) => (
            <div key={s.id} className="list-row">
              <div className="grow">
                <div className="path">{s.root}</div>
                <div className="muted small">
                  {s.photo_count.toLocaleString()} files ·{" "}
                  {scanning && s.online
                    ? "scanning…"
                    : s.last_scan_at
                      ? `scanned ${new Date(s.last_scan_at * 1000).toLocaleString()}`
                      : "not scanned yet"}
                  {s.missing_count > 0 && ` · ${s.missing_count.toLocaleString()} not found, kept`}
                  {!s.online && <span className="tag warn">offline</span>}
                </div>
              </div>
              <label
                className="check"
                title="For folders on an external or network drive. Files that cannot be found stay in the index, so they are matched again when the drive comes back."
              >
                <input type="checkbox" checked={s.removable} disabled={scanning} onChange={(e) => setRemovable(s, e.target.checked)} />
                Removable drive
              </label>
              <button onClick={() => remove(s)} disabled={scanning} title="Stops tracking this folder. Files are not touched.">
                Remove
              </button>
            </div>
          ))}
        </div>
      )}

      {status && status.photos > 0 && (
        <div className="tiles">
          <div className="tile">
            <div className="num">{status.photos.toLocaleString()}</div>
            <div className="muted">files indexed</div>
          </div>
          <div className="tile">
            <div className="num">{human(status.bytes)}</div>
            <div className="muted">total size</div>
          </div>
          <div className="tile">
            <div className="num">{status.decoded.toLocaleString()}</div>
            <div className="muted">photos analysed</div>
          </div>
          <div className="tile">
            <div className="num">{status.unreadable.toLocaleString()}</div>
            <div className="muted">
              unreadable{" "}
              {status.unreadable > 0 && unreadable === null && (
                <a onClick={() => api.unreadable().then(setUnreadable).catch((e) => onError(String(e)))}>show</a>
              )}
            </div>
          </div>
        </div>
      )}

      {unreadable && unreadable.length > 0 && (
        <div className="card list">
          <div className="muted small pad">Unreadable files still take part in exact-duplicate matching.</div>
          {unreadable.map((p) => (
            <div key={p.id} className="list-row">
              <div className="grow">
                <div className="path">{p.path}</div>
                <div className="muted small">{p.decode_error}</div>
              </div>
              <button onClick={() => api.reveal(p.id)}>Show in Finder</button>
            </div>
          ))}
        </div>
      )}

      {lastScan && !scanning && (
        <p className="muted small">
          Last scan{lastScan.cancelled ? " (stopped early)" : ""}: {lastScan.files_seen.toLocaleString()} files in{" "}
          {lastScan.seconds.toFixed(1)}s — {lastScan.added} new, {lastScan.changed} changed, {lastScan.removed} gone,{" "}
          {lastScan.decoded} analysed.
          {lastScan.missing > 0 && ` ${lastScan.missing} not found but kept (offline or removable).`}
          {lastScan.offline_sources.length > 0 && ` Skipped offline: ${lastScan.offline_sources.join(", ")}.`}
        </p>
      )}

      {status && status.ignored > 0 && (
        <p className="muted small">
          {status.ignored} "not duplicates" decision(s) saved.{" "}
          <a onClick={() => api.clearIgnored().then(onChanged).catch((e) => onError(String(e)))}>Reset</a>
        </p>
      )}
      {status && <p className="muted small">Index: {status.library_path}</p>}
    </div>
  );
}
