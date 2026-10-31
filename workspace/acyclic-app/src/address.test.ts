import { describe, expect, it } from "vitest";
import { addressOf, type Found, landing, NOTHING, parse, type Wanted } from "./address";

describe("the address", () => {
  it("is bare at home with nothing open, and names what is open otherwise, read back as written", () => {
    expect(addressOf(NOTHING)).toBe("/");
    const open: Wanted[] = [
      { ...NOTHING, view: "threads", thread: "t-3" },
      { ...NOTHING, view: "map", item: "a b/c&d" },
      { ...NOTHING, needs: true, needsAt: "unapproved:t-3:ready" },
      { ...NOTHING, view: "map", needs: true },
    ];
    for (const wanted of open) expect(parse(addressOf(wanted).slice(1))).toEqual(wanted);
  });

  it("writes Needs you without an entry bare, as notifications do, and an item without the Map it implies", () => {
    expect(addressOf({ ...NOTHING, needs: true })).toBe("/?needs");
    expect(addressOf({ ...NOTHING, view: "map", item: "x", needs: true })).toBe("/?item=x&needs");
    expect(parse("?item=x").view).toBe("map");
  });

  it("keeps what else the address says, such as a test run's switch, and changes only what is open", () => {
    expect(addressOf({ ...NOTHING, view: "map", item: "y" }, "?engine=inline&item=x&view=threads")).toBe(
      "/?engine=inline&item=y",
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
    expect(landing(wanted, plan)).toEqual({ wanted, archived: false, missing: null });
    expect(landing({ ...wanted, item: "old" }, plan)).toMatchObject({ archived: true, missing: null });
  });

  it("drops an item or thread the plan no longer holds, and says which, keeping the rest", () => {
    expect(landing({ ...NOTHING, view: "map", item: "gone", needs: true }, plan)).toEqual({
      wanted: { ...NOTHING, view: "map", needs: true },
      archived: false,
      missing: "item",
    });
    expect(landing({ ...NOTHING, view: "threads", thread: "t-9" }, plan)).toMatchObject({
      wanted: { ...NOTHING, view: "threads" },
      missing: "thread",
    });
  });
});
