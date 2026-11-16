import { useEffect, useRef, useState } from "react";
import { api, REVEAL_LABEL, dateTime, dirName, fileName, human, PhotoPage, SimilarOpts } from "./api";
import { PhotoMenu } from "./PhotoMenu";
import { Thumb } from "./Thumb";

const PAGE = 120;

interface Props {
  opts: SimilarOpts;
  revision: number;
  onInfo: (id: number) => void;
  onChanged: () => void;
  onError: (e: string) => void;
}

/** Photos that are in no exact group and no similar group. */
export function Others({ opts, revision, onInfo, onChanged, onError }: Props) {
  const [data, setData] = useState<PhotoPage | null>(null);
  const [pages, setPages] = useState(1);
  const [loading, setLoading] = useState(false);
  const request = useRef(0);

  useEffect(() => {
    const id = ++request.current;
    setLoading(true);
    api
      .otherPhotos(opts, 0, PAGE * pages)
      .then((p) => {
        if (id === request.current) setData(p);
      })
      .catch((e) => onError(String(e)))
      .finally(() => {
        if (id === request.current) setLoading(false);
      });
  }, [opts, pages, revision, onError]);

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h2>Other photos</h2>
          <p className="muted">
            Files that are not an exact duplicate of anything and not in a similar group at the current similarity
            level. Click a photo for its details.
          </p>
        </div>
      </header>

      {data && (
        <div className="summary">
          <strong>{data.total.toLocaleString()}</strong> files · <strong>{human(data.total_bytes)}</strong>
          {loading && <span className="spinner inline" />}
        </div>
      )}

      {data && data.total === 0 && !loading && (
        <div className="empty">
          <h3>Nothing here</h3>
          <p>Every indexed file is in a duplicate or similar group, or nothing has been scanned yet.</p>
        </div>
      )}

      {data && data.photos.length > 0 && (
        <div className="card">
          <div className="tiles-grid">
            {data.photos.map((p) => (
              <div key={p.id} className="photo" onClick={() => onInfo(p.id)} title={p.path}>
                <div className="thumb">
                  <Thumb photo={p} lazy />
                </div>
                <PhotoMenu
                  items={[
                    { label: "Info", onClick: () => onInfo(p.id) },
                    { label: REVEAL_LABEL, onClick: () => api.reveal(p.id).catch((e) => onError(String(e))) },
                    ...(p.not_dup
                      ? [
                          {
                            label: "Include in duplicate matching",
                            hint: "Undo “not a duplicate” for this file",
                            onClick: () => api.unignorePhoto(p.id).then(onChanged).catch((e) => onError(String(e))),
                          },
                        ]
                      : []),
                  ]}
                />
                <div className="meta">
                  <div className="name">{fileName(p.path)}</div>
                  <div className="muted small">
                    {p.width && p.height ? `${p.width}×${p.height} · ` : ""}
                    {human(p.size)}
                  </div>
                  {p.taken_at && <div className="muted small">{dateTime(p.taken_at)}</div>}
                  <div className="muted small dir">{dirName(p.path)}</div>
                  <div>
                    {p.not_dup && <span className="tag">not a duplicate</span>}
                    {p.missing && <span className="tag warn">offline</span>}
                    {p.decoded === 2 && <span className="tag warn">unreadable</span>}
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {data && data.photos.length < data.total && (
        <div className="center">
          <button onClick={() => setPages((n) => n + 1)} disabled={loading}>
            Show more ({(data.total - data.photos.length).toLocaleString()} left)
          </button>
        </div>
      )}
    </div>
  );
}
