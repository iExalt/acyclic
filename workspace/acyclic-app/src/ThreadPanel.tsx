// The picked thread: what approving it lets agents work on, its autonomy level and project-management flag, and the
// proposals agents made to it, each kept or dropped with one tap.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { type Command, failure, type Proposal, proposals, type ThreadDetail, thread } from "./api";
import { Badge, lineStyle } from "./Badge";
import { Choice } from "./Choice";
import { useFocusIn } from "./focus";
import {
  approveCommand,
  dropCommand,
  keepCommand,
  LEVELS,
  type Level,
  levelLabel,
  pmCommand,
  revokeCommand,
  threadLevelCommand,
} from "./forms";
import type { Line } from "./lines";
import { Prose } from "./Prose";

export interface ThreadPanelProps {
  id: string;
  globalLevel: string;
  /** The thread's line, for its badge. */
  line: Line | undefined;
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
  line,
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
        if (live) setLoaded({ failed: failure(err).message });
      },
    );
    return () => {
      live = false;
    };
  }, [id, version]);
  // One panel throughout, so focus put in it as it opens stays as it loads.
  const panel = useRef<HTMLElement>(null);
  useFocusIn(panel);
  const shell = (body: ReactNode) => (
    <aside
      ref={panel}
      tabIndex={-1}
      className="inspector"
      data-testid="thread-panel"
      aria-label="Picked thread"
      aria-busy={loaded === null ? true : undefined}
    >
      {body}
    </aside>
  );
  if (loaded === null) return shell(null);
  if ("failed" in loaded) return shell(<p role="alert">{loaded.failed}</p>);
  const { detail, open } = loaded;
  const members = detail.members.length;
  return shell(
    <>
      <header>
        <h2 className="line" style={lineStyle(line)}>
          <Badge line={line} />
          <Prose text={line?.name ?? detail.title} />
        </h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          ×
        </button>
      </header>
      <p className="facts">
        {members} {members === 1 ? "item" : "items"}, {detail.approved ? "approved" : "not approved yet"}
      </p>
      <div className="actions">
        {detail.approved ? (
          <button type="button" data-action="revoke" disabled={busy} onClick={() => onCommand(revokeCommand(id))}>
            Revoke approval
          </button>
        ) : (
          <button
            type="button"
            className="primary"
            data-action="approve"
            disabled={busy}
            onClick={() => onCommand(approveCommand(id))}
          >
            Approve {members} {members === 1 ? "item" : "items"}
          </button>
        )}
      </div>
      <Choice
        label="Autonomy"
        className="setting"
        name="level"
        value={detail.level ?? ""}
        disabled={busy}
        onWrite={(level) => onCommand(threadLevelCommand(id, level === "" ? null : (level as Level)))}
      >
        <option value="">Global ({levelLabel(globalLevel)})</option>
        {LEVELS.map((level) => (
          <option key={level} value={level}>
            {levelLabel(level)}
          </option>
        ))}
      </Choice>
      {/* The project-management flag, said as what it does to an agent's edits. */}
      <Choice
        label="Agent edits to approved scope"
        className="setting"
        name="pm"
        value={detail.pm ? "apply" : "wait"}
        disabled={busy}
        onWrite={(choice) => onCommand(pmCommand(id, choice === "apply"))}
      >
        <option value="wait">Wait for your OK</option>
        <option value="apply">Apply at once</option>
      </Choice>
      {open.length === 0 ? null : (
        <div className="proposals" data-testid="proposals">
          <h3>Proposals</h3>
          <ul>
            {open.map((proposal) => (
              <li key={proposal.id} data-proposal={proposal.id}>
                <span className="grow">{describe(proposal, titleOf)}</span>
                <button
                  type="button"
                  className="primary"
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
      {detail.brief === null ? null : (
        <p className="notes">
          <Prose text={detail.brief} />
        </p>
      )}
      {detail.heads.length === 0 ? null : (
        <div className="heads">
          <h3>Ready to start</h3>
          <ul className="rows">
            {detail.heads.map((head) => (
              <li key={head}>
                <button type="button" className="row" data-select={head} onClick={() => onSelect(head)}>
                  <Prose text={titleOf(head)} />
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </>,
  );
}

/** A proposal in a line: who proposed what, for which item. */
function describe(proposal: Proposal, titleOf: (id: string) => string): string {
  const { edit } = proposal;
  const item = edit.item === undefined ? "" : `${titleOf(edit.item)}: `;
  const shown = (value: unknown) => (value === null || value === undefined ? "none" : String(value));
  return `${item}${edit.field} ${shown(edit.before)} → ${shown(edit.after)} (${proposal.actor})`;
}
