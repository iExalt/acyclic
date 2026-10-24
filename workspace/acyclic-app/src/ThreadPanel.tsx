// The picked thread: what approving it lets agents work on, its autonomy level and project-management flag, and the
// proposals agents made to it, each kept or dropped with one tap.

import { useEffect, useState } from "react";
import { type Command, type Proposal, proposals, type ThreadDetail, thread } from "./api";
import {
  approveCommand,
  dropCommand,
  keepCommand,
  LEVELS,
  type Level,
  pmCommand,
  revokeCommand,
  threadLevelCommand,
} from "./forms";

export interface ThreadPanelProps {
  id: string;
  globalLevel: string;
  titleOf: (id: string) => string;
  /** Changes after every edit sent, so the panel reads the thread again. */
  version: number;
  busy: boolean;
  onCommand: (command: Command) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
}

type Loaded = { detail: ThreadDetail; open: Proposal[] } | { failed: string } | null;

export function ThreadPanel({
  id,
  globalLevel,
  titleOf,
  version,
  busy,
  onCommand,
  onSelect,
  onClose,
}: ThreadPanelProps) {
  const [loaded, setLoaded] = useState<Loaded>(null);
  // A new version, after any edit, reads the thread again.
  useEffect(() => {
    let live = true;
    Promise.all([thread(id), proposals()]).then(
      ([detail, all]) => {
        if (live) setLoaded({ detail, open: all.filter((one) => one.decision === null && one.threads.includes(id)) });
      },
      (err: unknown) => {
        if (live) setLoaded({ failed: err instanceof Error ? err.message : String(err) });
      },
    );
    return () => {
      live = false;
    };
  }, [id, version]);
  if (loaded === null) return <aside className="inspector" data-testid="thread-panel" aria-busy="true" />;
  if ("failed" in loaded)
    return (
      <aside className="inspector" data-testid="thread-panel">
        <p role="alert">{loaded.failed}</p>
      </aside>
    );
  const { detail, open } = loaded;
  const members = detail.members.length;
  return (
    <aside className="inspector" data-testid="thread-panel" aria-label="Picked thread">
      <header>
        <h2>{detail.title}</h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          ×
        </button>
      </header>
      {detail.brief === null ? null : <p className="notes">{detail.brief}</p>}
      <p className="facts">
        {detail.approved ? "Approved" : "Not approved"} · {members} {members === 1 ? "item" : "items"}
      </p>
      {detail.heads.length === 0 ? null : (
        <div className="links">
          <h3>Ready to start</h3>
          <ul>
            {detail.heads.map((head) => (
              <li key={head}>
                <button type="button" className="chip" data-select={head} onClick={() => onSelect(head)}>
                  {titleOf(head)}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="actions">
        {detail.approved ? (
          <button type="button" data-action="revoke" disabled={busy} onClick={() => onCommand(revokeCommand(id))}>
            Revoke approval
          </button>
        ) : (
          <button type="button" data-action="approve" disabled={busy} onClick={() => onCommand(approveCommand(id))}>
            Approve {members} {members === 1 ? "item" : "items"}
          </button>
        )}
      </div>
      <label className="setting">
        Autonomy
        <select
          name="level"
          value={detail.level ?? ""}
          disabled={busy}
          onChange={(event) =>
            onCommand(threadLevelCommand(id, event.target.value === "" ? null : (event.target.value as Level)))
          }
        >
          <option value="">Global ({globalLevel})</option>
          {LEVELS.map((level) => (
            <option key={level} value={level}>
              {level}
            </option>
          ))}
        </select>
      </label>
      <label className="setting">
        <input
          type="checkbox"
          name="pm"
          checked={detail.pm}
          disabled={busy}
          onChange={(event) => onCommand(pmCommand(id, event.target.checked))}
        />{" "}
        Project management: its agent's edits to approved scope apply at once
      </label>
      {open.length === 0 ? null : (
        <div className="proposals" data-testid="proposals">
          <h3>Proposals</h3>
          <ul>
            {open.map((proposal) => (
              <li key={proposal.id} data-proposal={proposal.id}>
                <span className="grow">{describe(proposal, titleOf)}</span>
                <button
                  type="button"
                  data-action="keep"
                  disabled={busy}
                  onClick={() => onCommand(keepCommand(proposal.id))}
                >
                  Keep
                </button>
                <button
                  type="button"
                  data-action="drop"
                  disabled={busy}
                  onClick={() => onCommand(dropCommand(proposal.id))}
                >
                  Drop
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </aside>
  );
}

/** A proposal in a line: who proposed what, for which item. */
function describe(proposal: Proposal, titleOf: (id: string) => string): string {
  const { edit } = proposal;
  const item = edit.item === undefined ? "" : `${titleOf(edit.item)}: `;
  const shown = (value: unknown) => (value === null || value === undefined ? "none" : String(value));
  return `${item}${edit.field} ${shown(edit.before)} → ${shown(edit.after)} (${proposal.actor})`;
}
