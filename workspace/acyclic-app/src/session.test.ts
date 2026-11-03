import { describe, expect, it } from "vitest";
import { addCommand } from "./edits";
import type { Intent } from "./nav";
import { createSession, type Notice, type Ports, UNDO_KEY, UNDOING } from "./session";

type Fault = "busy" | "archived" | "lost" | "lost-applied" | "500";

/** A command's name, as the server's tests name it: `needs`, `item list`, `item edit`. */
function nameOf(body: unknown): string {
  if (typeof body === "string") return body;
  const [group, verb] = Object.entries(body as object)[0] ?? ["", ""];
  return `${group} ${typeof verb === "string" ? verb : Object.keys(verb as object)[0]}`;
}

/**
 * A tracker behind the API: items by id with a title each, which `item add` and `item edit` change, and faults queued
 * by command name, each met once.
 */
function server(titles: Record<string, string> = { a: "A" }) {
  const plan = new Map(Object.entries(titles));
  const faults: Record<string, Fault[]> = {};
  const writes: unknown[] = [];
  const apply = (body: { item: { add?: { id: string; title: string }; edit?: { item: string; title?: string } } }) => {
    const { add, edit } = body.item;
    if (add) plan.set(add.id, add.title);
    if (edit?.title !== undefined) plan.set(edit.item, edit.title);
  };
  const answer = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
  const fetcher: Ports["fetcher"] = async (_url, init) => {
    const body = JSON.parse(String(init.body));
    const name = nameOf(body);
    const fault = faults[name]?.shift();
    if (name === "item add" || name === "item edit") writes.push(body);
    if (fault === "lost") throw new TypeError("Failed to fetch");
    if (fault === "lost-applied") {
      apply(body);
      throw new TypeError("Failed to fetch");
    }
    if (fault === "500") return new Response("oops", { status: 500 });
    if (fault === "busy" || fault === "archived")
      return new Response(JSON.stringify({ ok: false, error: { code: fault, message: fault, details: null } }));
    if (name === "item list") return answer({ items: [...plan.keys()].map((id) => ({ id })) });
    if (name === "item show")
      return answer(
        [...plan].map(([id, title]) => ({
          id,
          title,
          parent: null,
          thread: null,
          state: "backlog",
          notes: null,
          size: null,
          archived: false,
          links: { waits_for: [], waited_on_by: [] },
        })),
      );
    if (name === "thread list") return answer({ threads: [], waves: [], anytime: [] });
    if (name === "needs") return answer({ needs: [] });
    apply(body);
    return answer({});
  };
  const fail = (name: string, ...queued: Fault[]) => {
    faults[name] = [...(faults[name] ?? []), ...queued];
  };
  return { plan, fetcher, fail, writes };
}

/** A tab's storage, in memory, or one that throws at every call. */
function memory(throws = false) {
  const kept = new Map<string, string>();
  const or = <T>(run: () => T): T => {
    if (throws) throw new Error("blocked");
    return run();
  };
  return {
    kept,
    getItem: (key: string) => or(() => kept.get(key) ?? null),
    setItem: (key: string, value: string) => or(() => void kept.set(key, value)),
    removeItem: (key: string) => or(() => void kept.delete(key)),
  };
}

/** A session over `tracker`, with its moves of where the user is recorded. */
function open(tracker = server(), storage: Ports["storage"] = memory()) {
  const moves: Intent[] = [];
  const session = createSession({
    fetcher: tracker.fetcher,
    storage,
    nav: { go: (intent) => void moves.push(intent), item: () => null },
  });
  return { session, moves, tracker };
}

const rename = (id: string, title: string) => ({ item: { edit: { item: id, title } } });
const added = { id: "n", title: "New", parent: null, thread: null };

describe("a session's writes", () => {
  it("gives a refused add's form back with what was typed, drawing nothing", async () => {
    const { session, moves, tracker } = open();
    await session.refresh();
    tracker.fail("item add", "archived");
    expect(await session.send(addCommand(added), { select: "n", pin: null }, { kind: "add", item: added })).toBe(false);
    expect(moves.at(-1)).toMatchObject({ kind: "redraft", adding: { parent: null, draft: { title: "New" } } });
    expect(session.snapshot()).toMatchObject({ pending: null, busy: false, notice: { lasts: "refusal" } });
  });

  it("keeps an add the server saved though its answer was lost, the form closed", async () => {
    const { session, moves, tracker } = open();
    await session.refresh();
    tracker.fail("item add", "lost-applied");
    await session.send(addCommand(added), { select: "n", pin: null }, { kind: "add", item: added });
    expect(moves.some((move) => move.kind === "redraft")).toBe(false);
    expect(session.snapshot().load).toMatchObject({ items: [{ id: "a" }, { id: "n" }] });
  });

  it("says a write was saved when only the read after it failed, offering a refresh, never the write", async () => {
    const { session, moves, tracker } = open();
    await session.refresh();
    tracker.fail("item list", "500");
    expect(await session.send(rename("a", "B"), { select: "a", pin: null })).toBe(true);
    expect(session.snapshot().notice).toMatchObject({ offer: "refresh", lasts: "stale", retry: null });
    expect(session.snapshot().notice?.text).toMatch(/^Saved, but the refresh failed/);
    expect(moves).toEqual([{ kind: "wrote", select: null, pin: null }]);
  });

  it("offers a write the busy server turned away again, and sends it on Retry", async () => {
    const { session, tracker } = open();
    await session.refresh();
    tracker.fail("item edit", "busy");
    await session.send(rename("a", "B"), { select: "a", pin: null });
    expect(session.snapshot().notice?.offer).toBe("retry");
    expect(await session.retry()).toBe(true);
    expect(tracker.writes).toHaveLength(2);
    expect(tracker.plan.get("a")).toBe("B");
  });

  it("drops a write sent while another is out", async () => {
    const { session, tracker } = open();
    await session.refresh();
    const first = session.send(rename("a", "B"), { select: null, pin: null });
    expect(session.snapshot().busy).toBe(true);
    expect(await session.send(rename("a", "C"), { select: null, pin: null })).toBe(false);
    expect(await first).toBe(true);
    expect(tracker.writes).toEqual([rename("a", "B")]);
  });
});

describe("a session's reads", () => {
  it("fails the page on a first read that fails, and only says a later one's plan is stale", async () => {
    const tracker = server();
    tracker.fail("item list", "500");
    const { session } = open(tracker);
    await session.refresh();
    expect(session.snapshot().load.state).toBe("failed");
    await session.refresh();
    tracker.fail("item list", "500");
    await session.refresh();
    expect(session.snapshot().load.state).toBe("loaded");
    expect(session.snapshot().notice).toMatchObject({ offer: "refresh", lasts: "stale" });
  });

  it("hands out the same snapshot until something in it changes", async () => {
    const { session } = open();
    expect(session.snapshot()).toBe(session.snapshot());
    const was = session.snapshot();
    session.tell(null);
    expect(session.snapshot()).toBe(was);
    await session.refresh();
    expect(session.snapshot()).not.toBe(was);
  });
});

describe("a session's Undo", () => {
  /** A session with "a" renamed "B", its Undo offered. */
  async function renamed(storage: Ports["storage"] = memory()) {
    const opened = open(server(), storage);
    await opened.session.refresh();
    await opened.session.send(rename("a", "B"), { select: "a", pin: null, undo: true });
    return opened;
  }

  it("takes nothing back once the plan has changed since, sending no write", async () => {
    const { session, tracker } = await renamed();
    tracker.plan.set("a", "C");
    await session.undo();
    expect(session.snapshot().notice?.text).toBe("Changed since, so it can't be undone");
    expect(tracker.writes).toHaveLength(1);
  });

  it("keeps the Undo on offer when the plan can't be read to check it", async () => {
    const { session, tracker } = await renamed();
    tracker.fail("item list", "500");
    await session.undo();
    expect(session.snapshot().notice).toMatchObject({ offer: "undo" });
    expect(session.snapshot().notice?.text).toMatch(/^Couldn't check the plan/);
    await session.undo();
    expect(tracker.plan.get("a")).toBe("A");
  });

  it("is kept over a reload, offered again once a read says it still takes its write back", async () => {
    const storage = memory();
    await renamed(storage);
    const again = open(server({ a: "B" }), storage);
    expect(again.session.snapshot().notice).toMatchObject({ offer: "undo", restored: true });
    await again.session.refresh();
    expect(again.session.snapshot().notice).toMatchObject({ offer: "undo", text: "Edited B" });
    expect(again.session.snapshot().notice?.restored).toBeUndefined();
    // Changed since, it isn't offered at all.
    const stale = open(server({ a: "C" }), storage);
    await stale.session.refresh();
    expect(stale.session.snapshot().notice).toBeNull();
  });

  it("is dropped from the tab as anything else takes the slot, and passed on through Undoing…", async () => {
    const storage = memory();
    const { session } = await renamed(storage);
    session.tell(UNDOING);
    expect(storage.kept.has(UNDO_KEY)).toBe(true);
    const unsaved: Notice = { text: "Unsaved changes", offer: null, retry: null, lasts: "form", at: null };
    session.tell(unsaved);
    expect(storage.kept.has(UNDO_KEY)).toBe(false);
  });

  it("is offered as usual, though not kept, where the tab's storage throws", async () => {
    const { session } = await renamed(memory(true));
    expect(session.snapshot().notice).toMatchObject({ offer: "undo", text: "Edited B" });
    session.tell(null);
    expect(session.snapshot().notice).toBeNull();
  });
});
