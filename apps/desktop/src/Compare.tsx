import { useEffect, useState } from "react";
import { api, REVEAL_LABEL, dateTime, dirName, fileName, Group, human, Member, Photo, similarity } from "./api";
import { Thumb } from "./Thumb";
import type { Marks } from "./App";

interface Props {
  group: Group;
  /** Index of the member shown on the right at first. */
  start: number;
  marks: Marks;
  toggleMark: (p: Photo) => void;
  keepOnly: (id: number) => void;
  onInfo: (id: number) => void;
  onClose: () => void;
}

export function Compare({ group, start, marks, toggleMark, keepOnly, onInfo, onClose }: Props) {
  const members = group.members;
  const keeperIndex = Math.max(
    0,
    members.findIndex((m) => m.id === group.keeper),
  );
  const [left, setLeft] = useState(
    start === keeperIndex && members.length > 1 ? (keeperIndex + 1) % members.length : keeperIndex,
  );
  const [right, setRight] = useState(Math.min(start, members.length - 1));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowRight") setRight((r) => (r + 1) % members.length);
      else if (e.key === "ArrowLeft") setRight((r) => (r - 1 + members.length) % members.length);
      else if (e.key.toLowerCase() === "x") toggleMark(members[right]);
      else if (e.key.toLowerCase() === "k") keepOnly(members[right].id);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [members, right, onClose, toggleMark, keepOnly]);

  const a = members[Math.min(left, members.length - 1)];
  const b = members[Math.min(right, members.length - 1)];

  const pane = (m: Member, other: Member) => {
    const marked = marks.has(m.id);
    const px = (p: Member) => (p.width ?? 0) * (p.height ?? 0);
    const better = (mine: number, theirs: number) => (mine > theirs ? "better" : "");
    return (
      <div className={`pane ${marked ? "marked" : ""}`}>
        <div className="pane-img">
          <Thumb photo={m} full placeholder="No preview" />
          <span className={`badge ${marked ? "remove" : "keep"}`}>{marked ? "Remove" : "Keep"}</span>
        </div>
        <table className="facts">
          <tbody>
            <tr>
              <th>Name</th>
              <td>{fileName(m.path)}</td>
            </tr>
            <tr>
              <th>Folder</th>
              <td className="dir" title={dirName(m.path)}>
                {dirName(m.path)}
              </td>
            </tr>
            <tr>
              <th>Size</th>
              <td className={better(px(m), px(other))}>{m.width && m.height ? `${m.width} × ${m.height}` : "—"}</td>
            </tr>
            <tr>
              <th>File</th>
              <td className={better(m.size, other.size)}>
                {human(m.size)} · {(m.format ?? m.path.split(".").pop() ?? "").toUpperCase()}
              </td>
            </tr>
            <tr>
              <th>Taken</th>
              <td>{dateTime(m.taken_at)}</td>
            </tr>
          </tbody>
        </table>
        <div className="row">
          <button onClick={() => toggleMark(m)}>{marked ? "Keep this" : "Remove this"}</button>
          <button onClick={() => keepOnly(m.id)}>Keep only this</button>
          <button onClick={() => onInfo(m.id)}>Info</button>
          <button onClick={() => api.reveal(m.id)}>{REVEAL_LABEL}</button>
        </div>
      </div>
    );
  };

  return (
    <div className="modal-back" onClick={onClose}>
      <div className="modal wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <strong>
            Compare — {members.length} files
            {group.kind === "similar" && a.id !== b.id && a.distance != null && b.distance != null && (
              <span className="muted"> · right is {similarity(b.distance)}% similar to the suggested keeper</span>
            )}
          </strong>
          <span className="muted small">← → change right photo · X remove · K keep only · Esc close</span>
          <button onClick={onClose}>Close</button>
        </div>
        <div className="panes">
          {pane(a, b)}
          {pane(b, a)}
        </div>
        <div className="strip">
          {members.map((m, i) => (
            <div
              key={m.id}
              className={`strip-item ${i === left ? "is-left" : ""} ${i === right ? "is-right" : ""} ${marks.has(m.id) ? "marked" : ""}`}
              onClick={(e) => (e.shiftKey ? setLeft(i) : setRight(i))}
              title={`${m.path}\nClick: show on the right. Shift-click: show on the left.`}
            >
              <Thumb photo={m} placeholder="" />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
