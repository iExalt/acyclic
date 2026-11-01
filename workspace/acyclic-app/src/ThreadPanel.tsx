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
  levelHint,
  levelLabel,
  pmCommand,
  revokeCommand,
  threadLevelCommand,
} from "./forms";
import { useHeld } from "./held";
import { Check, CircleAlert, Icon, X } from "./icons";
import type { Line } from "./lines";
import { Prose } from "./Prose";
import { useSheet } from "./Sheet";
import { StageCounts, StageIcon } from "./Stage";

export interface ThreadPanelProps {
  id: string;
  globalLevel: string;
  /** Needs you asks the user to approve it: it is unapproved with work open. */
  awaiting: boolean;
  /** The thread's line, for its badge. */
  line: Line | undefined;
  /** Its leaves by stage, or undefined when it has none drawn. */
  counts: Counts | undefined;
  titleOf: (id: string) => string;
  stageOf: (id: string) => Stage | undefined;
  /** Changes after every edit sent, so the panel reads the thread again. */
  version: number;
  busy: boolean;
  /** Sends an edit, with what to say once it is saved when that is all that shows it was. */
  onCommand: (command: Command, done?: string) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
  /** Closes it as Escape does, for a drag down on a phone: a form holding typing stays, saying how to leave it. */
  onDismiss: () => void;
  /** A refusal or failure of what this panel sent, shown in it. */
  notice?: ReactNode;
  /** Copies the address that opens this again. */
  onCopyLink: () => Promise<boolean>;
}

type Loaded = { detail: ThreadDetail; open: Proposal[] } | { failed: string } | null;

export function ThreadPanel({
  id,
  globalLevel,
  awaiting,
  line,
  counts,
  titleOf,
  stageOf,
  version,
  busy,
  onCommand,
  onSelect,
  onClose,
  onDismiss,
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
  const sheet = useSheet(panel, "half", onDismiss);
  useFocusIn(panel);
  // A decision takes its button away once the thread is read again: the keyboard stays in the panel.
  const held = useHeld(busy, loaded);
  const toPanel = () => panel.current;
  const shell = (body: ReactNode) => (
    <aside
      ref={panel}
      tabIndex={-1}
      className="inspector"
      data-testid="thread-panel"
      aria-label="Picked thread"
      aria-busy={loaded === null ? true : undefined}
      data-detent={sheet.detent}
    >
      {sheet.handle}
      {body}
    </aside>
  );
  if (loaded === null) return shell(null);
  if ("failed" in loaded) return shell(<p role="alert">{loaded.failed}</p>);
  const { detail, open } = loaded;
  const members = detail.members.length;
  const name = line?.name ?? detail.title;
  const items = `${members} ${members === 1 ? "item" : "items"}`;
  /** After a proposal is settled, the next one's Keep, else the panel. */
  const nextProposal = (index: number) => () =>
    panel.current?.querySelectorAll<HTMLElement>("[data-proposal] [data-action=keep]")[index] ?? panel.current;
  return shell(
    <>
      <header {...sheet.header}>
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
      {/* Waiting only as Needs you says it is: an unapproved thread with nothing open asks nothing of the user. No
          total beside it: the counts below are of the tasks drawn, which a total of every member would not add up to. */}
      <p className={awaiting ? "thread-status waiting" : "thread-status"} data-testid="thread-status">
        {detail.approved ? <Icon of={Check} size={16} /> : awaiting ? <Icon of={CircleAlert} size={16} /> : null}
        {detail.approved ? "Approved" : awaiting ? "Waiting for your approval" : "Not approved, with nothing open"}
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
          {/* Still offered with nothing open, as approval covers what joins the thread later, but not as the decision:
              its items, all finished, are no longer what it approves. */}
          <button
            type="button"
            className={awaiting ? "primary" : undefined}
            data-action="approve"
            {...held(
              () => onCommand(approveCommand(id), awaiting ? `Approved ${name}: ${items}` : `Approved ${name}`),
              toPanel,
            )}
          >
            {awaiting ? `Approve ${items}` : "Approve"}
          </button>
          {awaiting ? null : <p className="muted aside">Covers what's added to it later.</p>}
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
            {open.map((proposal, index) => (
              <li key={proposal.id} data-proposal={proposal.id}>
                <span className="grow">{describe(proposal, titleOf)}</span>
                <button
                  type="button"
                  data-action="keep"
                  {...held(() => onCommand(keepCommand(proposal.id), `Kept ${change(proposal)}`), nextProposal(index))}
                >
                  Keep
                </button>
                <button
                  type="button"
                  className="quiet"
                  data-action="drop"
                  {...held(
                    () => onCommand(dropCommand(proposal.id), `Dropped ${change(proposal)}`),
                    nextProposal(index),
                  )}
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
            <Prose text={detail.brief} notes />
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
          hint={
            detail.level === null
              ? (() => {
                  const said = levelHint(globalLevel);
                  return said === null ? null : `${said}, as set for every thread`;
                })()
              : levelHint(detail.level)
          }
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
            {...held(() => onCommand(revokeCommand(id), `Revoked ${name}'s approval`), toPanel)}
          >
            Revoke approval
          </button>
        </div>
      ) : null}
    </>,
  );
}

/** A proposal as settling it says, as the needs-you list does: whose change of what. */
const change = (proposal: Proposal) => `${proposal.actor}'s ${proposal.edit.field} change`;

/** A proposal in a line: who proposed what, for which item. */
function describe(proposal: Proposal, titleOf: (id: string) => string): string {
  const { edit } = proposal;
  const item = edit.item === undefined ? "" : `${titleOf(edit.item)}: `;
  const shown = (value: unknown) => (value === null || value === undefined ? "none" : String(value));
  return `${item}${edit.field} ${shown(edit.before)} → ${shown(edit.after)} (${proposal.actor})`;
}
