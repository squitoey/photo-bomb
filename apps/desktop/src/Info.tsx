import { useEffect, useState } from "react";
import { api, dateTime, dirName, fileName, fullUrl, human, PhotoDetails, thumbUrl } from "./api";

interface Props {
  id: number;
  onClose: () => void;
  onChanged: () => void;
}

const STATES: Record<string, string> = {
  present: "In the library",
  trashed: "Moved to Trash by this app",
  moved: "Moved to a folder by this app",
};

/** Everything the index knows about one photo, plus the EXIF read from the file. */
export function Info({ id, onClose, onChanged }: Props) {
  const [d, setD] = useState<PhotoDetails | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);

  useEffect(() => {
    setError(null);
    api.photoDetails(id).then(setD).catch((e) => setError(String(e)));
  }, [id, reload]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopImmediatePropagation();
        onClose();
      }
    };
    // Capture, so Escape closes this panel before anything underneath it.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const row = (label: string, value: React.ReactNode, mono = false) => (
    <tr>
      <th>{label}</th>
      <td className={mono ? "mono" : ""}>{value}</td>
    </tr>
  );

  const body = () => {
    if (!d) return <div className="muted">{error ?? "Loading…"}</div>;
    const mp = d.width && d.height ? (d.width * d.height) / 1e6 : null;
    const analysed =
      d.decoded === 1 ? "Yes" : d.decoded === 2 ? `Could not be read: ${d.decode_error ?? "unknown error"}` : "Not yet (or not an image)";
    return (
      <div className="info">
        <div className="info-img">
          {d.decoded === 1 ? (
            <img src={d.exists ? fullUrl(d) : thumbUrl(d)} alt="" />
          ) : (
            <div className="noimg">No preview</div>
          )}
        </div>
        <div className="info-facts">
          <h4>File</h4>
          <table className="facts wide">
            <tbody>
              {row("Name", fileName(d.path))}
              {row("Folder", dirName(d.path))}
              {row("Source", d.source_root + (d.source_removable ? " (removable)" : ""))}
              {row("Size", `${human(d.size)} (${d.size.toLocaleString()} bytes)`)}
              {row("Format", (d.format ?? d.path.split(".").pop() ?? "").toUpperCase())}
              {row("Dimensions", d.width && d.height ? `${d.width} × ${d.height}${mp ? ` (${mp.toFixed(1)} MP)` : ""}` : "—")}
              {row("Taken", dateTime(d.taken_at))}
              {row("Modified", new Date(d.mtime_ns / 1e6).toLocaleString())}
              {row(
                "On disk now",
                d.exists ? "Yes" : d.missing ? "No — not found at the last scan; kept in the index" : "No",
              )}
              {row("Status", STATES[d.state] ?? d.state)}
              {row("Duplicate matching", d.not_dup ? "Left out — marked “not a duplicate”" : "Included")}
            </tbody>
          </table>

          <h4>Fingerprints</h4>
          <table className="facts wide">
            <tbody>
              {row("Content hash", d.content_hash ?? "Not computed — no other file has the same size", !!d.content_hash)}
              {row("Partial hash", d.partial_hash ?? "—", !!d.partial_hash)}
              {row("Visual hash", d.phash ? `${d.phash} · ${d.dhash}` : "—", !!d.phash)}
              {row("Analysed", analysed)}
              {row("Device / inode", `${d.dev} / ${d.ino}`, true)}
              {row("Index id", String(d.id), true)}
            </tbody>
          </table>

          <h4>Camera data (EXIF)</h4>
          {d.exif.length === 0 ? (
            <p className="muted small">{d.exists ? "This file has no EXIF data." : "The file is not available, so its EXIF data cannot be read."}</p>
          ) : (
            <table className="facts wide">
              <tbody>{d.exif.map(([k, v]) => <tr key={k}><th>{k}</th><td>{v}</td></tr>)}</tbody>
            </table>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="modal-back top" onClick={onClose}>
      <div className="modal info-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <strong>{d ? fileName(d.path) : "Photo info"}</strong>
          <div className="row">
            {d?.not_dup && (
              <button
                onClick={() =>
                  api
                    .unignorePhoto(d.id)
                    .then(() => {
                      setReload((r) => r + 1);
                      onChanged();
                    })
                    .catch((e) => setError(String(e)))
                }
              >
                Include in duplicate matching
              </button>
            )}
            {d && (
              <button onClick={() => api.reveal(d.id)} disabled={!d.exists}>
                Show in Finder
              </button>
            )}
            <button onClick={onClose}>Close</button>
          </div>
        </div>
        <div className="modal-body selectable">
          {d && error && <div className="banner error">{error}</div>}
          {body()}
        </div>
      </div>
    </div>
  );
}
