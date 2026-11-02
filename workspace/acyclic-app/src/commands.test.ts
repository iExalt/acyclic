import { describe, expect, it } from "vitest";
import { entries, entry, fold, isMac, MOST, opensMenu, type Plan, platformOf, rank, targets, undoes } from "./commands";
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
      "Go to Needs you",
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

  it("finds an entry only when every word is in it, and gives a project, not its items, for its id", () => {
    expect(titles("beds dig")).toEqual(["Dig the beds"]);
    expect(titles("beds zebra")).toEqual([]);
    const garden = rank("garden", entries(plan), plan.working)[0];
    expect(garden?.run).toEqual({ kind: "project", id: "garden" });
    const dig = rank("dig", entries(plan), plan.working)[0];
    expect(dig?.detail).toBe("in Garden");
  });

  it("finds archived items only while they're shown, and says which way the switch goes", () => {
    expect(titles("paint")).toEqual([]);
    const shown = { ...plan, showArchived: true };
    expect(titles("paint", shown)).toEqual(["Paint the shed"]);
    expect(titles("archived", shown)).toEqual(["Hide archived items"]);
  });

  it("finds a thread by its number, and stops at the most a list should hold, quickly in a large plan", () => {
    expect(titles("2")).toContain("Autumn");
    // A plan the size of the largest the app is timed with: a key typed ranks it in a few passes over its titles.
    const many = buildModel(
      Array.from({ length: 3500 }, (_, n) => item(`item-${n}`, null, [], { title: `Item ${n} of the résumé batch` })),
    );
    const listed = entries({ ...plan, model: many });
    // Timed against folding every title once, in the same rounds: a slow or busy machine (a CI runner with every test
    // file at once) slows both, while ranking that grew slower in itself slows only the one. Each the best of a few
    // rounds, as load only ever slows a round down.
    const time = (work: () => void) => {
      const started = performance.now();
      work();
      return performance.now() - started;
    };
    const pass: number[] = [];
    const each: number[] = [];
    for (let round = 0; round < 7; round++) {
      pass.push(
        time(() => {
          for (const one of listed) fold(one.title);
        }),
      );
      each.push(
        time(() => {
          for (const query of ["i", "it", "ite", "item", "item 1", "resume batch"]) rank(query, listed, plan.working);
        }) / 6,
      );
    }
    expect(rank("item", listed, plan.working)).toHaveLength(MOST);
    // About three passes on a laptop, and under five with the whole suite running; a dozen, the most allowed, is there
    // about a frame.
    expect(Math.min(...each) / Math.min(...pass)).toBeLessThan(12);
  });

  it("offers a waiting move any item shown or the top level, and a link any item shown", () => {
    const move = targets(plan, "move");
    expect(move[0]?.run).toEqual({ kind: "target", id: null });
    expect(rank("garden", move, new Set()).map((one) => one.run)).toEqual([{ kind: "target", id: "garden" }]);
    // Projects in their own group, offered before anything is typed.
    expect(move.find((one) => one.title === "Garden")?.group).toBe("Projects");
    expect(rank("", move, new Set()).map((one) => one.title)).toEqual(["Top level", "Garden"]);
    expect(targets(plan, "link").some((one) => one.run.kind === "target" && one.run.id === null)).toBe(false);
    expect(rank("paint", targets(plan, "link"), new Set())).toEqual([]);
  });

  it("never offers what wouldn't move it: the item, anything inside it, or where it already is", () => {
    const ids = (pick: "move" | "link", from: string) =>
      targets(plan, pick, from).map((one) => (one.run.kind === "target" ? one.run.id : "?"));
    // Garden's own item is inside it, and it is at the top level already.
    expect(ids("move", "garden")).not.toContain(null);
    expect(ids("move", "garden")).not.toEqual(expect.arrayContaining(["garden", "dig"]));
    expect(ids("move", "garden")).toContain("tools");
    // Dig is in Garden: Garden isn't offered, the top level is.
    expect(ids("move", "dig")).toEqual(expect.arrayContaining([null, "tools"]));
    expect(ids("move", "dig")).not.toContain("garden");
    expect(ids("link", "dig")).not.toContain("dig");
    expect(ids("link", "dig")).toContain("garden");
  });
});

describe("the open item's actions", () => {
  it("come first while its panel shows, and not at all without one", () => {
    const shown = rank("", entries({ ...plan, item: { id: "dig", title: "Dig the beds" } }), plan.working);
    expect(shown.slice(0, 5).map((one) => one.run)).toEqual([
      { kind: "on-item", action: "edit" },
      { kind: "on-item", action: "move" },
      { kind: "on-item", action: "waits-for" },
      { kind: "on-item", action: "relates-to" },
      { kind: "on-item", action: "add-inside" },
    ]);
    expect(entries(plan).some((one) => one.run.kind === "on-item")).toBe(false);
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
    // Client hints first, where the browser has them; the deprecated platform otherwise.
    expect(platformOf({ platform: "Win32", userAgentData: { platform: "macOS" } })).toBe("macOS");
    expect([
      platformOf({ platform: "iPhone" }),
      platformOf({ platform: "MacIntel", userAgentData: { platform: "" } }),
    ]).toEqual(["iPhone", "MacIntel"]);
  });

  it("never takes a key an input method is composing with", () => {
    expect(opensMenu(press({ metaKey: true, isComposing: true }), true)).toBe(false);
    expect(opensMenu(press({ metaKey: true, keyCode: 229 }), true)).toBe(false);
  });
});

describe("the undo key", () => {
  const press = (fields: Partial<KeyboardEvent>) => ({
    key: "z",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    isComposing: false,
    keyCode: 90,
    ...fields,
  });

  it("is ⌘Z on a Mac and Ctrl+Z elsewhere, never with Shift, which redoes, nor while composing", () => {
    expect([undoes(press({ metaKey: true }), true), undoes(press({ ctrlKey: true }), false)]).toEqual([true, true]);
    expect(undoes(press({ metaKey: true, shiftKey: true }), true)).toBe(false);
    expect(undoes(press({ ctrlKey: true }), true)).toBe(false);
    expect(undoes(press({ metaKey: true, isComposing: true }), true)).toBe(false);
  });
});
