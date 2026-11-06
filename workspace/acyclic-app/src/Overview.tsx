// The Overview home: what needs the user, the threads at work, the threads to start next, and each project's progress,
// as lists to scan; each row opens what it names. Nothing here acts on the plan: a decision is made in Needs you's
// view, where it is the entry's own, and a row here, a preview of it, only opens it there.

import { Fragment, type ReactNode } from "react";
import type { Counts, Item } from "./api";
import { Badge, lineStyle, NeedBadge } from "./Badge";
import { ChevronRight, Icon } from "./icons";
import { fitsFirstView, type Group, groupRows, groupTitle, type ListScope, rowLabel, shownGroups } from "./initiatives";
import type { Line } from "./lines";
import { countText, type Need, needView } from "./needs";
import type { Outline } from "./outline";
import { Prose } from "./Prose";
import { StageCounts } from "./Stage";
import { threadLists } from "./threadLists";
import { COLUMNS, SIDE_BY_SIDE, useWide } from "./wide";

/**
 * How many rows each section shows before it says how many more: from 700 px, as many in Needs you as In progress
 * shows, so the three columns of a wide screen end level; on a phone, where the sections stack, fewer, so the rest is
 * reached sooner.
 */
const THREADS_SHOWN = 8;
const NEEDS_SHOWN_STACKED = 5;

export interface OverviewProps {
  needs: Need[];
  lines: ReadonlyMap<string, Line>;
  /** An item's title, for an entry in Needs you that names it by id. */
  titleOf: (id: string) => string;
  /** Each thread's items by stage. */
  counts: ReadonlyMap<string, Counts>;
  outline: Outline | null;
  /** The initiatives the lists group their rows under. */
  scope: ListScope;
  /** The plan's items by id, for each project's own progress. */
  items: ReadonlyMap<string, Pick<Item, "progress">>;
  /** Goes to Needs you, at the entry `key` names if given. */
  onNeeds: (key?: string) => void;
  onPickThread: (id: string) => void;
  onSelectProject: (id: string) => void;
  /** The view to see everything a section stands for. */
  onView: (view: "map" | "threads") => void;
}

export function Overview({
  needs,
  lines,
  titleOf,
  counts,
  outline,
  scope,
  items,
  onNeeds,
  onPickThread,
  onSelectProject,
  onView,
}: OverviewProps) {
  const { working, next } = threadLists(counts);
  // Wide, rows stand under their initiative's heading; on a phone, which has no room for headings, each says its own.
  const headed = useWide(SIDE_BY_SIDE);
  const needsShown = useWide(COLUMNS) ? THREADS_SHOWN : NEEDS_SHOWN_STACKED;
  // In three columns, an Up next half as long as the longest list beside it stacks under In progress, and the projects
  // take its column, rather than leave most of it empty and start below the first view.
  const longest = Math.max(Math.min(needs.length, needsShown), Math.min(working.length, THREADS_SHOWN));
  const shortNext = 2 * Math.max(1, Math.min(next.length, THREADS_SHOWN)) <= longest;
  const thread = (id: string, label: string | null) => {
    const line = lines.get(id);
    const tally = counts.get(id);
    return (
      <li key={id}>
        <button
          type="button"
          className="row overview-row"
          data-row
          style={lineStyle(line)}
          data-overview-thread={id}
          data-task={line?.task ?? undefined}
          onClick={() => onPickThread(id)}
        >
          <Badge line={line} />
          <span className="what">
            <span className="row-title" title={line?.name ?? id}>
              <Prose text={line?.name ?? id} />
            </span>
            {tally === undefined ? null : (
              <span className="detail">
                <StageCounts counts={tally} />
              </span>
            )}
          </span>
          {label === null ? null : <span className="initiative-tag">{label}</span>}
          <span className="go">
            <Icon of={ChevronRight} size={16} />
          </span>
        </button>
      </li>
    );
  };
  /** `rows` grouped, and the first `limit` of them, within `fits` once headings take room, as the list shows them. */
  const laid = <T,>(
    rows: readonly T[],
    of: (row: T) => ReadonlySet<string | null>,
    limit: number,
    fits?: (rows: number, groups: number) => boolean,
  ) => {
    const groups = groupRows(rows, of, scope.order, scope.context);
    const shown = shownGroups(groups, limit, headed ? fits : undefined);
    // A thread in several initiatives is shown under each, and counted once.
    return { groups, shown, count: new Set(shown.flatMap((group) => group.rows)).size };
  };
  const needsLaid = laid(needs, scope.ofNeed, needsShown);
  const workingLaid = laid(working, scope.ofThread, THREADS_SHOWN);
  // Up next stays whole in the first view at 1280 x 800, so headings count against its rows.
  const nextLaid = laid(next, scope.ofThread, THREADS_SHOWN, fitsFirstView);
  /** The rows laid out as `laid` says: under their initiatives' headings when wide, else one list with labels. */
  const listed = <T,>(
    layout: ReturnType<typeof laid<T>>,
    of: (row: T) => ReadonlySet<string | null>,
    row: (one: T, label: string | null) => ReactNode,
  ) => {
    const { groups, shown } = layout;
    if (!headed)
      return (
        <ul className="rows">
          {shown.flatMap((group) => group.rows.map((one) => row(one, rowLabel(of(one), scope.titles, scope.context))))}
        </ul>
      );
    return shown.map((group: Group<T>) => (
      <Fragment key={group.initiative ?? ""}>
        <h3 className="group-head" data-initiative={group.initiative ?? ""}>
          <span>{groupTitle(group.initiative, scope.titles)}</span>
          <span className="group-n">{groups.find((one) => one.initiative === group.initiative)?.rows.length}</span>
        </h3>
        <ul className="rows">{group.rows.map((one) => row(one, null))}</ul>
      </Fragment>
    ));
  };
  const more = (shown: number, all: number, label: string, open: () => void) =>
    all <= shown ? null : (
      <button type="button" className="quiet more" onClick={open}>
        {all - shown} more {label}
      </button>
    );
  return (
    <section
      className="overview"
      data-testid="overview"
      data-short-next={shortNext || undefined}
      aria-labelledby="overview-title"
    >
      <h1 id="overview-title" className="view-title">
        Overview
      </h1>

      <section className="overview-section" aria-labelledby="overview-needs">
        <header>
          <h2 id="overview-needs">Needs you</h2>
          <span className="count muted">{countText(needs.length)}</span>
          <span className="grow" />
          {/* With more than fit, the link under the rows opens the list, and one link to it is enough. */}
          {needs.length === 0 || needs.length > needsShown ? null : (
            <button type="button" className="quiet" onClick={() => onNeeds()}>
              Open the list
            </button>
          )}
        </header>
        {needs.length === 0 ? (
          <p className="empty">
            Nothing needs you. Conflicts, cycles and done work still waiting on open work show here.
          </p>
        ) : (
          listed(needsLaid, scope.ofNeed, (need, label) => {
            const view = needView(need, lines, titleOf);
            return (
              <li key={view.key}>
                <button
                  type="button"
                  className="row overview-row"
                  data-row
                  style={lineStyle(view.line)}
                  onClick={() => onNeeds(view.key)}
                >
                  <NeedBadge line={view.line} />
                  <span className="what">
                    <Prose text={view.text} />
                    {view.detail === null ? null : <span className="detail">{view.detail}</span>}
                  </span>
                  {label === null ? null : <span className="initiative-tag">{label}</span>}
                  <span className="go">
                    <Icon of={ChevronRight} size={16} />
                  </span>
                </button>
              </li>
            );
          })
        )}
        {more(needsLaid.count, needs.length, "in Needs you", () => onNeeds())}
      </section>

      <div className="overview-pair">
        <section className="overview-section" aria-labelledby="overview-working">
          <header>
            <h2 id="overview-working">In progress</h2>
            <span className="count muted">{working.length}</span>
          </header>
          {working.length === 0 ? (
            <p className="empty">No thread has work in progress or in review.</p>
          ) : (
            listed(workingLaid, scope.ofThread, (id, label) => thread(id, label))
          )}
          {more(workingLaid.count, working.length, "in Threads", () => onView("threads"))}
        </section>

        <section className="overview-section" aria-labelledby="overview-next">
          <header>
            <h2 id="overview-next">Up next</h2>
            <span className="count muted">{next.length}</span>
            <span className="grow" />
            <button type="button" className="quiet" onClick={() => onView("threads")}>
              Open Threads
            </button>
          </header>
          {next.length === 0 ? (
            <p className="empty">No thread has work ready to start.</p>
          ) : (
            listed(nextLaid, scope.ofThread, (id, label) => thread(id, label))
          )}
          {more(nextLaid.count, next.length, "in Threads", () => onView("threads"))}
        </section>
      </div>

      <section className="overview-section overview-projects" aria-labelledby="overview-projects">
        <header>
          <h2 id="overview-projects">Projects</h2>
          <span className="count muted">{outline?.projects.length ?? 0}</span>
          <span className="grow" />
          <button type="button" className="quiet" onClick={() => onView("map")}>
            Open the Map
          </button>
        </header>
        {outline === null || outline.projects.length === 0 ? (
          <p className="empty">No projects yet: add an item, and work inside it.</p>
        ) : (
          listed(
            laid(outline.projects, (project) => scope.ofProject(project.id), Number.POSITIVE_INFINITY),
            (project) => scope.ofProject(project.id),
            (project, label) => {
              const progress = items.get(project.id)?.progress;
              return (
                <li key={project.id}>
                  <button
                    type="button"
                    className="row overview-row"
                    data-row
                    data-overview-project={project.id}
                    onClick={() => onSelectProject(project.id)}
                  >
                    <span className="what">
                      <span className="row-title" title={project.title}>
                        <Prose text={project.title} />
                      </span>
                      <span className="detail">
                        {progress === undefined ? (
                          `${project.threads.length} threads`
                        ) : (
                          <StageCounts counts={progress.counts} />
                        )}
                      </span>
                    </span>
                    {label === null ? null : <span className="initiative-tag">{label}</span>}
                    <span className="go">
                      <Icon of={ChevronRight} size={16} />
                    </span>
                  </button>
                </li>
              );
            },
          )
        )}
      </section>
    </section>
  );
}
