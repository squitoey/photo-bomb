import { useEffect, useState } from "react";
import { api, HistoryEntry, human } from "./api";

const LABELS: Record<string, string> = {
  trash: "Moved to Trash",
  move: "Moved to folder",
  hardlink: "Replaced with hard links",
};

interface Props {
  revision: number;
  onChanged: () => void;
  onError: (e: string) => void;
}

export function History({ revision, onChanged, onError }: Props) {
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    api.history().then(setEntries).catch((e) => onError(String(e)));
  }, [revision, onError]);

  const undo = async (h: HistoryEntry) => {
    setBusy(h.batch_id);
    setNote(null);
    try {
      const r = await api.undo(h.batch_id);
      setNote(
        `Restored ${r.done} file(s)` +
          (r.failed > 0 ? `; ${r.failed} could not be restored: ${r.errors.slice(0, 3).join("; ")}` : "."),
      );
      onChanged();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h2>History</h2>
          <p className="muted">Everything the app has done to your files. Each entry can be undone.</p>
        </div>
      </header>
      {note && <div className="banner">{note}</div>}
      {entries && entries.length === 0 && (
        <div className="empty">
          <h3>No actions yet</h3>
          <p>Nothing has been moved, trashed or linked.</p>
        </div>
      )}
      {entries && entries.length > 0 && (
        <div className="card list">
          {entries.map((h) => (
            <div key={h.batch_id} className="list-row">
              <div className="grow">
                <div>
                  <strong>{LABELS[h.op] ?? h.op}</strong> · {h.files.toLocaleString()} file(s) · {human(h.bytes)}
                </div>
                <div className="muted small">
                  {new Date(h.created_at * 1000).toLocaleString()}
                  {h.undone > 0 && ` · ${h.undone} undone`}
                  {h.failed > 0 && ` · ${h.failed} failed`}
                </div>
              </div>
              <button onClick={() => undo(h)} disabled={h.done === 0 || busy !== null}>
                {busy === h.batch_id ? "Undoing…" : h.done === 0 ? "Undone" : "Undo"}
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
