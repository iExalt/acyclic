// Initiatives in the lists: the switcher's rows and their open counts, the initiative in context, and how the Overview,
// Up next and Needs you group their rows under initiatives. Pure, so the rules are tested without a browser.

import type { Initiative } from "./api";
import type { Model } from "./map/model";
import type { Need } from "./needs";
import { oneOf } from "./outline";

/** What a row in no initiative is called, in a heading, a label or the switcher. */
export const NO_INITIATIVE = "No initiative";

export interface InitiativeRow {
  id: string;
  title: string;
  /** Items under it, at any depth, that are not finished or canceled: the leaves, as a parent's own stage is theirs. */
  open: number;
}

/**
 * The switcher's rows: every initiative with its open count, the busiest first, then by title, so the order stays put
 * as the one in context changes.
 */
export function switcherRows(initiatives: readonly Initiative[], model: Model | null): InitiativeRow[] {
  const open = new Map<string, number>();
  for (const item of model?.items.values() ?? []) {
    if (item.initiative === null || (model?.children.get(item.id) ?? []).length > 0) continue;
    if (item.stage === "done" || item.stage === "canceled") continue;
    open.set(item.initiative, (open.get(item.initiative) ?? 0) + 1);
  }
  return initiatives
    .map((one) => ({ id: one.id, title: one.title, open: open.get(one.id) ?? 0 }))
    .sort((a, b) => b.open - a.open || a.title.localeCompare(b.title) || a.id.localeCompare(b.id));
}

/**
 * The initiative in context: the one the user chose, while it exists, else the first of the switcher, so New item
 * always has somewhere to go. Null only when there are no initiatives.
 */
export function contextOf(chosen: string | null, rows: readonly { id: string }[]): string | null {
  return rows.find((one) => one.id === chosen)?.id ?? rows[0]?.id ?? null;
}

/** The initiative's title, or null for none in context. */
export function contextTitle(context: string | null, rows: readonly InitiativeRow[]): string | null {
  return rows.find((one) => one.id === context)?.title ?? null;
}

/** The initiatives each thread has tasks in; an item in none counts as `null`. A thread with no task has no entry. */
export function threadInitiatives(model: Model): Map<string, Set<string | null>> {
  const out = new Map<string, Set<string | null>>();
  for (const item of model.items.values()) {
    const leaf = (model.children.get(item.id) ?? []).length === 0;
    // A leaf in no thread is the thread of one it stands for.
    const thread = item.thread ?? (leaf ? oneOf(item.id) : null);
    if (thread === null) continue;
    const there = out.get(thread) ?? new Set<string | null>();
    there.add(item.initiative);
    out.set(thread, there);
  }
  return out;
}

/** What a list needs to put its rows under initiatives. */
export interface ListScope {
  /** Every initiative's id in the switcher's order, and the one in context, which goes first. */
  order: readonly string[];
  context: string | null;
  titles: ReadonlyMap<string, string>;
  /** The initiatives a thread has tasks in; one with none is in none. */
  ofThread: (thread: string) => ReadonlySet<string | null>;
  ofNeed: (need: Need) => ReadonlySet<string | null>;
  ofProject: (project: string) => ReadonlySet<string | null>;
}

/** A group's heading: its initiative's title, or "No initiative". */
export const groupTitle = (initiative: string | null, titles: ReadonlyMap<string, string>) =>
  initiative === null ? NO_INITIATIVE : (titles.get(initiative) ?? initiative);

export interface Group<T> {
  /** The initiative, or null for the rows in none. */
  initiative: string | null;
  rows: T[];
}

/**
 * `rows` under the initiatives `of` names for each: the one in context first, then the others in `order`, then rows in
 * none. A row in several initiatives is under each; one that names none, or only one that isn't in `order`, goes with
 * the rows in none. Groups with no rows are left out; rows keep their order.
 */
export function groupRows<T>(
  rows: readonly T[],
  of: (row: T) => ReadonlySet<string | null> | readonly (string | null)[],
  order: readonly string[],
  context: string | null,
): Group<T>[] {
  const ranked = context === null ? [...order] : [context, ...order.filter((id) => id !== context)];
  const known = new Set(ranked);
  const groups = new Map<string | null, T[]>();
  for (const row of rows) {
    const named = [...of(row)].map((id) => (id !== null && known.has(id) ? id : null));
    for (const id of new Set(named.length === 0 ? [null] : named)) groups.set(id, [...(groups.get(id) ?? []), row]);
  }
  return [...ranked, null].flatMap((id) => {
    const found = groups.get(id);
    return found === undefined ? [] : [{ initiative: id, rows: found }];
  });
}

/**
 * At most `limit` rows across `groups`, what a list shows before it says how many more: first one row for each group,
 * in order, while it still fits, so no populated group drops out; then the rest of each group's rows, in order, up to
 * the limit. Each group keeps its heading only while it has rows shown, and its rows their order. `fits(rows, groups)`
 * is asked of the rows and headings shown with the next row added, and stops what no longer fits.
 */
export function shownGroups<T>(
  groups: readonly Group<T>[],
  limit: number,
  fits: (rows: number, groups: number) => boolean = () => true,
): Group<T>[] {
  const taken = groups.map(() => 0);
  let rows = 0;
  let headed = 0;
  // One row to each group while it fits, then the rest in group order.
  for (const [at, group] of groups.entries()) {
    if (group.rows.length === 0 || rows >= limit || !fits(rows + 1, headed + 1)) continue;
    taken[at] = 1;
    rows += 1;
    headed += 1;
  }
  for (const [at, group] of groups.entries()) {
    if ((taken[at] ?? 0) === 0) continue;
    while ((taken[at] ?? 0) < group.rows.length && rows < limit && fits(rows + 1, headed)) {
      taken[at] = (taken[at] ?? 0) + 1;
      rows += 1;
    }
  }
  return groups.flatMap((group, at) =>
    (taken[at] ?? 0) === 0 ? [] : [{ ...group, rows: group.rows.slice(0, taken[at]) }],
  );
}

/**
 * Whether `rows` rows under `groups` headings of Up next fit the first view at 1280 x 800: measured, each row takes
 * 60 px and each heading 44 px, and eight rows under two headings end at 698 px, so a bottom of at most 770 px (30 px
 * short of the fold) allows eight rows under three headings, or fewer rows under more, six at most with a heading each; the rest is counted in "more".
 */
export const fitsFirstView = (rows: number, groups: number) => 60 * rows + 44 * groups <= 640;

/**
 * A row's trailing label on a phone, where there are no headings: its initiative's title, the first when it is in
 * several with a count of the rest; "No initiative" for one in none, or in one the titles lack.
 */
export function rowLabel(
  initiatives: ReadonlySet<string | null> | readonly (string | null)[],
  titles: ReadonlyMap<string, string>,
  context: string | null = null,
): string {
  // An initiative the titles lack is in none, as the headings group it.
  const ids = [...new Set([...initiatives].map((id) => (id !== null && titles.has(id) ? id : null)))];
  // The one in context reads first, then the rest as given, and none last.
  const rank = (id: string | null) => (id === null ? 2 : id === context ? 0 : 1);
  ids.sort((a, b) => rank(a) - rank(b));
  const first = ids[0] ?? null;
  const name = first === null ? NO_INITIATIVE : (titles.get(first) ?? NO_INITIATIVE);
  return ids.length > 1 ? `${name} +${ids.length - 1}` : name;
}

/**
 * The initiatives a needs-you entry is about: those of every item it names, whether as `item`, or among its `items`,
 * `dependents` and `open`, and of the tasks of every thread it names; none when it names nothing the plan holds.
 */
export function needInitiatives(
  need: Need,
  model: Model | null,
  ofThread: ReadonlyMap<string, ReadonlySet<string | null>>,
): Set<string | null> {
  const row = (value: unknown) =>
    typeof value === "object" && value !== null && typeof (value as { id?: unknown }).id === "string"
      ? (value as { id: string; initiative?: unknown })
      : null;
  const list = (value: unknown) => (Array.isArray(value) ? value : []);
  const found = new Set<string | null>();
  for (const value of [need.item, ...list(need.items), ...list(need.dependents), ...list(need.open)]) {
    const item = row(value);
    if (item === null) continue;
    // The entry says where its item is; failing that, the plan does.
    if (typeof item.initiative === "string" || item.initiative === null) found.add(item.initiative);
    else {
      const known = model?.items.get(item.id);
      if (known !== undefined) found.add(known.initiative);
    }
  }
  for (const thread of [need.thread, ...list(need.threads)]) {
    const id = row(thread)?.id;
    for (const initiative of (id === undefined ? undefined : ofThread.get(id)) ?? []) found.add(initiative);
  }
  return found.size === 0 ? new Set([null]) : found;
}

/**
 * The project New item starts in: the one the selected item is in, when it is in the initiative in context, else none,
 * so the item goes under the initiative in context, as a project of its own.
 */
export function startProject(
  current: string | null,
  initiativeOf: (project: string) => string | null | undefined,
  context: string | null,
): string | null {
  return current !== null && initiativeOf(current) === context ? current : null;
}

/** Where a new item goes: under the item it is added inside, else the project chosen, else the initiative in context. */
export function newParent(parent: string | null, project: string | null, context: string | null): string | null {
  return parent ?? project ?? context;
}
