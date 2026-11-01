// The Overview home: what needs the user, the threads at work, the threads to start next, and each phase's progress,
// as lists to scan; each row opens what it names. Nothing here acts on the plan: a decision is made in Needs you,
// where it is the entry's own, and a row here only opens it.

import type { Counts, Item } from "./api";
import { Badge, lineStyle } from "./Badge";
import { ChevronRight, Icon, Inbox } from "./icons";
import type { Line } from "./lines";
import { awaitingApproval, countText, type Need, needView } from "./needs";
import type { Outline } from "./outline";
import { Prose } from "./Prose";
import { StageCounts } from "./Stage";
import { threadLists } from "./threadLists";
import type { WavesView } from "./waves";

/** How many rows each section shows before it says how many more. */
const NEEDS_SHOWN = 5;
const THREADS_SHOWN = 8;

export interface OverviewProps {
  needs: Need[];
  lines: ReadonlyMap<string, Line>;
  /** Each thread's tasks by stage. */
  counts: ReadonlyMap<string, Counts>;
  waves: WavesView | null;
  outline: Outline | null;
  /** The plan's items by id, for each phase's own progress. */
  items: ReadonlyMap<string, Pick<Item, "progress">>;
  /** Opens Needs you, at the entry `key` names if given. */
  onNeeds: (key?: string) => void;
  onPickThread: (id: string) => void;
  onSelectPhase: (id: string) => void;
  /** The view to see everything a section stands for. */
  onView: (view: "map" | "threads") => void;
}

export function Overview({
  needs,
  lines,
  counts,
  waves,
  outline,
  items,
  onNeeds,
  onPickThread,
  onSelectPhase,
  onView,
}: OverviewProps) {
  const { working, next } = threadLists(counts, waves, awaitingApproval(needs));
  const thread = (id: string) => {
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
          onClick={() => onPickThread(id)}
        >
          <Badge line={line} />
          <span className="what">
            <Prose text={line?.name ?? id} />
            {tally === undefined ? null : (
              <span className="detail">
                <StageCounts counts={tally} />
              </span>
            )}
          </span>
          <span className="go">
            <Icon of={ChevronRight} size={16} />
          </span>
        </button>
      </li>
    );
  };
  const more = (shown: number, all: number, label: string, open: () => void) =>
    all <= shown ? null : (
      <button type="button" className="quiet more" onClick={open}>
        {all - shown} more {label}
      </button>
    );
  return (
    <section className="overview" data-testid="overview" aria-labelledby="overview-title">
      <h1 id="overview-title" className="view-title">
        Overview
      </h1>

      <section className="overview-section" aria-labelledby="overview-needs">
        <header>
          <h2 id="overview-needs">Needs you</h2>
          <span className="count muted">{countText(needs.length)}</span>
          <span className="grow" />
          {needs.length === 0 ? null : (
            <button type="button" className="quiet" onClick={() => onNeeds()}>
              Open the list
            </button>
          )}
        </header>
        {needs.length === 0 ? (
          <p className="empty">Nothing needs you. New approvals and blockers show here.</p>
        ) : (
          <ul className="rows">
            {needs.slice(0, NEEDS_SHOWN).map((need) => {
              const view = needView(need, lines);
              return (
                <li key={view.key}>
                  <button
                    type="button"
                    className="row overview-row"
                    data-row
                    style={lineStyle(view.line)}
                    onClick={() => onNeeds(view.key)}
                  >
                    {view.line === undefined ? (
                      <span className="badge plain" aria-hidden="true">
                        <Icon of={Inbox} size={16} />
                      </span>
                    ) : (
                      <Badge line={view.line} />
                    )}
                    <span className="what">
                      <Prose text={view.text} />
                      {view.detail === null ? null : <span className="detail">{view.detail}</span>}
                    </span>
                    <span className="go">
                      <Icon of={ChevronRight} size={16} />
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        {more(NEEDS_SHOWN, needs.length, "in Needs you", () => onNeeds())}
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
            <ul className="rows">{working.slice(0, THREADS_SHOWN).map(thread)}</ul>
          )}
          {more(THREADS_SHOWN, working.length, "threads", () => onView("threads"))}
        </section>

        <section className="overview-section" aria-labelledby="overview-next">
          <header>
            <h2 id="overview-next">Up next</h2>
            <span className="count muted">{next.length}</span>
            <span className="grow" />
            <button type="button" className="quiet" onClick={() => onView("threads")}>
              All waves
            </button>
          </header>
          {next.length === 0 ? (
            <p className="empty">No approved thread has work ready to start.</p>
          ) : (
            <ul className="rows">{next.slice(0, THREADS_SHOWN).map(thread)}</ul>
          )}
          {more(THREADS_SHOWN, next.length, "ready", () => onView("threads"))}
        </section>
      </div>

      <section className="overview-section overview-phases" aria-labelledby="overview-phases">
        <header>
          <h2 id="overview-phases">Phases</h2>
          <span className="count muted">{outline?.phases.length ?? 0}</span>
          <span className="grow" />
          <button type="button" className="quiet" onClick={() => onView("map")}>
            Open the Map
          </button>
        </header>
        {outline === null || outline.phases.length === 0 ? (
          <p className="empty">No phases yet: add an item at the top level, and work inside it.</p>
        ) : (
          <ul className="rows">
            {outline.phases.map((phase) => {
              const progress = items.get(phase.id)?.progress;
              return (
                <li key={phase.id}>
                  <button
                    type="button"
                    className="row overview-row"
                    data-row
                    data-overview-phase={phase.id}
                    onClick={() => onSelectPhase(phase.id)}
                  >
                    <span className="what">
                      <Prose text={phase.title} />
                      <span className="detail">
                        {progress === undefined ? (
                          `${phase.threads.length} threads`
                        ) : (
                          <StageCounts counts={progress.counts} />
                        )}
                      </span>
                    </span>
                    <span className="go">
                      <Icon of={ChevronRight} size={16} />
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </section>
  );
}
