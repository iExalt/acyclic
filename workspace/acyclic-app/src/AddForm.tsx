// Adding an item: a title, its thread, and, when no parent is given, the project it goes in or the top level. Its id is
// made from the title, and asked for only when the server refused it.

import { type FormEvent, useEffect, useRef, useState } from "react";
import type { Thread } from "./api";
import { type NewItem, newId } from "./edits";
import { useFocusIn } from "./focus";

export interface AddFormProps {
  parent: string | null;
  parentTitle: string | null;
  /** The thread it starts in. */
  thread: string | null;
  /** With no parent, the projects to put it in, and the one it starts in. */
  projects: readonly { id: string; title: string }[];
  project: string | null;
  threads: Thread[];
  taken: ReadonlySet<string>;
  /** Set when the server refused the id, to have the user change it. */
  idRefused: boolean;
  /** What the user wrote before, when a failed add opens the form again. */
  draft: NewItem | null;
  busy: boolean;
  onSubmit: (item: NewItem) => void;
  onCancel: () => void;
}

export function AddForm({
  parent,
  parentTitle,
  thread: initialThread,
  projects,
  project: initialProject,
  threads,
  taken,
  idRefused,
  draft,
  busy,
  onSubmit,
  onCancel,
}: AddFormProps) {
  const [title, setTitle] = useState(draft?.title ?? "");
  const [id, setId] = useState<string | null>(draft?.item ?? null);
  const [thread, setThread] = useState(draft === null ? initialThread : draft.thread);
  const [project, setProject] = useState(draft === null ? initialProject : draft.parent);
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
    onSubmit({ item: shownId, title: title.trim(), parent: parent ?? project, thread });
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
      <div className="form-head">
        <h2>New item</h2>
        {parent === null ? null : <p className="muted">Inside {parentTitle}</p>}
      </div>
      <label>
        Title
        {/* biome-ignore lint/a11y/noAutofocus: the form opens to take a title */}
        <input name="title" value={title} autoFocus onChange={(event) => setTitle(event.target.value)} />
      </label>
      {parent !== null || projects.length === 0 ? null : (
        <label>
          Project
          <select
            name="project"
            value={project ?? ""}
            onChange={(event) => setProject(event.target.value === "" ? null : event.target.value)}
          >
            <option value="">None, at the top level</option>
            {projects.map((one) => (
              <option key={one.id} value={one.id}>
                {one.title}
              </option>
            ))}
          </select>
        </label>
      )}
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
      {/* Made from the title, which is all most adds need; asked for once the server refuses it. */}
      {idRefused ? (
        <label>
          Id
          <span className="hint">Not accepted: see why below, and choose another.</span>
          <input
            name="id"
            ref={idField}
            value={shownId}
            aria-invalid={idRefused}
            onChange={(event) => setId(event.target.value)}
          />
        </label>
      ) : null}
      <div className="actions">
        <button type="submit" className="primary" disabled={busy || title.trim() === ""}>
          Add item
        </button>
        <button type="button" className="quiet" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
