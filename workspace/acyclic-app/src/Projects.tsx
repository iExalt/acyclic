// The sidebar's outline: projects, each opening into the threads at work in it, then the threads in none. A selected
// item marks its project and its thread's row there, and opens that project; so does a thread picked elsewhere.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { Badge, lineStyle } from "./Badge";
import { useFocusIn } from "./focus";
import { Icon, X } from "./icons";
import type { Line } from "./lines";
import type { Outline } from "./outline";
import { Prose } from "./Prose";
import { useSheet } from "./Sheet";

export interface ProjectsProps {
  outline: Outline;
  lines: ReadonlyMap<string, Line>;
  /** The thread picked, whose rows are pressed. */
  picked: string | null;
  /** The rows the selected item marks. */
  current: { project: string | null; thread: string | null };
  onPickThread: (id: string) => void;
  onSelectProject: (id: string) => void;
}

export function Projects({ outline, lines, picked, current, onPickThread, onSelectProject }: ProjectsProps) {
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const reveal = (project: string | undefined) =>
    project !== undefined && setOpen((was) => (was.has(project) ? was : new Set([...was, project])));
  useEffect(() => {
    reveal(current.project ?? undefined);
  }, [current.project]);
  // A thread picked with none of its rows in view opens the first project it works in.
  useEffect(() => {
    if (picked === null) return;
    const homes = outline.projects.filter((project) => project.threads.some((one) => one.id === picked));
    if (!homes.some((project) => open.has(project.id))) reveal(homes[0]?.id);
  }, [picked]);
  const title = (id: string) => outline.projects.find((project) => project.id === id)?.title ?? id;

  const row = (thread: string, project: string | null, also: string[] = []) => {
    const line = lines.get(thread);
    return (
      <li key={thread}>
        <button
          type="button"
          className="thread"
          style={lineStyle(line)}
          aria-pressed={thread === picked}
          aria-current={thread === current.thread && project === current.project ? "true" : undefined}
          data-thread={thread}
          data-project={project ?? ""}
          title={also.length === 0 ? line?.name : `${line?.name ?? thread}, also in ${also.map(title).join(", ")}`}
          onClick={() => onPickThread(thread)}
        >
          <Badge line={line} />
          <span className="name">
            <Prose text={line?.name ?? thread} />
            {also.length === 0 ? null : <span className="also">also in {also.map(title).join(", ")}</span>}
          </span>
        </button>
      </li>
    );
  };

  return (
    <div className="outline">
      <h2 className="heading">
        Projects <span className="count">{outline.projects.length}</span>
      </h2>
      <ul>
        {outline.projects.map((project) => {
          const expanded = open.has(project.id);
          return (
            <li key={project.id} className="project" aria-current={project.id === current.project ? "true" : undefined}>
              <div className="project-row">
                <button
                  type="button"
                  className="chevron"
                  aria-expanded={expanded}
                  aria-label={`${expanded ? "Close" : "Open"} ${project.title}`}
                  data-toggle-project={project.id}
                  onClick={() =>
                    setOpen((was) => {
                      const next = new Set(was);
                      if (expanded) next.delete(project.id);
                      else next.add(project.id);
                      return next;
                    })
                  }
                />
                <button
                  type="button"
                  className="project-title"
                  title={project.title}
                  data-select-project={project.id}
                  onClick={() => onSelectProject(project.id)}
                >
                  <Prose text={project.title} />
                </button>
              </div>
              {!expanded ? null : project.threads.length === 0 ? (
                <p className="subheading">No thread has work in this project</p>
              ) : (
                <>
                  <h3 className="subheading">Threads in this project</h3>
                  <ul>{project.threads.map((one) => row(one.id, project.id, one.also))}</ul>
                </>
              )}
            </li>
          );
        })}
      </ul>
      {outline.others.length === 0 ? null : (
        <>
          <h2 className="heading">
            Other threads <span className="count">{outline.others.length}</span>
          </h2>
          <ul>{outline.others.map((thread) => row(thread, null))}</ul>
        </>
      )}
    </div>
  );
}

/** The outline as a sheet over the Map, as a phone shows it. */
export function ProjectsSheet({
  onClose,
  onDismiss,
  children,
}: {
  onClose: () => void;
  /** Closes it as Escape does, for a drag down. */
  onDismiss: () => void;
  children: ReactNode;
}) {
  const panel = useRef<HTMLElement>(null);
  const sheet = useSheet(panel, "half", onDismiss);
  useFocusIn(panel);
  return (
    <aside
      ref={panel}
      tabIndex={-1}
      className="inspector"
      data-testid="projects"
      aria-label="Outline"
      data-detent={sheet.detent}
    >
      <header {...sheet.header}>
        {sheet.handle}
        <h2>Outline</h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          <Icon of={X} />
        </button>
      </header>
      {children}
    </aside>
  );
}
