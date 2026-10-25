// Adding an item: a title, the id it gets (made from the title until the user changes it), and its thread. It goes
// under the parent given, or at the top level.

import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import type { Thread } from "./api";
import { type NewItem, newId } from "./edits";
import { useFocusIn } from "./focus";

export interface AddFormProps {
  parent: string | null;
  parentTitle: string | null;
  /** The thread it starts in. */
  thread: string | null;
  threads: Thread[];
  taken: ReadonlySet<string>;
  /** Set when the server refused the id, to have the user change it. */
  idRefused: boolean;
  /** What the user wrote before, when a failed add opens the form again. */
  draft: NewItem | null;
  busy: boolean;
  /** A refusal or failure to show, under its heading rather than over it. */
  notice: ReactNode;
  onSubmit: (item: NewItem) => void;
  onCancel: () => void;
}

export function AddForm({
  parent,
  parentTitle,
  thread: initialThread,
  threads,
  taken,
  idRefused,
  draft,
  busy,
  notice,
  onSubmit,
  onCancel,
}: AddFormProps) {
  const [title, setTitle] = useState(draft?.title ?? "");
  const [id, setId] = useState<string | null>(draft?.item ?? null);
  const [thread, setThread] = useState(draft === null ? initialThread : draft.thread);
  const idField = useRef<HTMLInputElement>(null);
  const form = useRef<HTMLFormElement>(null);
  useFocusIn(form);
  const shownId = id ?? newId(title, taken);
  useEffect(() => {
    if (idRefused) idField.current?.focus();
  }, [idRefused]);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (title.trim() === "") return;
    onSubmit({ item: shownId, title: title.trim(), parent, thread });
  };
  return (
    <form
      ref={form}
      className="sheet add"
      data-testid="add-form"
      // What the user typed, which no key throws away.
      data-dirty={title !== "" || id !== null}
      onSubmit={submit}
    >
      <h2>Add {parent === null ? "at the top level" : <>inside {parentTitle}</>}</h2>
      {notice}
      <label>
        Title
        {/* biome-ignore lint/a11y/noAutofocus: the form opens to take a title */}
        <input name="title" value={title} autoFocus onChange={(event) => setTitle(event.target.value)} />
      </label>
      <label>
        Id
        <input
          name="id"
          ref={idField}
          value={shownId}
          aria-invalid={idRefused}
          onChange={(event) => setId(event.target.value)}
        />
      </label>
      <label>
        Thread
        <select
          name="thread"
          value={thread ?? ""}
          onChange={(event) => setThread(event.target.value === "" ? null : event.target.value)}
        >
          <option value="">None</option>
          {threads.map((one) => (
            <option key={one.id} value={one.id}>
              {one.title}
            </option>
          ))}
        </select>
      </label>
      <div className="actions">
        <button type="submit" className="primary" disabled={busy || title.trim() === ""}>
          Add
        </button>
        <button type="button" className="quiet" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
