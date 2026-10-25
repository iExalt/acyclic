import { describe, expect, it } from "vitest";
import { item } from "./map/fixture";
import { answered, drawn, NO_READS, type Reads, show, start } from "./reads";

const pond = { kind: "add" as const, item: { item: "pond", title: "Pond", parent: null, thread: null } };
const garden = [item("garden")];

/** Shows read `n` with `items`, failing if it's overtaken. */
function shown(reads: Reads, n: number, items = garden): Reads {
  const next = show(reads, n, items);
  if (next === null) throw new Error(`read ${n} overtaken`);
  return next;
}

describe("the plan's reads", () => {
  it("never shows a read overtaken by one started later and shown", () => {
    const two = start(start(NO_READS));
    expect(show(shown(two, 2), 1, [])).toBeNull();
    expect(show(shown(two, 1), 2, [])).not.toBeNull();
  });

  it("keeps a saved write drawn over a read started before it answered, as an older refresh is", () => {
    // A refresh starts; an add is answered; its own read starts and fails; the refresh then comes back.
    const refreshing = start(NO_READS);
    const saved = start(answered(refreshing, pond));
    const after = shown(saved, 1);
    expect(after.unread).toHaveLength(1);
    expect(drawn(after.items, after.unread, null).map((one) => one.id)).toEqual(["garden", "pond"]);
  });

  it("drops a saved write once a read started after it answered is shown", () => {
    const saved = answered(start(NO_READS), pond);
    const later = shown(start(saved), 2, [...garden, item("pond")]);
    expect(later.unread).toEqual([]);
    expect(drawn(later.items, later.unread, null).map((one) => one.id)).toEqual(["garden", "pond"]);
  });

  it("keeps a saved edit's fields over an older read, under the write still out", () => {
    const beds = item("beds", null, [], { notes: "old" });
    const base = { title: "Beds", state: "backlog", thread: "", notes: "old", gate: "", size: "" };
    const draft = { ...base, notes: "new" };
    const saved = answered(start(NO_READS), { kind: "edit", id: "beds", edited: { base, draft } });
    const older = shown(saved, 1, [beds]);
    expect(drawn(older.items, older.unread, null)[0]?.notes).toBe("new");
    const out = { kind: "edit" as const, id: "beds", edited: { base: draft, draft: { ...draft, notes: "newer" } } };
    expect(drawn(older.items, older.unread, out)[0]?.notes).toBe("newer");
  });
});
