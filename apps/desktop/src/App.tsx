import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, AppStatus, human, Mark, Photo, ScanFinished, ScanProgress, ScanSummary, SimilarOpts } from "./api";
import { Sources } from "./Sources";
import { Groups } from "./Groups";
import { Review } from "./Review";
import { History } from "./History";
import { Others } from "./Others";
import { Info } from "./Info";

type View = "sources" | "exact" | "similar" | "others" | "history";

/** Photos marked for removal, across every view. */
export type Marks = Map<number, Mark>;

export default function App() {
  const [view, setView] = useState<View>("sources");
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [lastScan, setLastScan] = useState<ScanSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [marks, setMarks] = useState<Marks>(new Map());
  const [similarOpts, setSimilarOpts] = useState<SimilarOpts>({
    threshold: 8,
    same_aspect: false,
    time_window_secs: null,
  });
  const [counts, setCounts] = useState<{ exact: number | null; similar: number | null; others: number | null }>({
    exact: null,
    similar: null,
    others: null,
  });
  const [infoId, setInfoId] = useState<number | null>(null);
  const [reviewing, setReviewing] = useState(false);
  // Bumped whenever the index changes, so lists reload.
  const [revision, setRevision] = useState(0);

  const refresh = useCallback(() => {
    api
      .status()
      .then(setStatus)
      .catch((e) => setError(String(e)));
    setRevision((r) => r + 1);
  }, []);

  // While a scan runs, results are saved as they are found; reload the lists now and then.
  const lastLiveRefresh = useRef(0);

  useEffect(() => {
    refresh();
    const unlisten = [
      listen<ScanProgress>("scan-progress", (e) => {
        setProgress(e.payload);
        const now = Date.now();
        if (now - lastLiveRefresh.current > 8000) {
          lastLiveRefresh.current = now;
          refresh();
        }
      }),
      listen<ScanFinished>("scan-finished", (e) => {
        setProgress(null);
        if (e.payload.error) setError(e.payload.error);
        if (e.payload.summary) setLastScan(e.payload.summary);
        refresh();
      }),
    ];
    return () => {
      unlisten.forEach((p) => p.then((f) => f()));
    };
  }, [refresh]);

  // Sidebar counts.
  useEffect(() => {
    api
      .exactGroups(0, 0)
      .then((p) => setCounts((c) => ({ ...c, exact: p.total_groups })))
      .catch(() => {});
  }, [revision]);
  useEffect(() => {
    api
      .similarGroups(similarOpts, 0, 0)
      .then((p) => setCounts((c) => ({ ...c, similar: p.total_groups })))
      .catch(() => {});
    api
      .otherPhotos(similarOpts, 0, 0)
      .then((p) => setCounts((c) => ({ ...c, others: p.total })))
      .catch(() => {});
  }, [revision, similarOpts]);

  const startScan = useCallback(() => {
    setError(null);
    setProgress({ stage: "Starting", done: 0, total: 0 });
    api.startScan().catch((e) => {
      setProgress(null);
      setError(String(e));
    });
  }, []);

  const toggleMark = useCallback((p: Photo) => {
    setMarks((prev) => {
      const next = new Map(prev);
      if (next.has(p.id)) next.delete(p.id);
      else next.set(p.id, { id: p.id, size: p.size, path: p.path });
      return next;
    });
  }, []);

  const setMarked = useCallback((photos: Mark[], marked: boolean) => {
    setMarks((prev) => {
      const next = new Map(prev);
      for (const p of photos) {
        if (marked) next.set(p.id, p);
        else next.delete(p.id);
      }
      return next;
    });
  }, []);

  const markedBytes = [...marks.values()].reduce((s, m) => s + m.size, 0);
  const scanning = progress !== null || !!status?.scanning;

  const nav = (id: View, label: string, count?: number | null) => (
    <button className={`nav ${view === id ? "active" : ""}`} onClick={() => setView(id)}>
      <span>{label}</span>
      {count != null && <span className="count">{count}</span>}
    </button>
  );

  return (
    <div className="app">
      <aside className="sidebar">
        <h1>Photo Bomb</h1>
        {nav("sources", "Sources", status?.sources)}
        {nav("exact", "Exact duplicates", counts.exact)}
        {nav("similar", "Similar photos", counts.similar)}
        {nav("others", "Other photos", counts.others)}
        {nav("history", "History")}
        <div className="sidebar-foot">
          {status && (
            <>
              <div>{status.photos.toLocaleString()} files</div>
              <div>{human(status.bytes)}</div>
            </>
          )}
        </div>
      </aside>

      <main className="main">
        {error && (
          <div className="banner error">
            <span>{error}</span>
            <button onClick={() => setError(null)}>Dismiss</button>
          </div>
        )}
        {view === "sources" && (
          <Sources
            status={status}
            scanning={scanning}
            progress={progress}
            lastScan={lastScan}
            revision={revision}
            onScan={startScan}
            onChanged={refresh}
            onError={setError}
          />
        )}
        {(view === "exact" || view === "similar") && (
          <Groups
            key={view}
            kind={view}
            opts={similarOpts}
            onOpts={setSimilarOpts}
            marks={marks}
            toggleMark={toggleMark}
            setMarked={setMarked}
            revision={revision}
            onInfo={setInfoId}
            onChanged={refresh}
            onError={setError}
          />
        )}
        {view === "others" && (
          <Others opts={similarOpts} revision={revision} onInfo={setInfoId} onChanged={refresh} onError={setError} />
        )}
        {view === "history" && <History revision={revision} onChanged={refresh} onError={setError} />}
      </main>

      <footer className="bottombar">
        <div className="scan-state">
          {scanning && progress ? (
            <>
              <span className="spinner" />
              {progress.stage}
              {progress.total > 0
                ? ` ${progress.done.toLocaleString()} / ${progress.total.toLocaleString()}`
                : progress.done > 0
                  ? ` ${progress.done.toLocaleString()}`
                  : "…"}
            </>
          ) : (
            "Ready"
          )}
        </div>
        <div className="selection">
          {marks.size > 0 ? (
            <>
              <span>
                {marks.size.toLocaleString()} selected · {human(markedBytes)}
              </span>
              <button onClick={() => setMarks(new Map())}>Clear</button>
              <button className="primary" disabled={scanning} onClick={() => setReviewing(true)}>
                Review &amp; apply…
              </button>
            </>
          ) : (
            <span className="muted">Click photos to select the ones to remove</span>
          )}
        </div>
      </footer>

      {infoId !== null && <Info id={infoId} onClose={() => setInfoId(null)} onChanged={refresh} />}

      {reviewing && (
        <Review
          marks={marks}
          similarOpts={similarOpts}
          onClose={() => setReviewing(false)}
          onDone={(doneIds) => {
            setMarks((prev) => {
              const next = new Map(prev);
              doneIds.forEach((id) => next.delete(id));
              return next;
            });
            refresh();
          }}
          onUndone={refresh}
        />
      )}
    </div>
  );
}
