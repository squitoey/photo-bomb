import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, BatchResult, human, Op, Plan, SimilarOpts } from "./api";
import type { Marks } from "./App";

interface Props {
  marks: Marks;
  similarOpts: SimilarOpts;
  onClose: () => void;
  onDone: (doneIds: number[]) => void;
  onUndone: () => void;
}

type Choice = "trash" | "move" | "hardlink";

export function Review({ marks, similarOpts, onClose, onDone, onUndone }: Props) {
  const ids = useMemo(() => [...marks.keys()], [marks]);
  const [choice, setChoice] = useState<Choice>("trash");
  const [dest, setDest] = useState<string | null>(null);
  const [allowEmpty, setAllowEmpty] = useState(false);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<BatchResult | null>(null);
  const [undone, setUndone] = useState<BatchResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const op: Op | null = useMemo(() => {
    if (choice === "trash") return { type: "trash" };
    if (choice === "hardlink") return { type: "hardlink" };
    return dest ? { type: "move", dest } : null;
  }, [choice, dest]);

  // Dry run whenever the choice changes. Nothing on disk is touched here.
  useEffect(() => {
    if (result) return;
    setPlan(null);
    setError(null);
    if (!op) return;
    let live = true;
    api
      .plan(ids, op, similarOpts, allowEmpty)
      .then((p) => live && setPlan(p))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [ids, op, similarOpts, allowEmpty, result]);

  const pickDest = async () => {
    const picked = await open({ directory: true, title: "Move selected files to…" });
    if (typeof picked === "string") setDest(picked);
  };

  const apply = async () => {
    if (!op || !plan) return;
    setBusy(true);
    setError(null);
    try {
      const okIds = plan.items.filter((i) => i.status === "ok").map((i) => i.photo_id);
      const res = await api.execute(ids, op, similarOpts, allowEmpty);
      setResult(res);
      onDone(res.failed === 0 ? okIds : []);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const undo = async () => {
    if (!result) return;
    setBusy(true);
    try {
      setUndone(await api.undo(result.batch_id));
      onUndone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const problems = plan?.items.filter((i) => i.status !== "ok") ?? [];
  const okItems = plan?.items.filter((i) => i.status === "ok") ?? [];
  const verb = choice === "trash" ? "Move to Trash" : choice === "move" ? "Move files" : "Replace with hard links";

  return (
    <div className="modal-back" onClick={busy ? undefined : onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <strong>Review &amp; apply</strong>
          <button onClick={onClose} disabled={busy}>
            Close
          </button>
        </div>

        {result ? (
          <div className="modal-body">
            <h3>
              {result.done.toLocaleString()} file(s) processed · {human(result.bytes)}
            </h3>
            {result.skipped > 0 && <p className="muted">{result.skipped} skipped.</p>}
            {result.failed > 0 && <p className="error-text">{result.failed} failed:</p>}
            {result.errors.length > 0 && (
              <ul className="plain small">
                {result.errors.map((e, i) => (
                  <li key={i}>{e}</li>
                ))}
              </ul>
            )}
            {undone ? (
              <p>
                Undone: {undone.done} restored{undone.failed > 0 && `, ${undone.failed} could not be restored`}.
                {undone.errors.map((e, i) => (
                  <span key={i} className="error-text small">
                    <br />
                    {e}
                  </span>
                ))}
              </p>
            ) : (
              result.done > 0 && (
                <div className="row">
                  <button onClick={undo} disabled={busy}>
                    Undo
                  </button>
                  <span className="muted small">You can also undo later from History.</span>
                </div>
              )
            )}
          </div>
        ) : (
          <div className="modal-body">
            <div className="choices">
              <label className={choice === "trash" ? "on" : ""}>
                <input type="radio" checked={choice === "trash"} onChange={() => setChoice("trash")} />
                <div>
                  <strong>Move to Trash</strong>
                  <div className="muted small">Files go to the macOS Trash. Nothing is permanently deleted.</div>
                </div>
              </label>
              <label className={choice === "move" ? "on" : ""}>
                <input type="radio" checked={choice === "move"} onChange={() => setChoice("move")} />
                <div>
                  <strong>Move to a folder</strong>
                  <div className="muted small">
                    Keeps the folder structure. {dest ? <code>{dest}</code> : "No folder chosen."}{" "}
                    <a
                      onClick={(e) => {
                        e.preventDefault();
                        setChoice("move");
                        pickDest();
                      }}
                    >
                      Choose…
                    </a>
                  </div>
                </div>
              </label>
              <label className={choice === "hardlink" ? "on" : ""}>
                <input type="radio" checked={choice === "hardlink"} onChange={() => setChoice("hardlink")} />
                <div>
                  <strong>Replace with hard links</strong>
                  <div className="muted small">
                    Exact duplicates only. Every path stays, but copies share one set of bytes on disk. Editing one then
                    changes all of them.
                  </div>
                </div>
              </label>
            </div>

            {error && <p className="error-text">{error}</p>}
            {!op && <p className="muted">Choose a destination folder to continue.</p>}
            {op && !plan && !error && <p className="muted">Checking files…</p>}

            {plan && (
              <>
                <div className="summary">
                  <strong>{plan.ok.toLocaleString()}</strong> file(s) · <strong>{human(plan.bytes)}</strong>
                  {choice === "move" ? " to move" : " to free"}
                  {plan.skipped > 0 && <span className="muted"> · {plan.skipped} will be skipped</span>}
                  {plan.blocked > 0 && <span className="error-text"> · {plan.blocked} blocked</span>}
                </div>

                {plan.blocked > 0 && (
                  <div className="banner warn">
                    Some groups have every copy selected, so nothing would be kept. Unselect one photo in each, or
                    override below.
                  </div>
                )}
                {(plan.blocked > 0 || allowEmpty) && choice !== "hardlink" && (
                  <label className="check">
                    <input type="checkbox" checked={allowEmpty} onChange={(e) => setAllowEmpty(e.target.checked)} />
                    Allow removing every copy in a group
                  </label>
                )}

                {problems.length > 0 && (
                  <ul className="plain small scroll">
                    {problems.map((i) => (
                      <li key={i.photo_id}>
                        <span className={`tag ${i.status === "blocked" ? "warn" : ""}`}>{i.status}</span> {i.path}
                        <span className="muted"> — {i.note}</span>
                      </li>
                    ))}
                  </ul>
                )}
                <details>
                  <summary>Files to process ({okItems.length.toLocaleString()})</summary>
                  <ul className="plain small scroll">
                    {okItems.slice(0, 500).map((i) => (
                      <li key={i.photo_id}>
                        {i.path}
                        {i.target && <span className="muted"> → {i.target}</span>}
                      </li>
                    ))}
                    {okItems.length > 500 && <li className="muted">…and {okItems.length - 500} more</li>}
                  </ul>
                </details>
              </>
            )}

            <div className="row end">
              <button onClick={onClose} disabled={busy}>
                Cancel
              </button>
              <button className="primary" onClick={apply} disabled={busy || !plan || plan.ok === 0 || plan.blocked > 0}>
                {busy ? "Working…" : `${verb} (${plan?.ok ?? 0})`}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
