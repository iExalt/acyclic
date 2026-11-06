// The command menu's contents: what it can do and go to, and which of those a query finds, best first. Pure, so the
// ranking is tested without a browser.

import type { Model } from "./map/model";
import { natural } from "./natural";
import type { View } from "./Shell";

export type Run =
  | { kind: "view"; view: View }
  | { kind: "add" }
  | { kind: "refresh" }
  | { kind: "item"; id: string }
  | { kind: "thread"; id: string }
  | { kind: "project"; id: string }
  /** Something to do to the item whose panel shows. */
  | { kind: "on-item"; action: OnItem }
  /** The target of a move or link waiting for one: an item, or the top level for a move. */
  | { kind: "target"; id: string | null };

export type OnItem = "edit" | "move" | "waits-for" | "add-inside";

export type Group = "Actions" | "Threads" | "Projects" | "Items";
/** The order groups list in, and break ties in. */
export const GROUP_ORDER: readonly Group[] = ["Actions", "Threads", "Projects", "Items"];

export interface Entry {
  /** Unique among entries, for the list's keys and the active option. */
  key: string;
  group: Group;
  title: string;
  /** A second, quieter line: an item's parent and root project. */
  detail: string | null;
  /** Searched beside the title: an item's or project's id. */
  also: string;
  run: Run;
  /** The title and `also`, folded once when listed rather than on every key typed. */
  folded: { title: string; also: string };
}

/** An entry, folded for matching. */
export function entry(fields: Omit<Entry, "folded">): Entry {
  return { ...fields, folded: { title: searchText(fields.title), also: searchText(fields.also) } };
}

/** At most this many found, which is more than a screen holds. */
export const MOST = 50;

export interface Plan {
  model: Model;
  /** Threads in the order they are listed, each with its name. */
  threads: readonly { id: string; name: string }[];
  /** The threads at work, listed before a query is typed. */
  working: ReadonlySet<string>;
  /** The item whose panel shows, which the menu offers to act on. */
  item?: { id: string; title: string } | null;
}

const VIEW_NAMES: Record<View, string> = { overview: "Overview", map: "Map", threads: "Threads", needs: "Needs you" };

/** Every entry the menu can find in `plan`: actions, threads, projects, then items. */
export function entries(plan: Plan): Entry[] {
  const actions: Entry[] = [
    ...(["overview", "map", "threads", "needs"] as const).map((view) =>
      entry({
        key: `view:${view}`,
        group: "Actions",
        title: `Go to ${VIEW_NAMES[view]}`,
        detail: null,
        also: "",
        run: { kind: "view", view },
      }),
    ),
    entry({ key: "add", group: "Actions", title: "New item", detail: null, also: "add", run: { kind: "add" } }),
    entry({
      key: "refresh",
      group: "Actions",
      title: "Refresh",
      detail: null,
      also: "reload",
      run: { kind: "refresh" },
    }),
  ];
  const item = plan.item ?? null;
  const onItem: Entry[] =
    item === null
      ? []
      : (
          [
            ["edit", `Edit ${item.title}`],
            ["move", `Move ${item.title}…`],
            ["waits-for", `${item.title} waits for…`],
            ["add-inside", `Add inside ${item.title}`],
          ] as const
        ).map(([action, title]) =>
          entry({
            key: `on-item:${action}`,
            group: "Actions",
            title,
            detail: null,
            also: "",
            run: { kind: "on-item", action },
          }),
        );
  const threads = plan.threads.map((thread) =>
    entry({
      key: `thread:${thread.id}`,
      group: "Threads",
      title: thread.name,
      detail: null,
      also: "",
      run: { kind: "thread", id: thread.id },
    }),
  );
  const projects: Entry[] = [];
  const items: Entry[] = [];
  for (const { id, title, parent, project, isProject } of listed(plan))
    if (isProject)
      projects.push(
        entry({ key: `project:${id}`, group: "Projects", title, detail: null, also: id, run: { kind: "project", id } }),
      );
    else
      items.push(
        entry({
          key: `item:${id}`,
          group: "Items",
          title,
          detail: where(project, parent),
          also: id,
          run: { kind: "item", id },
        }),
      );
  return [...onItem, ...actions, ...threads, ...projects, ...items];
}

/**
 * What a move or a link waiting for its target can be pointed at from the menu: any item shown, projects among them,
 * and for a move the top level first.
 */
export function targets(plan: Plan, pick: "move" | "link", from: string | null = null): Entry[] {
  const { model } = plan;
  // Never the item itself, nor, for a move, where it already is or anything inside it: none of those moves it.
  const parent = from === null ? null : (model.items.get(from)?.parent ?? null);
  const inside = (id: string) => {
    const seen = new Set<string>();
    for (let up: string | null = id; up !== null && !seen.has(up); up = model.items.get(up)?.parent ?? null) {
      if (up === from) return true;
      seen.add(up);
    }
    return false;
  };
  const left = (id: string) => (pick === "move" ? !inside(id) && id !== parent : id !== from);
  const top =
    pick === "move" && (from === null || parent !== null)
      ? [
          entry({
            key: "target:top",
            group: "Actions",
            title: "Top level",
            detail: null,
            also: "",
            run: { kind: "target", id: null },
          }),
        ]
      : [];
  return [
    ...top,
    ...listed(plan)
      .filter(({ id }) => left(id))
      .map(({ id, title, parent: itemParent, project, isProject }) =>
        entry({
          key: `target:${id}`,
          group: isProject ? "Projects" : "Items",
          title,
          detail: where(project, itemParent),
          also: id,
          run: { kind: "target", id },
        }),
      ),
  ];
}

/** Where an item is; its IDs, which find it too, would only be plumbing under its title. */
const where = (project: { id: string; title: string } | null, parent: { id: string; title: string } | null) =>
  project === null
    ? null
    : parent !== null && parent.id !== project.id
      ? `in ${parent.title} · ${project.title}`
      : `in ${project.title}`;

/**
 * Every item shown, with its parent, root project and whether it is a project: a top-level item with children.
 */
function listed({ model }: Plan) {
  /** The root project an item is in, from its top-level ancestor; parents that loop end the walk. */
  const projectOf = (id: string) => {
    const seen = new Set([id]);
    let top = id;
    for (let up = model.items.get(id)?.parent ?? null; up !== null && model.items.has(up) && !seen.has(up); ) {
      seen.add(up);
      top = up;
      up = model.items.get(up)?.parent ?? null;
    }
    const project = model.items.get(top);
    return top === id || project === undefined ? null : { id: top, title: project.title };
  };
  const roots = new Set(model.roots);
  return [...model.items.values()].map((item) => {
    const parent = item.parent === null ? null : (model.items.get(item.parent) ?? null);
    return {
      id: item.id,
      title: item.title,
      parent: parent === null ? null : { id: parent.id, title: parent.title },
      project: projectOf(item.id),
      isProject: roots.has(item.id) && (model.children.get(item.id)?.length ?? 0) > 0,
    };
  });
}

/** Folded for matching: case and accents set aside, so "resume" finds "Résumé". */
export const fold = (text: string) => text.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();

/**
 * The entries `query` finds, best first, at most `MOST`: exact titles first, then title prefixes, embedded phrases,
 * word starts, other title matches and ID matches. Every word must occur in the title or ID; ties go by group, then
 * title. With nothing typed, the actions and the threads at work, or for a waiting move or link, the projects it can go to.
 */
export function rank(query: string, all: readonly Entry[], working: ReadonlySet<string>): Entry[] {
  const words = searchText(query).split(" ").filter(Boolean);
  const byGroup = (a: Entry, b: Entry) =>
    GROUP_ORDER.indexOf(a.group) - GROUP_ORDER.indexOf(b.group) || natural(a.title, b.title);
  if (words.length === 0)
    return all
      .filter(
        (entry) =>
          entry.group === "Actions" ||
          (entry.run.kind === "thread" && working.has(entry.run.id)) ||
          // A move's or link's targets offer the projects, the places most moves go, before anything is typed.
          (entry.run.kind === "target" && entry.group === "Projects"),
      )
      .slice(0, MOST);
  const whole = words.join(" ");
  const starts = words.map((word) => new RegExp(`(^|[^\\p{L}\\p{N}])${escaped(word)}`, "u"));
  const found: { entry: Entry; score: number }[] = [];
  for (const entry of all) {
    const { title, also } = entry.folded;
    if (!words.every((word) => title.includes(word) || also.includes(word))) continue;
    const score =
      title === whole
        ? 0
        : title.startsWith(whole)
          ? 1
          : words.length > 1 && title.includes(whole)
            ? 2
            : starts.every((start) => start.test(title))
              ? 3
              : words.every((word) => title.includes(word))
                ? 4
                : 5;
    found.push({ entry, score });
  }
  const best = found
    .sort((a, b) => a.score - b.score || byGroup(a.entry, b.entry))
    .slice(0, MOST)
    .map(({ entry }) => entry);
  return best;
}

const escaped = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Matching ignores case, accents and whitespace runs. */
const searchText = (text: string) => fold(text).trim().replace(/\s+/g, " ");

/** Whether the platform's command key is ⌘ rather than Ctrl. */
export const isMac = (platform: string) => /mac|iphone|ipad|ipod/i.test(platform);

/** The browser's platform: User-Agent Client Hints' where the browser has them, else the deprecated `platform`. */
export function platformOf(nav: { platform: string; userAgentData?: { platform?: string } }): string {
  return nav.userAgentData?.platform || nav.platform;
}

/** Whether this browser runs on a Mac, an iPhone or an iPad, whose command key is ⌘. */
export const onMac = () => isMac(platformOf(navigator as Navigator & { userAgentData?: { platform?: string } }));

/**
 * Whether a key press opens (or closes) the command menu: ⌘K on a Mac, Ctrl+K elsewhere, with no other modifier, and
 * never a key an input method is composing with, which would take a character from the user's typing.
 */
export function opensMenu(
  event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "isComposing" | "keyCode">,
  mac: boolean,
): boolean {
  if (event.isComposing || event.keyCode === 229) return false;
  if (event.key.toLowerCase() !== "k" || event.altKey || event.shiftKey) return false;
  return mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

/** Whether a key press asks to undo: ⌘Z on a Mac, Ctrl+Z elsewhere, without Shift (which redoes) or composing. */
export function undoes(
  event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "isComposing" | "keyCode">,
  mac: boolean,
): boolean {
  if (event.isComposing || event.keyCode === 229) return false;
  if (event.key.toLowerCase() !== "z" || event.altKey || event.shiftKey) return false;
  return mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}
