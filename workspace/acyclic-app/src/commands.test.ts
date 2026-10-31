import { describe, expect, it } from "vitest";
import { entries, entry, isMac, MOST, opensMenu, type Plan, rank, targets } from "./commands";
import { item } from "./map/fixture";
import { buildModel } from "./map/model";

const model = buildModel([
  item("garden", null, [], { title: "Garden" }),
  item("dig", "garden", [], { title: "Dig the beds" }),
  item("resume", "garden", [], { title: "Résumé of the bed plan" }),
  item("bedding", "garden", [], { title: "Order bedding plants" }),
  item("shed", null, [], { title: "Shed", archived: true }),
  item("paint", "shed", [], { title: "Paint the shed" }),
  item("tools", null, [], { title: "Sharpen tools" }),
]);
const plan: Plan = {
  model,
  threads: [
    { id: "thread-1", number: "1", name: "Spring beds" },
    { id: "thread-2", number: "2", name: "Autumn" },
  ],
  working: new Set(["thread-1"]),
  showArchived: false,
};
const titles = (query: string, over: Plan = plan) => rank(query, entries(over), over.working).map((one) => one.title);

describe("the command menu", () => {
  it("lists the actions and the threads at work before anything is typed", () => {
    expect(titles("")).toEqual([
      "Go to Overview",
      "Go to Map",
      "Go to Threads",
      "Open Needs you",
      "New item",
      "Show archived items",
      "Refresh",
      "Spring beds",
    ]);
  });

  it("ranks a title starting with the query, then words started, then words anywhere, then the id alone", () => {
    const one = (title: string, also = "") =>
      entry({ key: title, group: "Items", title, detail: null, also, run: { kind: "item", id: title } });
    const found = rank("bed", [one("Other", "bed-7"), one("Flowerbed"), one("Raised bed"), one("Bed plan")], new Set());
    expect(found.map((one) => one.title)).toEqual(["Bed plan", "Raised bed", "Flowerbed", "Other"]);
    // Equally good, by group, then title.
    expect(titles("bed")).toEqual(["Spring beds", "Dig the beds", "Order bedding plants", "Résumé of the bed plan"]);
    expect(titles("dig")).toEqual(["Dig the beds"]);
    // In the id only: "resume" is the item's id, and its title too once accents are set aside.
    expect(titles("RESUME")).toEqual(["Résumé of the bed plan"]);
    expect(titles("garden")).toEqual(["Garden"]);
  });

  it("finds an entry only when every word is in it, and gives a phase, not its items, for its id", () => {
    expect(titles("beds dig")).toEqual(["Dig the beds"]);
    expect(titles("beds zebra")).toEqual([]);
    const garden = rank("garden", entries(plan), plan.working)[0];
    expect(garden?.run).toEqual({ kind: "phase", id: "garden" });
    const dig = rank("dig", entries(plan), plan.working)[0];
    expect(dig?.detail).toBe("dig in Garden");
  });

  it("finds archived items only while they're shown, and says which way the switch goes", () => {
    expect(titles("paint")).toEqual([]);
    const shown = { ...plan, showArchived: true };
    expect(titles("paint", shown)).toEqual(["Paint the shed"]);
    expect(titles("archived", shown)).toEqual(["Hide archived items"]);
  });

  it("finds a thread by its number, and stops at the most a list should hold, quickly in a large plan", () => {
    expect(titles("2")).toContain("Autumn");
    // A plan the size of the largest the app is timed with: a key typed ranks it well inside a frame.
    const many = buildModel(
      Array.from({ length: 3500 }, (_, n) => item(`task-${n}`, null, [], { title: `Task ${n} of the résumé batch` })),
    );
    const listed = entries({ ...plan, model: many });
    const started = performance.now();
    for (const query of ["t", "ta", "tas", "task", "task 1", "resume batch"]) rank(query, listed, plan.working);
    const each = (performance.now() - started) / 6;
    expect(rank("task", listed, plan.working)).toHaveLength(MOST);
    expect(each).toBeLessThan(16);
  });

  it("offers a waiting move any item shown or the top level, and a link any item shown", () => {
    const move = targets(plan, "move");
    expect(move[0]?.run).toEqual({ kind: "target", id: null });
    expect(rank("garden", move, new Set()).map((one) => one.run)).toEqual([{ kind: "target", id: "garden" }]);
    expect(targets(plan, "link").some((one) => one.run.kind === "target" && one.run.id === null)).toBe(false);
    expect(rank("paint", targets(plan, "link"), new Set())).toEqual([]);
  });
});

describe("the command menu's key", () => {
  const press = (fields: Partial<KeyboardEvent>) => ({
    key: "k",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    isComposing: false,
    keyCode: 75,
    ...fields,
  });

  it("is ⌘K on a Mac and Ctrl+K elsewhere, with no other modifier", () => {
    expect(opensMenu(press({ metaKey: true }), true)).toBe(true);
    expect(opensMenu(press({ ctrlKey: true, key: "K" }), false)).toBe(true);
    expect(opensMenu(press({ ctrlKey: true }), true)).toBe(false);
    expect(opensMenu(press({ metaKey: true }), false)).toBe(false);
    expect(opensMenu(press({ metaKey: true, shiftKey: true }), true)).toBe(false);
    expect(opensMenu(press({}), true)).toBe(false);
    expect([isMac("MacIntel"), isMac("iPhone"), isMac("Linux x86_64"), isMac("Win32")]).toEqual([
      true,
      true,
      false,
      false,
    ]);
  });

  it("never takes a key an input method is composing with", () => {
    expect(opensMenu(press({ metaKey: true, isComposing: true }), true)).toBe(false);
    expect(opensMenu(press({ metaKey: true, keyCode: 229 }), true)).toBe(false);
  });
});
