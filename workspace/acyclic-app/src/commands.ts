// The command menu's contents: what it can do and go to, and which of those a query finds, best first. Pure, so the
// ranking is tested without a browser.

import type { Model } from "./map/model";
import { inArchived } from "./map/model";
import { natural } from "./natural";
import type { View } from "./Shell";

export type Run =
  | { kind: "view"; view: View }
  | { kind: "add" }
  | { kind: "archived" }
  | { kind: "refresh" }
  | { kind: "item"; id: string }
  | { kind: "thread"; id: string }
  | { kind: "project"; id: string }
  /** Something to do to the item whose panel shows. */
  | { kind: "on-item"; action: OnItem }
  /** The target of a move or link waiting for one: an item, or the top level for a move. */
  | { kind: "target"; id: string | null };

export type OnItem = "edit" | "move" | "waits-for" | "relates-to" | "add-inside";

export type Group = "Actions" | "Threads" | "Projects" | "Items";
/** The order groups list in, and break ties in. */
export const GROUP_ORDER: readonly Group[] = ["Actions", "Threads", "Projects", "Items"];

export interface Entry {
  /** Unique among entries, for the list's keys and the active option. */
  key: string;
  group: Group;
  title: string;
  /** A second, quieter line: an item's id and project, a thread's number. */
  detail: string | null;
  /** Searched beside the title: an item's or project's id, a thread's number. */
  also: string;
  run: Run;
  /** The title and `also`, folded once when listed rather than on every key typed. */
  folded: { title: string; also: string };
}

/** An entry, folded for matching. */
export function entry(fields: Omit<Entry, "folded">): Entry {
  return { ...fields, folded: { title: fold(fields.title), also: fold(fields.also) } };
}

/** At most this many found, which is more than a screen holds. */
export const MOST = 50;

export interface Plan {
  model: Model;
  /** Threads by number, each with its badge's number and its name. */
  threads: readonly { id: string; number: string | null; name: string }[];
  /** The threads at work, listed before a query is typed. */
  working: ReadonlySet<string>;
  showArchived: boolean;
  /** The item whose panel shows, which the menu offers to act on. */
  item?: { id: string; title: string } | null;
}

const VIEW_NAMES: Record<View, string> = { overview: "Overview", map: "Map", threads: "Threads", needs: "Needs you" };

/** Every entry the menu can find in `plan`: actions, threads, projects, then items, archived ones only while shown. */
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
      key: "archived",
      group: "Actions",
      title: plan.showArchived ? "Hide archived items" : "Show archived items",
      detail: null,
      also: "",
      run: { kind: "archived" },
    }),
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
            ["relates-to", `${item.title} relates to…`],
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
      // Its badge carries the number, which a search still finds.
      detail: null,
      also: thread.number ?? thread.id,
      run: { kind: "thread", id: thread.id },
    }),
  );
  const projects: Entry[] = [];
  const items: Entry[] = [];
  for (const { id, title, project, isProject } of listed(plan))
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
          detail: where(project),
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
      .map(({ id, title, project, isProject }) =>
        entry({
          key: `target:${id}`,
          group: isProject ? "Projects" : "Items",
          title,
          detail: where(project),
          also: id,
          run: { kind: "target", id },
        }),
      ),
  ];
}

/** Where an item is; its id, which finds it too, would only be plumbing under its title. */
const where = (project: string | null) => (project === null ? null : `in ${project}`);

/**
 * Every item shown, with the title of the project it is in, and whether it is a project: a top-level item with
 * children.
 */
function listed({ model, showArchived }: Plan) {
  const shown = (id: string) => showArchived || !inArchived(model, id);
  /** The title of the project an item is in, from its top-level ancestor; parents that loop end the walk. */
  const projectOf = (id: string) => {
    const seen = new Set([id]);
    let top = id;
    for (let up = model.items.get(id)?.parent ?? null; up !== null && model.items.has(up) && !seen.has(up); ) {
      seen.add(up);
      top = up;
      up = model.items.get(up)?.parent ?? null;
    }
    return top === id ? null : (model.items.get(top)?.title ?? null);
  };
  const roots = new Set(model.roots);
  return [...model.items.values()]
    .filter((item) => shown(item.id))
    .map((item) => ({
      id: item.id,
      title: item.title,
      project: projectOf(item.id),
      isProject: roots.has(item.id) && (model.children.get(item.id)?.length ?? 0) > 0,
    }));
}

/** Folded for matching: case and accents set aside, so "resume" finds "Résumé". */
export const fold = (text: string) => text.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();

/**
 * The entries `query` finds, best first, at most `MOST`: every word of it must be in the title or the id. A title
 * starting with the whole query comes first, then one where every word starts a word, then any match in the title,
 * then one in the id alone; ties go by group, then title. Then grouped, the group of the best first. With nothing
 * typed, the actions and the threads at work, or for a waiting move or link, the projects it can go to.
 */
export function rank(query: string, all: readonly Entry[], working: ReadonlySet<string>): Entry[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
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
    const score = title.startsWith(whole)
      ? 0
      : starts.every((start) => start.test(title))
        ? 1
        : words.every((word) => title.includes(word))
          ? 2
          : 3;
    found.push({ entry, score });
  }
  const best = found
    .sort((a, b) => a.score - b.score || byGroup(a.entry, b.entry))
    .slice(0, MOST)
    .map(({ entry }) => entry);
  // Listed by group, the group of the best first, each in its own order: what ↑ and ↓ move through is what shows.
  const groups = [...new Set(best.map((one) => one.group))];
  return groups.flatMap((group) => best.filter((one) => one.group === group));
}

const escaped = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

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
