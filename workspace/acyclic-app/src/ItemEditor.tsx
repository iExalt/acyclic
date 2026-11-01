// The selected item's fields as a form. Save sends only what changed; sets of areas, resources and repositories stay
// with the CLI.

import { type FormEvent, useState } from "react";
import type { Command, Item } from "./api";
import type { Edited } from "./edits";
import { draftOf, type ItemDraft, itemEdit, SIZES, STATES, stateHint } from "./forms";
import { StageLabel } from "./Stage";
import { stateWords } from "./stages";

export interface ItemEditorProps {
  item: Item;
  /** The threads to pick from, each as text with its number. */
  threads: { id: string; label: string }[];
  /** What the user wrote before, and over what, when a failed save opens the editor again. */
  initial: Edited | null;
  busy: boolean;
  onSave: (edit: Command, edited: Edited) => void;
  onCancel: () => void;
}

export function ItemEditor({ item, threads, initial, busy, onSave, onCancel }: ItemEditorProps) {
  // The fields as it opened on them, so a save sends only what the user changed, not what changed elsewhere meanwhile.
  const [base] = useState<ItemDraft>(() => initial?.base ?? draftOf(item));
  const [draft, setDraft] = useState<ItemDraft>(() => initial?.draft ?? base);
  const set = (field: keyof ItemDraft) => (event: { target: { value: string } }) =>
    setDraft((was) => ({ ...was, [field]: event.target.value }));
  const edit = itemEdit(item.id, base, draft);
  const hint = stateHint(draft, item.stage !== undefined);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (edit === null) onCancel();
    else onSave(edit, { base, draft });
  };
  return (
    <form
      className="fields"
      data-testid="item-form"
      // What the user changed, which no key throws away; unchanged, Escape closes the editor alone.
      data-dirty={edit !== null}
      onSubmit={submit}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || edit !== null) return;
        event.stopPropagation();
        onCancel();
      }}
    >
      <label>
        Title
        <input name="title" value={draft.title} onChange={set("title")} />
      </label>
      {item.state === "done" ? (
        // Finished work's state is set by finishing it, so it reads as the panel says it, not as a field.
        <div className="fixed-field">
          State
          <StageLabel stage={item.stage ?? "done"} />
        </div>
      ) : (
        <label>
          State
          <select name="state" value={draft.state} onChange={set("state")}>
            {STATES.map((state) => (
              <option key={state} value={state}>
                {stateWords(state)}
              </option>
            ))}
          </select>
          {hint === null ? null : <span className="hint">{hint}</span>}
        </label>
      )}
      <label>
        Thread
        <select name="thread" value={draft.thread} onChange={set("thread")}>
          <option value="">None</option>
          {threads.map((one) => (
            <option key={one.id} value={one.id}>
              {one.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        Gate
        <input name="gate" value={draft.gate} onChange={set("gate")} aria-describedby="gate-hint" />
        <span className="hint" id="gate-hint">
          What must pass for it to be done
        </span>
      </label>
      <label>
        Size
        <select name="size" value={draft.size} onChange={set("size")}>
          <option value="">None</option>
          {SIZES.map((size) => (
            <option key={size} value={size}>
              {size}
            </option>
          ))}
        </select>
      </label>
      <label>
        Notes
        <textarea name="notes" rows={4} value={draft.notes} onChange={set("notes")} />
      </label>
      <div className="actions">
        <button type="submit" className="primary" disabled={busy || edit === null}>
          Save
        </button>
        <button type="button" className="quiet" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
