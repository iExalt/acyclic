import { describe, expect, it } from "vitest";
import { addCommand } from "./edits";
import type { Intent } from "./nav";
import { createTracker, type Notice, type Ports, UNDO_KEY, UNDOING } from "./tracker";

type Fault = "busy" | "archived" | "changed-since" | "lost" | "lost-applied" | "500";

/** A command's name, as the server's tests name it: `needs`, `item list`, `item edit`. */
function nameOf(body: unknown): string {
  if (typeof body === "string") return body;
  const [group, verb] = Object.entries(body as object)[0] ?? ["", ""];
  return `${group} ${typeof verb === "string" ? verb : Object.keys(verb as object)[0]}`;
}

/**
 * A backend behind the API: items by id with a title each, which `item add` and `item edit` change, and faults queued
 * by command name, each met once.
 */
function server(titles: Record<string, string> = { a: "A" }) {
  const plan = new Map(Object.entries(titles));
  const faults: Record<string, Fault[]> = {};
  const writes: unknown[] = [];
  const requests: unknown[] = [];
  const needsRows: { kind: string; identity?: string }[] = [];
  const apply = (body: { item: { add?: { id: string; title: string }; edit?: { item: string; title?: string } } }) => {
    const { add, edit } = body.item;
    if (add) plan.set(add.id, add.title);
    if (edit?.title !== undefined) plan.set(edit.item, edit.title);
  };
  const answer = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
  const fetcher: Ports["fetcher"] = async (_url, init) => {
    const request = JSON.parse(String(init.body));
    requests.push(request);
    const body = typeof request === "object" && request !== null && "command" in request ? request.command : request;
    const name = nameOf(body);
    const fault = faults[name]?.shift();
    if (name === "item add" || name === "item edit") writes.push(body);
    if (fault === "lost") throw new TypeError("Failed to fetch");
    if (fault === "lost-applied") {
      apply(body);
      throw new TypeError("Failed to fetch");
    }
    if (fault === "500") return new Response("oops", { status: 500 });
    if (fault === "busy" || fault === "archived" || fault === "changed-since")
      return new Response(JSON.stringify({ ok: false, error: { code: fault, message: fault, details: null } }));
    const conditions = (request as { expect?: { kind: string; item?: string; field?: string; value?: unknown }[] })
      .expect;
    if (
      conditions?.some(
        (condition) =>
          condition.kind === "item-field" &&
          condition.field === "title" &&
          condition.item !== undefined &&
          plan.get(condition.item) !== condition.value,
      )
    )
      return new Response(
        JSON.stringify({ ok: false, error: { code: "changed-since", message: "changed since", details: null } }),
      );
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
    if (name === "needs") return answer({ needs: needsRows });
    apply(body);
    return answer({});
  };
  const fail = (name: string, ...queued: Fault[]) => {
    faults[name] = [...(faults[name] ?? []), ...queued];
  };
  return { plan, fetcher, fail, requests, writes, needsRows };
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

/** A tracker over `backend`, with its moves of where the user is recorded. */
function open(backend = server(), storage: Ports["storage"] = memory()) {
  const moves: Intent[] = [];
  const tracker = createTracker({
    fetcher: backend.fetcher,
    storage,
    nav: { go: (intent) => void moves.push(intent), item: () => null },
  });
  return { tracker, moves, backend };
}

const rename = (id: string, title: string) => ({ item: { edit: { item: id, title } } });
const added = { id: "n", title: "New", parent: null, thread: null };

describe("a tracker's writes", () => {
  it("gives a refused add's form back with what was typed, drawing nothing", async () => {
    const { tracker, moves, backend } = open();
    await tracker.refresh();
    backend.fail("item add", "archived");
    expect(await tracker.send(addCommand(added), { select: "n", pin: null }, { kind: "add", item: added })).toBe(false);
    expect(moves.at(-1)).toMatchObject({ kind: "redraft", adding: { parent: null, draft: { title: "New" } } });
    expect(tracker.snapshot()).toMatchObject({ pending: null, busy: false, notice: { lasts: "refusal" } });
  });

  it("keeps an add the server saved though its answer was lost, the form closed", async () => {
    const { tracker, moves, backend } = open();
    await tracker.refresh();
    backend.fail("item add", "lost-applied");
    await tracker.send(addCommand(added), { select: "n", pin: null }, { kind: "add", item: added });
    expect(moves.some((move) => move.kind === "redraft")).toBe(false);
    expect(tracker.snapshot().load).toMatchObject({ items: [{ id: "a" }, { id: "n" }] });
  });

  it("says a write was saved when only the read after it failed, offering a refresh, never the write", async () => {
    const { tracker, moves, backend } = open();
    await tracker.refresh();
    backend.fail("item list", "500");
    expect(await tracker.send(rename("a", "B"), { select: "a", pin: null })).toBe(true);
    expect(tracker.snapshot().notice).toMatchObject({ offer: "refresh", lasts: "stale", retry: null });
    expect(tracker.snapshot().notice?.text).toMatch(/^Saved, but the refresh failed/);
    expect(moves).toEqual([{ kind: "wrote", select: null, pin: null }]);
  });

  it("offers a write the busy server turned away again, and sends it on Retry", async () => {
    const { tracker, backend } = open();
    await tracker.refresh();
    backend.fail("item edit", "busy");
    await tracker.send(rename("a", "B"), { select: "a", pin: null });
    expect(tracker.snapshot().notice?.offer).toBe("retry");
    expect(await tracker.retry()).toBe(true);
    expect(backend.writes).toHaveLength(2);
    expect(backend.plan.get("a")).toBe("B");
  });

  it("sends only the observed opaque need identities and keeps them on busy retry", async () => {
    const { tracker, backend } = open();
    backend.needsRows.push({ kind: "reopened", identity: "opaque-core-entry" });
    await tracker.refresh();
    backend.fail("item edit", "busy");
    await tracker.send(rename("a", "B"), { select: "a", pin: null });
    const writes = () =>
      backend.requests.filter((request) => typeof request === "object" && request !== null && "command" in request);
    expect(writes().at(-1)).toEqual({
      command: rename("a", "B"),
      observed: ["opaque-core-entry"],
    });
    await tracker.retry();
    expect(writes().at(-1)).toEqual({
      command: rename("a", "B"),
      observed: ["opaque-core-entry"],
    });
  });

  it("drops a write sent while another is out", async () => {
    const { tracker, backend } = open();
    await tracker.refresh();
    const first = tracker.send(rename("a", "B"), { select: null, pin: null });
    expect(tracker.snapshot().busy).toBe(true);
    expect(await tracker.send(rename("a", "C"), { select: null, pin: null })).toBe(false);
    expect(await first).toBe(true);
    expect(backend.writes).toEqual([rename("a", "B")]);
  });
});

describe("a tracker's reads", () => {
  it("fails the page on a first read that fails, and only says a later one's plan is stale", async () => {
    const backend = server();
    backend.fail("item list", "500");
    const { tracker } = open(backend);
    await tracker.refresh();
    expect(tracker.snapshot().load.state).toBe("failed");
    await tracker.refresh();
    backend.fail("item list", "500");
    await tracker.refresh();
    expect(tracker.snapshot().load.state).toBe("loaded");
    expect(tracker.snapshot().notice).toMatchObject({ offer: "refresh", lasts: "stale" });
  });

  it("hands out the same snapshot until something in it changes", async () => {
    const { tracker } = open();
    expect(tracker.snapshot()).toBe(tracker.snapshot());
    const was = tracker.snapshot();
    tracker.tell(null);
    expect(tracker.snapshot()).toBe(was);
    await tracker.refresh();
    expect(tracker.snapshot()).not.toBe(was);
  });
});

describe("a tracker's Undo", () => {
  /** A tracker with "a" renamed "B", its Undo offered. */
  async function renamed(storage: Ports["storage"] = memory()) {
    const opened = open(server(), storage);
    await opened.tracker.refresh();
    await opened.tracker.send(rename("a", "B"), { select: "a", pin: null, undo: true });
    return opened;
  }

  it("takes nothing back once the plan has changed since, sending no write", async () => {
    const { tracker, backend } = await renamed();
    backend.plan.set("a", "C");
    await tracker.undo();
    expect(tracker.snapshot().notice?.text).toBe("Changed since, so it can't be undone");
    expect(backend.writes).toHaveLength(1);
  });

  it("sends the checked scalar value and keeps a server changed-since refusal in the notice", async () => {
    const { tracker, backend } = await renamed();
    backend.fail("item edit", "changed-since");
    await tracker.undo();
    expect(backend.requests.at(-1)).toEqual({
      command: rename("a", "A"),
      expect: [{ kind: "item-field", item: "a", field: "title", value: "B" }],
      observed: [],
    });
    expect(backend.plan.get("a")).toBe("B");
    expect(tracker.snapshot().notice).toMatchObject({
      text: "Changed since, so it can't be undone",
      offer: null,
      lasts: "refusal",
    });
  });

  it("keeps Undo expectations when retrying a busy write", async () => {
    const { tracker, backend } = await renamed();
    backend.fail("item edit", "busy");
    await tracker.undo();
    expect(tracker.snapshot().notice?.offer).toBe("retry");
    backend.plan.set("a", "C");
    await tracker.retry();
    expect(backend.requests.at(-1)).toEqual({
      command: rename("a", "A"),
      expect: [{ kind: "item-field", item: "a", field: "title", value: "B" }],
      observed: [],
    });
    expect(backend.plan.get("a")).toBe("C");
    expect(tracker.snapshot().notice).toMatchObject({
      text: "Changed since, so it can't be undone",
      offer: null,
      lasts: "refusal",
    });
  });

  it("keeps the Undo on offer when the plan can't be read to check it", async () => {
    const { tracker, backend } = await renamed();
    backend.fail("item list", "500");
    await tracker.undo();
    expect(tracker.snapshot().notice).toMatchObject({ offer: "undo" });
    expect(tracker.snapshot().notice?.text).toMatch(/^Couldn't check the plan/);
    await tracker.undo();
    expect(backend.plan.get("a")).toBe("A");
  });

  it("stays kept over a reload while it is still on offer after a check that failed", async () => {
    const storage = memory();
    const { tracker, backend } = await renamed(storage);
    const stored = storage.kept.get(UNDO_KEY);
    backend.fail("item list", "500");
    await tracker.undo();
    expect(tracker.snapshot().notice?.offer).toBe("undo");
    expect(storage.kept.get(UNDO_KEY)).toBe(stored);
    const again = open(server({ a: "B" }), storage);
    expect(again.tracker.snapshot().notice).toMatchObject({ offer: "undo", restored: true });
  });

  it("is kept over a reload, offered again once a read says it still takes its write back", async () => {
    const storage = memory();
    await renamed(storage);
    const again = open(server({ a: "B" }), storage);
    expect(again.tracker.snapshot().notice).toMatchObject({ offer: "undo", restored: true });
    await again.tracker.refresh();
    expect(again.tracker.snapshot().notice).toMatchObject({ offer: "undo", text: "Edited B" });
    expect(again.tracker.snapshot().notice?.restored).toBeUndefined();
    // Changed since, it isn't offered at all.
    const stale = open(server({ a: "C" }), storage);
    await stale.tracker.refresh();
    expect(stale.tracker.snapshot().notice).toBeNull();
  });

  it("is dropped from the tab as anything else takes the slot, and passed on through Undoing…", async () => {
    const storage = memory();
    const { tracker } = await renamed(storage);
    tracker.tell(UNDOING);
    expect(storage.kept.has(UNDO_KEY)).toBe(true);
    const unsaved: Notice = { text: "Unsaved changes", offer: null, retry: null, lasts: "form", at: null };
    tracker.tell(unsaved);
    expect(storage.kept.has(UNDO_KEY)).toBe(false);
  });

  it("is offered as usual, though not kept, where the tab's storage throws", async () => {
    const { tracker } = await renamed(memory(true));
    expect(tracker.snapshot().notice).toMatchObject({ offer: "undo", text: "Edited B" });
    tracker.tell(null);
    expect(tracker.snapshot().notice).toBeNull();
  });
});
