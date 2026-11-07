// The selected item: its fields, its conflicts with a way to settle each, and its links both ways, each a row that
// selects the item it names. Quiet, as Linear's: Edit in view, the rest of what can be done in the ⋯ menu beside the
// title and on their keys. Notes are agents' text, so only a small, safe part of Markdown is read in them, built as
// elements and never as markup.

import { useEffect, useRef, useState } from "react";
import type { Command, Item, Stage, Thread } from "./api";
import { Badge, lineStyle } from "./Badge";
import type { Edited } from "./edits";
import { useFocusIn } from "./focus";
import { reopenCommand } from "./forms";
import { useHeld } from "./held";
import { ItemEditor } from "./ItemEditor";
import { ArrowUpLeft, FolderInput, Icon, LinkIcon, Pencil, Plus, RotateCcw, X } from "./icons";
import type { Line } from "./lines";
import { Menu } from "./Menu";
import { conflictOf, keepsIn, type Need, structuralKey } from "./needs";
import { Prose } from "./Prose";
import { useSheet } from "./Sheet";
import { StageCounts, StageIcon, StageLabel } from "./Stage";
import { stateWords } from "./stages";

export interface InspectorProps {
  item: Item;
  isProject: boolean;
  titleOf: (id: string) => string;
  /** A linked item's stage, to show beside it. */
  stageOf: (id: string) => Stage | undefined;
  onSelect: (id: string) => void;
  /** The keyboard's ask to edit: each new one opens the editor, as Edit does. */
  edit?: object | undefined;
  /** Whether single-key shortcuts are on, so the buttons say their keys. */
  shortcuts: boolean;
  /** Picks the item's thread, which closes the item to show the thread's panel. */
  onPickThread: (id: string) => void;
  onUnlink: (from: string, to: string) => void;
  onMove: () => void;
  onLink: () => void;
  onAdd: () => void;
  onClose: () => void;
  /** Closes it as Escape does, for a drag down on a phone: a form holding typing stays, saying how to leave it. */
  onDismiss: () => void;
  /** Copies the address that opens this again, saying so. */
  onCopyLink: () => void;
  /** Its editor closed, saved or not. */
  onEditorClosed: () => void;
  /** Sends an edit of this item, such as a conflict settled. */
  onCommand: (command: Command) => void;
  /** Saves its fields, with the draft to draw at once. */
  onSave: (command: Command, edited: Edited) => void;
  /** The editor's draft, asked open again after a save failed: each new ask opens the editor with it. */
  reopen?: { edited: Edited } | undefined;
  threads: Thread[];
  /** The initiatives' titles by id: an epic's parent is one, which no item selects. */
  initiativeTitles: ReadonlyMap<string, string>;
  /** Each thread's line, to show the item's. */
  lines: ReadonlyMap<string, Line>;
  /** The needs-you list, whose conflict entries say how each of the item's conflicted values is kept. */
  needs: readonly Need[];
  busy: boolean;
}

export function Inspector({
  item,
  isProject,
  titleOf,
  stageOf,
  onSelect,
  edit,
  shortcuts,
  onPickThread,
  onUnlink,
  onMove,
  onLink,
  onAdd,
  onClose,
  onDismiss,
  onCopyLink,
  onEditorClosed,
  onCommand,
  onSave,
  reopen,
  threads,
  initiativeTitles,
  lines,
  needs,
  busy,
}: InspectorProps) {
  const [editing, setEditing] = useState(false);
  // An ask from the keyboard, made after the panel opened; one made before belongs to an earlier panel.
  const asked = useRef(edit);
  useEffect(() => {
    if (edit === asked.current) return;
    asked.current = edit;
    if (edit !== undefined && !busy) setEditing(true);
  }, [edit, busy]);
  /** What the user wrote, open again once their save failed. */
  const [reopened, setReopened] = useState<Edited | null>(null);
  const reopenSeen = useRef(reopen);
  useEffect(() => {
    if (reopen === reopenSeen.current) return;
    reopenSeen.current = reopen;
    if (reopen === undefined) return;
    setReopened(reopen.edited);
    setEditing(true);
  }, [reopen]);
  /** A button's key, for assistive technology and a tooltip, while shortcuts are on. */
  const key = (label: string, letter: string) =>
    shortcuts ? { title: `${label} (${letter})`, "aria-keyshortcuts": letter } : {};
  /** An action in the menu, with its key shown at the row's end while shortcuts are on. */
  const action = (
    name: string,
    label: string,
    icon: Parameters<typeof Icon>[0]["of"],
    run: () => void,
    letter = "",
  ) => (
    <button
      type="button"
      className="menu-action"
      onClick={run}
      data-action={name}
      disabled={busy}
      {...(letter === "" ? {} : key(label, letter))}
    >
      <Icon of={icon} size={16} />
      <span className="grow">{label}</span>
      {shortcuts && letter !== "" ? (
        <span className="kbd" aria-hidden="true">
          {letter}
        </span>
      ) : null}
    </button>
  );
  const panel = useRef<HTMLElement>(null);
  const sheet = useSheet(panel, onDismiss);
  // A removed link's row goes with its button: focus the first remaining row in its group, else the panel.
  const held = useHeld(busy, [...item.links.waits_for, ...item.links.waited_on_by].join("\n"));
  useFocusIn(panel);
  const chip = (id: string) => {
    const stage = stageOf(id);
    return (
      <button type="button" className="chip" data-select={id} onClick={() => onSelect(id)}>
        {stage === undefined ? null : <StageIcon stage={stage} />}
        <Prose text={titleOf(id)} />
      </button>
    );
  };
  /** A list of links, a row each, cut to a line, its remove button shown on hover or focus where there is hover. */
  const links = (label: string, ids: string[], unlink: (id: string) => void, testid: string) =>
    ids.length === 0 ? null : (
      <div className="links" data-testid={testid}>
        <h3>
          {label} <span className="count">{ids.length}</span>
        </h3>
        <ul>
          {ids.map((id) => {
            const stage = stageOf(id);
            return (
              <li key={id} className="link">
                <button
                  type="button"
                  className="link-row"
                  data-select={id}
                  title={titleOf(id)}
                  onClick={() => onSelect(id)}
                >
                  {stage === undefined ? null : <StageIcon stage={stage} />}
                  <span className="link-title">
                    <Prose text={titleOf(id)} />
                  </span>
                </button>
                <button
                  type="button"
                  className="unlink"
                  aria-label={`Remove the link to ${titleOf(id)}`}
                  {...held(
                    `${testid}:${id}`,
                    () => unlink(id),
                    () =>
                      panel.current?.querySelector<HTMLElement>(`[data-testid="${testid}"] .link-row`) ?? panel.current,
                  )}
                >
                  <Icon of={X} size={16} />
                </button>
              </li>
            );
          })}
        </ul>
      </div>
    );
  return (
    <aside
      ref={panel}
      tabIndex={-1}
      className={editing ? "inspector tall" : "inspector"}
      data-testid="inspector"
      aria-label="Selected item"
      data-detent={sheet.detent}
    >
      {/* A form has no height to change, so its editor neither shows the handle (app.css) nor follows a drag. */}
      <header {...(editing ? {} : sheet.header)}>
        {sheet.handle}
        {/* First, to float at the title's end: the title wraps under them, at the header's whole width. */}
        <div className="header-actions">
          <button
            type="button"
            className="close"
            aria-label="Edit"
            title="Edit"
            onClick={() => {
              if (!busy) setEditing(true);
            }}
            data-action="edit"
            {...key("Edit", "E")}
            disabled={editing}
            aria-disabled={busy || undefined}
          >
            <Icon of={Pencil} />
          </button>
          <Menu label="Item actions" testid="item-menu" list>
            {action("move", "Move…", FolderInput, onMove, "M")}
            {item.state === "done"
              ? action("reopen", "Reopen", RotateCcw, () => onCommand(reopenCommand(item.id)))
              : null}
            {action("add-child", "Add inside", Plus, onAdd, "A")}
            {action("waits-for", "Waits for…", ArrowUpLeft, onLink, "W")}
            {/* In the menu rather than beside it, so the title keeps the header's width. */}
            {action("copy-link", "Copy link", LinkIcon, onCopyLink)}
          </Menu>
          <button type="button" className="close" aria-label="Close" onClick={onClose}>
            <Icon of={X} />
          </button>
        </div>
        <h2>
          <Prose text={item.title} />
        </h2>
      </header>
      <dl className="facts">
        {/* While editing, the form's State and Thread say these, so the rows would only repeat them. */}
        {editing ? null : (
          <>
            <dt>Status</dt>
            {/* An item's main thing to see at the sheet's first height: its status. */}
            <dd data-testid="state" className="status-value" data-main>
              <ItemStatus item={item} />
            </dd>
            {isProject && item.thread === null ? null : (
              <>
                <dt>Thread</dt>
                <dd>
                  {/* A thread of one is the task alone, which names no thread. */}
                  {item.thread === null || lines.get(item.thread)?.task ? (
                    <span className="muted">None</span>
                  ) : (
                    <button
                      type="button"
                      className="chip line"
                      style={lineStyle(lines.get(item.thread))}
                      data-pick-thread={item.thread}
                      onClick={() => item.thread !== null && onPickThread(item.thread)}
                    >
                      <Badge line={lines.get(item.thread)} />
                      <Prose text={lines.get(item.thread)?.name ?? item.thread} />
                    </button>
                  )}
                </dd>
              </>
            )}
          </>
        )}
        <dt>Parent</dt>
        <dd>
          {item.parent === null ? (
            <span className="muted">No initiative</span>
          ) : initiativeTitles.has(item.parent) ? (
            <Prose text={initiativeTitles.get(item.parent) ?? item.parent} />
          ) : (
            chip(item.parent)
          )}
        </dd>
        <dt>Id</dt>
        <dd>
          <code className="id">{item.id}</code>
        </dd>
      </dl>
      <Conflicts item={item} needs={needs} titleOf={titleOf} busy={busy} onCommand={onCommand} />
      {editing ? (
        <ItemEditor
          item={item}
          threads={threads.map((one) => ({ id: one.id, label: one.title }))}
          initial={reopened}
          busy={busy}
          onSave={(command, edited) => {
            setEditing(false);
            // Sent with what it drew: a failure asks the editor open again with it.
            setReopened(null);
            onEditorClosed();
            onSave(command, edited);
          }}
          onCancel={() => {
            setEditing(false);
            onEditorClosed();
          }}
        />
      ) : null}
      {links("Waits for", item.links.waits_for, (id) => onUnlink(item.id, id), "waits-for")}
      {links("Waited on by", item.links.waited_on_by, (id) => onUnlink(id, item.id), "waited-on-by")}
      {/* While editing, the form's Notes field holds them. */}
      {item.notes === null || editing ? null : (
        <div className="notes-block">
          <h3>Notes</h3>
          <p className="notes" data-testid="notes">
            <Prose text={item.notes} notes />
          </p>
        </div>
      )}
    </aside>
  );
}

/** Each conflicted field's values, written apart by different replicas, and a button to keep each one. */
function Conflicts({
  item,
  needs,
  titleOf,
  busy,
  onCommand,
}: {
  item: Item;
  needs: readonly Need[];
  titleOf: (id: string) => string;
  busy: boolean;
  onCommand: (command: Command) => void;
}) {
  const fields = Object.entries(item.conflicts);
  if (fields.length === 0) return null;
  const shown = (field: string, value: string | null) =>
    value === null ? "(none)" : field === "parent" ? titleOf(value) : value;
  return (
    <div className="conflicts" data-testid="conflicts">
      <h3>Written differently on two machines</h3>
      {fields.map(([field, values]) => (
        <div key={field} className="conflict" data-conflict={field}>
          <span className="field">{field}</span>
          <ul>
            {(values ?? []).map((value) => {
              const key = structuralKey(value);
              const kept = keepsIn(conflictOf(needs, "item", item.id, field)).find(
                (one) => structuralKey(one.value) === key,
              );
              const settle = kept?.command ?? null;
              return (
                <li key={key}>
                  <span className="value">{shown(field, value)}</span>
                  {settle === null ? (
                    <span className="why">
                      {kept?.why ?? "Settle this one in a terminal: run acyclic needs to see how"}
                    </span>
                  ) : (
                    <button type="button" data-keep={value ?? ""} disabled={busy} onClick={() => onCommand(settle)}>
                      Use this
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </div>
  );
}

/** Where an item stands, with a parent's leaves counted by stage. */
function ItemStatus({ item }: { item: Item }) {
  // A parent with no leaf to count, as on a cycle of parents, has no stage: it says its own state, as a leaf would.
  if (item.progress !== undefined)
    return (
      <>
        <span>{item.stage === undefined ? stateWords(item.state) : <StageLabel stage={item.stage} />}</span>
        {item.progress.counts.leaves + item.progress.counts.canceled === 0 ? null : (
          <StageCounts counts={item.progress.counts} />
        )}
      </>
    );
  // A leaf core hasn't placed is one drawn before the server has it: its stage comes with the server's read, and its
  // state, open, would say something else first.
  if (item.stage === undefined) return <span className="muted">Saving…</span>;
  return <span>{item.stage === undefined ? stateWords(item.state) : <StageLabel stage={item.stage} />}</span>;
}
