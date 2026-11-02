// The picked thread: what approving it lets agents work on, its items in the order their work runs, its autonomy level
// and project-management flag, and the proposals agents made to it, each kept or dropped with one tap.

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
import { Check, Icon, Inbox, X } from "./icons";
import type { Line } from "./lines";
import { workOrder } from "./members";
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
  /** What an item waits for, to list the thread's items in the order their work runs. */
  waitsFor: (id: string) => readonly string[];
  /** Changes after every edit sent, so the panel reads the thread again. */
  version: number;
  busy: boolean;
  /** Sends an edit, with what to say once it is saved when that is all that shows it was. */
  onCommand: (command: Command, done?: string, undo?: true) => void;
  /**
   * Saves a setting in the background, after any write out, resolving to whether it was saved: the setting shows its
   * new value at once, and only a refusal puts the old one back.
   */
  onSetting: (command: Command) => Promise<boolean>;
  onSelect: (id: string) => void;
  onClose: () => void;
  /** Closes it as Escape does, for a drag down on a phone: a form holding typing stays, saying how to leave it. */
  onDismiss: () => void;
  /** Copies the address that opens this again. */
  onCopyLink: () => Promise<boolean>;
}

/**
 * The thread as read, with whether Needs you asked for its approval when it was: the list is read again before the
 * thread is, so between the two the list would say a thread just approved waits for nothing, and it isn't approved.
 */
type Loaded = { detail: ThreadDetail; open: Proposal[]; awaiting: boolean } | { failed: string } | null;

export function ThreadPanel({
  id,
  globalLevel,
  awaiting: awaitingNow,
  line,
  counts,
  titleOf,
  stageOf,
  waitsFor,
  version,
  busy,
  onCommand,
  onSetting,
  onSelect,
  onClose,
  onDismiss,
  onCopyLink,
}: ThreadPanelProps) {
  const [loaded, setLoaded] = useState<Loaded>(null);
  /** Settings chosen and not yet read back, by name: shown in place of what the thread was read with. */
  const [showAll, setShowAll] = useState(false);
  const [chosen, setChosen] = useState<Record<string, string>>({});
  /** Shows `value` for setting `name` at once, and the thread's own value again once it is read back or refused. */
  const setting = (name: string, value: string, command: Command) => {
    setChosen((was) => ({ ...was, [name]: value }));
    void onSetting(command).then((saved) => {
      // Refused, it goes back, unless a later pick has taken its place.
      if (!saved)
        setChosen((was) =>
          was[name] === value ? Object.fromEntries(Object.entries(was).filter(([n]) => n !== name)) : was,
        );
    });
  };
  const awaitingRef = useRef(awaitingNow);
  awaitingRef.current = awaitingNow;
  // A new version, after any edit, reads the thread again.
  useEffect(() => {
    let live = true;
    Promise.all([thread(id), proposals()]).then(
      ([detail, all]) => {
        if (!live) return;
        setLoaded({
          detail,
          open: all.filter((one) => one.decision === null && one.threads.includes(id)),
          awaiting: awaitingRef.current,
        });
        // A setting read back as chosen is the thread's own now; one read otherwise waits for its write to land.
        const read: Record<string, string> = { level: detail.level ?? "", pm: detail.pm ? "apply" : "wait" };
        setChosen((was) => Object.fromEntries(Object.entries(was).filter(([name, value]) => read[name] !== value)));
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
      {body}
    </aside>
  );
  if (loaded === null) return shell(null);
  if ("failed" in loaded) return shell(<p role="alert">{loaded.failed}</p>);
  const { detail, open, awaiting } = loaded;
  const members = detail.members.length;
  const ordered = workOrder(detail.members, waitsFor);
  const name = line?.name ?? detail.title;
  const items = `${members} ${members === 1 ? "item" : "items"}`;
  /** After a proposal is settled, the next one's Keep, else the panel. */
  const nextProposal = (index: number) => () =>
    panel.current?.querySelectorAll<HTMLElement>("[data-proposal] [data-action=keep]")[index] ?? panel.current;
  return shell(
    <>
      <header {...sheet.header}>
        {sheet.handle}
        <h2 className="line" style={lineStyle(line)}>
          <Badge line={line} />
          <Prose text={line?.name ?? detail.title} />
        </h2>
        <CopyLink onCopy={onCopyLink} />
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          <Icon of={X} />
        </button>
      </header>
      {/* Waiting only as Needs you says it is: an unapproved thread with nothing open asks nothing of the user. No
          total beside it: the counts below are of the items drawn, which a total of every member would not add up to. */}
      <p className={awaiting ? "thread-status waiting" : "thread-status"} data-testid="thread-status">
        {detail.approved ? <Icon of={Check} size={16} /> : awaiting ? <Icon of={Inbox} size={16} /> : null}
        {detail.approved ? "Approved" : awaiting ? "Waiting for your approval" : "Not approved"}
      </p>
      <div className="progress">
        {counts === undefined || counts.leaves === 0 ? (
          <span className="muted">No items shown</span>
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
              "approve",
              () => onCommand(approveCommand(id), awaiting ? `Approved ${name}: ${items}` : `Approved ${name}`, true),
              toPanel,
            )}
          >
            {awaiting ? `Approve ${items}` : "Approve"}
          </button>
          {awaiting ? null : <p className="muted aside">Covers what's added to it later.</p>}
        </div>
      )}
      {ordered.length === 0 ? null : (
        <div className="thread-items">
          <h3>
            Items <span className="count muted">{members}</span>
          </h3>
          <ul className="rows">
            {(showAll ? ordered : ordered.slice(0, ITEMS_SHOWN)).map((member) => {
              const stage = stageOf(member);
              return (
                <li key={member}>
                  <button type="button" className="row" data-select={member} onClick={() => onSelect(member)}>
                    {stage === undefined ? null : <StageIcon stage={stage} />}
                    <Prose text={titleOf(member)} />
                  </button>
                </li>
              );
            })}
          </ul>
          {showAll || ordered.length <= ITEMS_SHOWN ? null : (
            <button type="button" className="quiet more" onClick={() => setShowAll(true)}>
              {ordered.length - ITEMS_SHOWN} more
            </button>
          )}
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
                  {...held(
                    `keep ${proposal.id}`,
                    () => onCommand(keepCommand(proposal.id), `Kept ${change(proposal)}`),
                    nextProposal(index),
                  )}
                >
                  Keep
                </button>
                <button
                  type="button"
                  className="quiet"
                  data-action="drop"
                  {...held(
                    `drop ${proposal.id}`,
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
      {/* Label beside value, as the item panel's properties are; what a value means is said on hover, as a line under
          each read as clutter. */}
      <div className="settings">
        <h3>Settings</h3>
        <Choice
          label="Autonomy"
          className="setting"
          name="level"
          value={chosen.level ?? detail.level ?? ""}
          title={
            (chosen.level ?? detail.level ?? "") === ""
              ? (() => {
                  const said = levelHint(globalLevel);
                  return said === null ? undefined : `${said}, as set for every thread`;
                })()
              : (levelHint(chosen.level ?? detail.level ?? "") ?? undefined)
          }
          disabled={false}
          onWrite={(level) => setting("level", level, threadLevelCommand(id, level === "" ? null : (level as Level)))}
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
          label="Scope edits"
          className="setting"
          name="pm"
          value={chosen.pm ?? (detail.pm ? "apply" : "wait")}
          title="What happens when an agent changes the scope of an approved thread"
          disabled={false}
          onWrite={(choice) => setting("pm", choice, pmCommand(id, choice === "apply"))}
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
            {...held("revoke", () => onCommand(revokeCommand(id), `Revoked ${name}'s approval`), toPanel)}
          >
            Revoke approval
          </button>
        </div>
      ) : null}
    </>,
  );
}

/** How many of a thread's items show before the rest are asked for. */
const ITEMS_SHOWN = 8;

/** A proposal as settling it says, as the needs-you list does: whose change of what. */
const change = (proposal: Proposal) => `${proposal.actor}'s ${proposal.edit.field} change`;

/** A proposal in a line: who proposed what, for which item. */
function describe(proposal: Proposal, titleOf: (id: string) => string): string {
  const { edit } = proposal;
  const item = edit.item === undefined ? "" : `${titleOf(edit.item)}: `;
  const shown = (value: unknown) => (value === null || value === undefined ? "none" : String(value));
  return `${item}${edit.field} ${shown(edit.before)} → ${shown(edit.after)} (${proposal.actor})`;
}
