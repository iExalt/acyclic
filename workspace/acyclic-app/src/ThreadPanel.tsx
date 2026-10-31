// The picked thread: what approving it lets agents work on, its autonomy level and project-management flag, and the
// proposals agents made to it, each kept or dropped with one tap.

import { type ReactNode, useEffect, useRef, useState } from "react";
import {
  type Command,
  type Counts,
  failure,
  type Proposal,
  proposals,
  type Stage,
  type ThreadDetail,
  thread,
} from "./api";
import { Badge, lineStyle } from "./Badge";
import { Choice } from "./Choice";
import { CopyLink } from "./CopyLink";
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
import { Check, CircleAlert, Icon, X } from "./icons";
import type { Line } from "./lines";
import { Prose } from "./Prose";
import { StageCounts, StageIcon } from "./Stage";

export interface ThreadPanelProps {
  id: string;
  globalLevel: string;
  /** The thread's line, for its badge. */
  line: Line | undefined;
  /** Its leaves by stage, or undefined when it has none drawn. */
  counts: Counts | undefined;
  titleOf: (id: string) => string;
  stageOf: (id: string) => Stage | undefined;
  /** Changes after every edit sent, so the panel reads the thread again. */
  version: number;
  busy: boolean;
  onCommand: (command: Command) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
  /** A refusal or failure of what this panel sent, shown in it. */
  notice?: ReactNode;
  /** Copies the address that opens this again. */
  onCopyLink: () => Promise<boolean>;
}

type Loaded = { detail: ThreadDetail; open: Proposal[] } | { failed: string } | null;

export function ThreadPanel({
  id,
  globalLevel,
  line,
  counts,
  titleOf,
  stageOf,
  version,
  busy,
  onCommand,
  onSelect,
  onClose,
  notice,
  onCopyLink,
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
        <CopyLink onCopy={onCopyLink} />
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          <Icon of={X} />
        </button>
      </header>
      {notice}
      <p className={detail.approved ? "thread-status" : "thread-status waiting"} data-testid="thread-status">
        <Icon of={detail.approved ? Check : CircleAlert} size={16} />
        {/* No total beside it: the counts below are of the tasks drawn, which a total of every member would not add
            up to. */}
        {detail.approved ? "Approved" : "Waiting for your approval"}
      </p>
      <div className="progress">
        {counts === undefined || counts.leaves === 0 ? (
          <span className="muted">No tasks shown</span>
        ) : (
          <StageCounts counts={counts} />
        )}
      </div>
      {detail.approved ? null : (
        <div className="actions">
          <button
            type="button"
            className="primary"
            data-action="approve"
            disabled={busy}
            onClick={() => onCommand(approveCommand(id))}
          >
            Approve {members} {members === 1 ? "item" : "items"}
          </button>
        </div>
      )}
      {detail.heads.length === 0 ? null : (
        <div className="heads">
          <h3>Unblocked</h3>
          <ul className="rows">
            {detail.heads.map((head) => {
              const stage = stageOf(head);
              return (
                <li key={head}>
                  <button type="button" className="row" data-select={head} onClick={() => onSelect(head)}>
                    {stage === undefined ? null : <StageIcon stage={stage} />}
                    <Prose text={titleOf(head)} />
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
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
                  className="quiet"
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
        <div className="notes-block">
          <h3>Brief</h3>
          <p className="notes">
            <Prose text={detail.brief} />
          </p>
        </div>
      )}
      <div className="settings">
        <h3>Settings</h3>
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
      </div>
      {detail.approved ? (
        <div className="actions">
          <button
            type="button"
            className="danger-quiet"
            data-action="revoke"
            disabled={busy}
            onClick={() => onCommand(revokeCommand(id))}
          >
            Revoke approval
          </button>
        </div>
      ) : null}
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
