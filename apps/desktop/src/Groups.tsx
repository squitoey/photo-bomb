import { useEffect, useRef, useState } from "react";
import {
  api,
  REVEAL_LABEL,
  dateTime,
  dirName,
  fileName,
  Group,
  GroupPage,
  human,
  Mark,
  Member,
  Photo,
  SimilarOpts,
  similarity,
} from "./api";
import type { Marks } from "./App";
import { Compare } from "./Compare";
import { PhotoMenu } from "./PhotoMenu";
import { Thumb } from "./Thumb";

const PAGE = 40;

const PRESETS: [number, string][] = [
  [0, "Identical image"],
  [4, "Strict"],
  [8, "Balanced"],
  [12, "Loose"],
  [16, "Very loose"],
];

const WINDOWS: [number | null, string][] = [
  [null, "Any time"],
  [10, "Within 10 seconds"],
  [60, "Within 1 minute"],
  [3600, "Within 1 hour"],
  [86400, "Within 1 day"],
];

interface Props {
  kind: "exact" | "similar";
  opts: SimilarOpts;
  onOpts: (o: SimilarOpts) => void;
  marks: Marks;
  toggleMark: (p: Photo) => void;
  setMarked: (photos: Mark[], marked: boolean) => void;
  revision: number;
  onInfo: (id: number) => void;
  onChanged: () => void;
  onError: (e: string) => void;
}

const asMark = (m: Member): Mark => ({ id: m.id, size: m.size, path: m.path });

export function Groups({ kind, opts, onOpts, marks, toggleMark, setMarked, revision, onInfo, onChanged, onError }: Props) {
  const [data, setData] = useState<GroupPage | null>(null);
  const [pages, setPages] = useState(1);
  const [loading, setLoading] = useState(false);
  const [comparing, setComparing] = useState<{ group: Group; start: number } | null>(null);
  const [slider, setSlider] = useState(opts.threshold);
  const request = useRef(0);

  // Debounce the slider so dragging stays smooth.
  useEffect(() => {
    if (slider === opts.threshold) return;
    const t = setTimeout(() => onOpts({ ...opts, threshold: slider }), 120);
    return () => clearTimeout(t);
  }, [slider, opts, onOpts]);

  useEffect(() => {
    const id = ++request.current;
    setLoading(true);
    const load = kind === "exact" ? api.exactGroups(0, PAGE * pages) : api.similarGroups(opts, 0, PAGE * pages);
    load
      .then((p) => {
        if (id === request.current) setData(p);
      })
      .catch((e) => onError(String(e)))
      .finally(() => {
        if (id === request.current) setLoading(false);
      });
  }, [kind, opts, pages, revision, onError]);

  // Keep an open compare view in step with reloaded data.
  useEffect(() => {
    if (!comparing || !data) return;
    const fresh = data.groups.find((g) => g.id === comparing.group.id);
    if (fresh && fresh !== comparing.group) setComparing({ group: fresh, start: comparing.start });
  }, [data, comparing]);

  const autoSelect = async () => {
    try {
      setMarked(await api.autoSelect(kind, kind === "similar" ? opts : null), true);
    } catch (e) {
      onError(String(e));
    }
  };

  const ignore = async (g: Group) => {
    try {
      await api.ignoreGroup(g.members.map((m) => m.id));
      setMarked(g.members.map(asMark), false);
      onChanged();
    } catch (e) {
      onError(String(e));
    }
  };

  /** Takes one photo out of its group; the rest stay grouped. */
  const ignoreOne = async (g: Group, m: Member) => {
    try {
      await api.ignorePhoto(m.id, g.members.map((x) => x.id));
      setMarked([asMark(m)], false);
      onChanged();
    } catch (e) {
      onError(String(e));
    }
  };

  const keepOnly = (g: Group, keepId: number) => {
    setMarked(g.members.filter((m) => m.id === keepId).map(asMark), false);
    setMarked(g.members.filter((m) => m.id !== keepId).map(asMark), true);
  };

  const preset = PRESETS.reduce((best, p) => (Math.abs(p[0] - slider) < Math.abs(best[0] - slider) ? p : best));

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h2>{kind === "exact" ? "Exact duplicates" : "Similar photos"}</h2>
          <p className="muted">
            {kind === "exact"
              ? "Files with identical bytes. Removing a copy loses nothing."
              : "Photos that look alike but are different files. Check before removing."}
          </p>
        </div>
        <div className="row">
          <button onClick={autoSelect} disabled={!data || data.total_groups === 0} title="Keep the suggested photo in every group and select the rest">
            Auto-select all
          </button>
        </div>
      </header>

      {kind === "similar" && (
        <div className="card controls">
          <label className="slider">
            <span>
              Similarity: <strong>{preset[1]}</strong> <span className="muted">({slider})</span>
            </span>
            <div className="slider-row">
              <span className="muted small">Strict</span>
              <input type="range" min={0} max={16} step={1} value={slider} onChange={(e) => setSlider(Number(e.target.value))} />
              <span className="muted small">Loose</span>
            </div>
          </label>
          <label className="check">
            <input type="checkbox" checked={opts.same_aspect} onChange={(e) => onOpts({ ...opts, same_aspect: e.target.checked })} />
            Same shape only
          </label>
          <label>
            Taken{" "}
            <select
              value={opts.time_window_secs ?? ""}
              onChange={(e) => onOpts({ ...opts, time_window_secs: e.target.value === "" ? null : Number(e.target.value) })}
            >
              {WINDOWS.map(([v, label]) => (
                <option key={label} value={v ?? ""}>
                  {label}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}

      {data && (
        <div className="summary">
          <strong>{data.total_groups.toLocaleString()}</strong> groups ·{" "}
          <strong>{data.total_photos.toLocaleString()}</strong> files ·{" "}
          <strong>{human(data.total_reclaimable)}</strong> reclaimable
          {loading && <span className="spinner inline" />}
        </div>
      )}

      {data && data.total_groups === 0 && !loading && (
        <div className="empty">
          <h3>Nothing here</h3>
          <p>
            {kind === "exact"
              ? "No byte-identical files were found. Scan a folder from Sources if you have not yet."
              : "No similar photos at this level. Try moving the slider toward Loose."}
          </p>
        </div>
      )}

      {data?.groups.map((g) => {
        const markedCount = g.members.filter((m) => marks.has(m.id)).length;
        const allMarked = markedCount === g.members.length;
        return (
          <section key={g.id} className={`card group ${allMarked ? "danger" : ""}`}>
            <div className="group-head">
              <div>
                <strong>{g.members.length} files</strong>
                <span className="muted"> · {human(g.reclaimable)} reclaimable</span>
                {allMarked && <span className="tag warn">nothing kept</span>}
              </div>
              <div className="row">
                <button onClick={() => keepOnly(g, g.keeper)}>Keep suggested</button>
                <button onClick={() => setComparing({ group: g, start: g.members.length > 1 ? 1 : 0 })}>Compare</button>
                <button onClick={() => ignore(g)} title="Stop showing any of these as duplicates of each other. To take out a single photo, use its ⋯ menu.">
                  None are duplicates
                </button>
              </div>
            </div>
            <div className="tiles-grid">
              {g.members.map((m, i) => {
                const marked = marks.has(m.id);
                return (
                  <div
                    key={m.id}
                    className={`photo ${marked ? "marked" : ""}`}
                    onClick={() => toggleMark(m)}
                    onDoubleClick={() => setComparing({ group: g, start: i })}
                    title={m.path}
                  >
                    <div className="thumb">
                      <Thumb photo={m} lazy />
                      {marked ? (
                        <span className="badge remove">Remove</span>
                      ) : m.id === g.keeper ? (
                        <span className="badge keep">Keep</span>
                      ) : null}
                    </div>
                    <PhotoMenu
                      items={[
                        { label: "Info", hint: "Show all details of this file", onClick: () => onInfo(m.id) },
                        { label: "Compare", onClick: () => setComparing({ group: g, start: i }) },
                        { label: REVEAL_LABEL, onClick: () => api.reveal(m.id).catch((e) => onError(String(e))) },
                        {
                          label: "Not a duplicate",
                          hint: "Take only this photo out of the group. The others stay grouped.",
                          onClick: () => ignoreOne(g, m),
                        },
                      ]}
                    />
                    <div className="meta">
                      <div className="name">{fileName(m.path)}</div>
                      <div className="muted small">
                        {m.width && m.height ? `${m.width}×${m.height} · ` : ""}
                        {human(m.size)}
                        {g.kind === "similar" && m.id !== g.keeper && m.distance != null && ` · ${similarity(m.distance)}%`}
                      </div>
                      {m.taken_at && <div className="muted small">{dateTime(m.taken_at)}</div>}
                      <div className="muted small dir">{dirName(m.path)}</div>
                      <div>
                        {m.exact_copy && g.kind === "similar" && <span className="tag">identical bytes</span>}
                        {m.hardlinked && <span className="tag">hard link</span>}
                        {m.missing && <span className="tag warn">offline</span>}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
        );
      })}

      {data && data.groups.length < data.total_groups && (
        <div className="center">
          <button onClick={() => setPages((p) => p + 1)} disabled={loading}>
            Show more ({(data.total_groups - data.groups.length).toLocaleString()} left)
          </button>
        </div>
      )}

      {comparing && (
        <Compare
          group={comparing.group}
          start={comparing.start}
          marks={marks}
          toggleMark={toggleMark}
          keepOnly={(id) => keepOnly(comparing.group, id)}
          onInfo={onInfo}
          onClose={() => setComparing(null)}
        />
      )}
    </div>
  );
}
