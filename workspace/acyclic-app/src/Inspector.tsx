// The selected item: its fields, its conflicts with a way to settle each, and its links both ways, each a button that
// selects the item it names. Notes are agents' text, so they are shown as text and never as markup.

import { useState } from "react";
import type { Command, Item, Thread } from "./api";
import type { LinkKind } from "./edits";
import { reopenCommand, resolveCommand } from "./forms";
import { ItemEditor } from "./ItemEditor";

export interface InspectorProps {
  item: Item;
  titleOf: (id: string) => string;
  onSelect: (id: string) => void;
  onUnlink: (from: string, kind: LinkKind, to: string) => void;
  onMove: () => void;
  onLink: (kind: LinkKind) => void;
  onAdd: () => void;
  onClose: () => void;
  /** Sends an edit of this item, such as its fields or a conflict settled. */
  onCommand: (command: Command) => void;
  threads: Thread[];
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
  onCommand,
  threads,
  busy,
}: InspectorProps) {
  const [editing, setEditing] = useState(false);
  const chip = (id: string) => (
    <button type="button" className="chip" data-select={id} onClick={() => onSelect(id)}>
      {titleOf(id)}
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
                onClick={() => unlink(id)}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      </div>
    );
  return (
    <aside className="inspector" data-testid="inspector" aria-label="Selected item">
      <header>
        <h2>{item.title}</h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          ×
        </button>
      </header>
      <p className="facts">
        <code>{item.id}</code> · {item.state}
        {item.thread === null ? null : <> · {item.thread}</>}
      </p>
      <p className="facts">In {item.parent === null ? "the top level" : chip(item.parent)}</p>
      <Conflicts item={item} titleOf={titleOf} busy={busy} onCommand={onCommand} />
      {editing ? (
        <ItemEditor
          item={item}
          threads={threads}
          busy={busy}
          onSave={(edit) => {
            setEditing(false);
            onCommand(edit);
          }}
          onCancel={() => setEditing(false)}
        />
      ) : null}
      <div className="actions">
        <button type="button" onClick={() => setEditing(true)} data-action="edit" disabled={editing}>
          Edit
        </button>
        {item.state === "done" ? (
          <button type="button" onClick={() => onCommand(reopenCommand(item.id))} data-action="reopen" disabled={busy}>
            Reopen
          </button>
        ) : null}
        <button type="button" onClick={onAdd} data-action="add-child">
          Add inside
        </button>
        <button type="button" onClick={onMove} data-action="move">
          Move
        </button>
        <button type="button" onClick={() => onLink("waits-for")} data-action="waits-for">
          Waits for…
        </button>
        <button type="button" onClick={() => onLink("relates-to")} data-action="relates-to">
          Relates to…
        </button>
      </div>
      {links("Waits for", item.links.waits_for, (id) => onUnlink(item.id, "waits-for", id), "waits-for")}
      {links("Waited on by", item.links.waited_on_by, (id) => onUnlink(id, "waits-for", item.id), "waited-on-by")}
      {links("Relates to", item.links.relates_to, (id) => onUnlink(item.id, "relates-to", id), "relates-to")}
      {links("Related from", item.links.related_from, (id) => onUnlink(id, "relates-to", item.id), "related-from")}
      {item.notes === null ? null : (
        <p className="notes" data-testid="notes">
          {item.notes}
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
                  <button
                    type="button"
                    data-keep={value ?? ""}
                    disabled={busy || settle === null}
                    onClick={() => settle !== null && onCommand(settle)}
                  >
                    Use this
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </div>
  );
}
