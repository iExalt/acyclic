// The selected item: its fields, its conflicts with a way to settle each, and its links both ways, each a button that
// selects the item it names. Notes are agents' text, so they are shown as text and never as markup.

import { type ReactNode, useRef, useState } from "react";
import type { Command, Item, Thread } from "./api";
import { Badge, lineStyle } from "./Badge";
import type { Edited, LinkKind } from "./edits";
import { useFocusIn } from "./focus";
import { reopenCommand, resolveCommand, unsettledWhy } from "./forms";
import { ItemEditor } from "./ItemEditor";
import { Icon, X } from "./icons";
import type { Line } from "./lines";
import { Prose } from "./Prose";
import { StageCounts, StageLabel } from "./Stage";
import { stateWords } from "./stages";

export interface InspectorProps {
  item: Item;
  titleOf: (id: string) => string;
  onSelect: (id: string) => void;
  onUnlink: (from: string, kind: LinkKind, to: string) => void;
  onMove: () => void;
  onLink: (kind: LinkKind) => void;
  onAdd: () => void;
  onClose: () => void;
  /** Its editor closed, saved or not. */
  onEditorClosed: () => void;
  /** Sends an edit of this item, such as a conflict settled. */
  onCommand: (command: Command) => void;
  /** Saves its fields, with the draft to draw at once. */
  onSave: (command: Command, edited: Edited) => void;
  /** The editor's draft, open again after a save failed, or null. */
  reopened: Edited | null;
  threads: Thread[];
  /** A refusal or failure to show, under its title rather than over it. */
  notice: ReactNode;
  /** Each thread's line, to show the item's. */
  lines: ReadonlyMap<string, Line>;
  busy: boolean;
}

export function Inspector({
  item,
  titleOf,
  onSelect,
  onUnlink,
  onMove,
  onLink,
  onAdd,
  onClose,
  onEditorClosed,
  onCommand,
  onSave,
  reopened,
  threads,
  notice,
  lines,
  busy,
}: InspectorProps) {
  const [editing, setEditing] = useState(reopened !== null);
  const panel = useRef<HTMLElement>(null);
  useFocusIn(panel);
  const chip = (id: string) => (
    <button type="button" className="chip" data-select={id} onClick={() => onSelect(id)}>
      <Prose text={titleOf(id)} />
    </button>
  );
  /** A list of links, each removable from the item that holds it. */
  const links = (label: string, ids: string[], unlink: (id: string) => void, testid: string) =>
    ids.length === 0 ? null : (
      <div className="links" data-testid={testid}>
        <h3>{label}</h3>
        <ul>
          {ids.map((id) => (
            <li key={id}>
              {chip(id)}
              <button
                type="button"
                className="unlink"
                aria-label={`Remove the link to ${id}`}
                disabled={busy}
                onClick={() => unlink(id)}
              >
                <Icon of={X} />
              </button>
            </li>
          ))}
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
    >
      <header>
        <h2>
          <Prose text={item.title} />
        </h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          <Icon of={X} />
        </button>
      </header>
      {notice}
      <dl className="facts">
        <dt>Id</dt>
        <dd>{item.id}</dd>
        <dt>Status</dt>
        <dd data-testid="state" className="status-value">
          <ItemStatus item={item} />
        </dd>
        {item.thread === null ? null : (
          <>
            <dt>Thread</dt>
            <dd className="line" style={lineStyle(lines.get(item.thread))}>
              <Badge line={lines.get(item.thread)} />
              {lines.get(item.thread)?.name ?? item.thread}
            </dd>
          </>
        )}
        <dt>In</dt>
        <dd>{item.parent === null ? "the top level" : chip(item.parent)}</dd>
      </dl>
      <Conflicts item={item} titleOf={titleOf} busy={busy} onCommand={onCommand} />
      {editing ? (
        <ItemEditor
          item={item}
          threads={threads}
          initial={reopened}
          busy={busy}
          onSave={(edit, edited) => {
            setEditing(false);
            onEditorClosed();
            onSave(edit, edited);
          }}
          onCancel={() => {
            setEditing(false);
            onEditorClosed();
          }}
        />
      ) : null}
      <div className="actions">
        <button type="button" onClick={() => setEditing(true)} data-action="edit" disabled={editing || busy}>
          Edit
        </button>
        {item.state === "done" ? (
          <button type="button" onClick={() => onCommand(reopenCommand(item.id))} data-action="reopen" disabled={busy}>
            Reopen
          </button>
        ) : null}
        <button type="button" onClick={onAdd} data-action="add-child" disabled={busy}>
          Add inside
        </button>
        <button type="button" onClick={onMove} data-action="move" disabled={busy}>
          Move
        </button>
        <button type="button" onClick={() => onLink("waits-for")} data-action="waits-for" disabled={busy}>
          Waits for…
        </button>
        <button type="button" onClick={() => onLink("relates-to")} data-action="relates-to" disabled={busy}>
          Relates to…
        </button>
      </div>
      {links("Waits for", item.links.waits_for, (id) => onUnlink(item.id, "waits-for", id), "waits-for")}
      {links("Waited on by", item.links.waited_on_by, (id) => onUnlink(id, "waits-for", item.id), "waited-on-by")}
      {links("Relates to", item.links.relates_to, (id) => onUnlink(item.id, "relates-to", id), "relates-to")}
      {links("Related from", item.links.related_from, (id) => onUnlink(id, "relates-to", item.id), "related-from")}
      {item.notes === null ? null : (
        <p className="notes" data-testid="notes">
          <Prose text={item.notes} />
        </p>
      )}
    </aside>
  );
}

/** Each conflicted field's values, written apart by different replicas, and a button to keep each one. */
function Conflicts({
  item,
  titleOf,
  busy,
  onCommand,
}: {
  item: Item;
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
              const settle = resolveCommand("item", item.id, field, value);
              return (
                <li key={JSON.stringify(value)}>
                  <span className="value">{shown(field, value)}</span>
                  {settle === null ? (
                    <span className="why">{unsettledWhy("item", field, value)}</span>
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

/** Where an item stands: a leaf's stage, or a parent's state from the work under it with its leaves by stage. */
function ItemStatus({ item }: { item: Item }) {
  const archived = item.archived ? <span className="muted">, archived</span> : null;
  if (item.progress !== undefined)
    return (
      <>
        <span>
          {stateWords(item.progress.state)}
          {item.progress.counts.leaves === 0 ? " (all archived)" : null}
          {archived}
        </span>
        {item.progress.counts.leaves === 0 ? null : <StageCounts counts={item.progress.counts} />}
      </>
    );
  return (
    <span>
      {item.stage === undefined ? stateWords(item.state) : <StageLabel stage={item.stage} />}
      {archived}
    </span>
  );
}
