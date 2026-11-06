// Adding an item: a title, its thread, and, when no parent is given, the project it goes in, or else the initiative in
// context, where it becomes a project of its own. Its id is
// made as the form opens, so a send that is repeated is the same add.

import { type FormEvent, useRef, useState } from "react";
import type { NewItem } from "./edits";
import { useFieldInView, useFocusIn } from "./focus";
import { Icon, X } from "./icons";
import { groupRows, groupTitle, newParent } from "./initiatives";
import { uniqueProjectThread } from "./outline";
import { uuidv7 } from "./uuid";

export interface AddFormProps {
  parent: string | null;
  parentTitle: string | null;
  /** The thread it starts in. */
  thread: string | null;
  /** With no parent, the projects to put it in, and the one it starts in. */
  projects: readonly { id: string; title: string; initiative: string | null }[];
  project: string | null;
  /** With no parent and no project, the initiative in context it goes under, as a project of its own. */
  initiative: { id: string; title: string } | null;
  /** Each initiative's title, which groups the projects. */
  initiativeTitles: ReadonlyMap<string, string>;
  /** Visible threads in each project, used only while the thread choice remains automatic. */
  projectThreads: readonly { id: string; threads: readonly { id: string }[] }[];
  /** Each thread, named as everywhere a badge doesn't stand beside it. */
  threads: readonly { id: string; label: string }[];
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
  initiative,
  initiativeTitles,
  projectThreads,
  threads,
  draft,
  busy,
  onSubmit,
  onCancel,
}: AddFormProps) {
  const [title, setTitle] = useState(draft?.title ?? "");
  // A reopened draft keeps its id: the add that failed may have been saved, and sending it again is the same add.
  const [id] = useState(() => draft?.id ?? uuidv7());
  const [thread, setThread] = useState(
    draft === null ? (initialThread ?? uniqueProjectThread(initialProject, projectThreads)) : draft.thread,
  );
  const [project, setProject] = useState(draft === null ? initialProject : draft.parent);
  const [threadChosen, setThreadChosen] = useState(draft !== null);
  const inferredThread = (projectId: string | null) => initialThread ?? uniqueProjectThread(projectId, projectThreads);
  const form = useRef<HTMLFormElement>(null);
  useFocusIn(form);
  useFieldInView(form);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (title.trim() === "") return;
    onSubmit({ id, title: title.trim(), parent: newParent(parent, project, initiative?.id ?? null), thread });
  };
  return (
    <form
      ref={form}
      className="sheet add"
      data-testid="add-form"
      // What the user typed, which no key throws away.
      data-dirty={title !== ""}
      onSubmit={submit}
    >
      <div className="form-head">
        <h2>New item</h2>
        {/* Where every other panel has it, beside the title, as well as Cancel at the foot. */}
        <button type="button" className="close" aria-label="Close" onClick={onCancel}>
          <Icon of={X} />
        </button>
        {parent === null ? null : <p className="muted">Inside {parentTitle}</p>}
      </div>
      <label>
        Title
        {/* biome-ignore lint/a11y/noAutofocus: the form opens to take a title */}
        <input name="title" value={title} autoFocus onChange={(event) => setTitle(event.target.value)} />
      </label>
      {parent !== null || (projects.length === 0 && initiative === null) ? null : (
        <label>
          Project
          <select
            name="project"
            // A draft's parent that is an initiative, which no project here stands for, is the first option.
            value={projects.some((one) => one.id === project) ? (project ?? "") : ""}
            onChange={(event) => {
              const next = event.target.value === "" ? null : event.target.value;
              setProject(next);
              if (!threadChosen) setThread(inferredThread(next));
            }}
          >
            <option value="">{initiative === null ? "None" : `None, a new project in ${initiative.title}`}</option>
            {projectGroups(projects, initiative?.id ?? null, initiativeTitles).map((group) => (
              <optgroup key={group.initiative ?? ""} label={group.label}>
                {group.projects.map((one) => (
                  <option key={one.id} value={one.id}>
                    {one.title}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </label>
      )}
      <label>
        Thread
        <select
          name="thread"
          value={thread ?? ""}
          onChange={(event) => {
            setThreadChosen(true);
            setThread(event.target.value === "" ? null : event.target.value);
          }}
        >
          <option value="">Automatic</option>
          {threads.map((one) => (
            <option key={one.id} value={one.id}>
              {one.label}
            </option>
          ))}
        </select>
        <span className="hint">
          Uses the parent's, siblings' or ancestor's thread when available; otherwise no thread.
        </span>
      </label>
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

/** The projects by initiative, the one in context first, as the lists group them. */
function projectGroups(
  projects: readonly { id: string; title: string; initiative: string | null }[],
  context: string | null,
  titles: ReadonlyMap<string, string>,
) {
  const groups = groupRows(projects, (one) => [one.initiative], [...titles.keys()], context);
  return groups.map((group) => ({
    initiative: group.initiative,
    label: groupTitle(group.initiative, titles),
    projects: group.rows,
  }));
}
