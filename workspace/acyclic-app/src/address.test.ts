import { describe, expect, it } from "vitest";
import { addressOf, type Found, landing, NOTHING, parse, type Wanted } from "./address";

describe("the address", () => {
  it("is bare at home with nothing open, and names what is open otherwise, read back as written", () => {
    expect(addressOf(NOTHING)).toBe("/");
    const open: Wanted[] = [
      { ...NOTHING, view: "threads", thread: "t-3" },
      { ...NOTHING, view: "map", item: "a b/c&d" },
      { ...NOTHING, view: "needs" },
      { ...NOTHING, view: "needs", needsAt: "unapproved:t-3:ready" },
      // An item left open over another view, as going there with it open leaves it.
      { ...NOTHING, view: "threads", item: "x" },
      { ...NOTHING, view: "overview", item: "x" },
    ];
    for (const wanted of open) expect(parse(addressOf(wanted).slice(1))).toEqual(wanted);
  });

  it("writes Needs you as a view, the entry it opens at beside it, and an item without the Map it implies", () => {
    expect(addressOf({ ...NOTHING, view: "needs" })).toBe("/?view=needs");
    expect(addressOf({ ...NOTHING, view: "needs", needsAt: "a:b" })).toBe("/?view=needs&needs=a%3Ab");
    // An entry is Needs you's alone: on another view, the address leaves it out.
    expect(addressOf({ ...NOTHING, view: "map", needsAt: "a:b" })).toBe("/?view=map");
    expect(parse("?item=x").view).toBe("map");
  });

  it("opens Needs you's view at an old address, as notifications and bookmarks still send it", () => {
    expect(parse("?needs")).toEqual({ ...NOTHING, view: "needs" });
    expect(parse("?needs=a:b")).toEqual({ ...NOTHING, view: "needs", needsAt: "a:b" });
    // Once opened over an item, the list is now the view, the item left open over it.
    expect(parse("?item=x&needs")).toEqual({ ...NOTHING, view: "needs", item: "x" });
    // A view named wins, and an entry on any other view is dropped.
    expect(parse("?view=threads&needs=a")).toEqual({ ...NOTHING, view: "threads" });
  });

  it("keeps what else the address says, such as a test run's switch, and changes only what is open", () => {
    expect(addressOf({ ...NOTHING, view: "map", item: "y" }, "?engine=inline&item=x&view=threads")).toBe(
      "/?engine=inline&item=y",
    );
    expect(addressOf({ ...NOTHING, view: "threads", item: "y" }, "?engine=inline")).toBe(
      "/?engine=inline&view=threads&item=y",
    );
    expect(addressOf(NOTHING, "?needs&engine=inline")).toBe("/?engine=inline");
  });

  it("opens at home what it doesn't know or finds empty", () => {
    expect(parse("?view=elsewhere&item=&thread=&utm=1")).toEqual(NOTHING);
    expect(parse("")).toEqual(NOTHING);
  });
});

describe("landing", () => {
  const plan = {
    item: (id: string): Found => ({ here: "shown" as const, old: "archived" as const })[id] ?? null,
    thread: (id: string) => id === "t-1",
  };

  it("opens what the plan holds as named, and an archived item with archived items shown", () => {
    const wanted = { ...NOTHING, view: "map" as const, item: "here", thread: "t-1" };
    expect(landing(wanted, plan)).toEqual({ wanted, archived: false, missing: [] });
    expect(landing({ ...wanted, item: "old" }, plan)).toMatchObject({ archived: true, missing: [] });
  });

  it("drops an item or thread the plan no longer holds, and says which, keeping the rest", () => {
    expect(landing({ ...NOTHING, view: "needs", item: "gone" }, plan)).toEqual({
      wanted: { ...NOTHING, view: "needs" },
      archived: false,
      missing: ["item"],
    });
    expect(landing({ ...NOTHING, view: "threads", thread: "t-9" }, plan)).toMatchObject({
      wanted: { ...NOTHING, view: "threads" },
      missing: ["thread"],
    });
    expect(landing({ ...NOTHING, view: "map", item: "gone", thread: "t-9" }, plan).missing).toEqual(["item", "thread"]);
  });
});
