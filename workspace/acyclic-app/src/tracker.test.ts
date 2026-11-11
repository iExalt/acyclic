import { describe, expect, it, vi } from "vitest";
import { addCommand } from "./edits";
import type { Intent } from "./nav";
import { createTracker, type Notice, PLAN_KEY, type Ports, UNDO_KEY, UNDOING } from "./tracker";

type Fault =
  | "redundant-dependency"
  | "busy"
  | "self-link"
  | "changed-since"
  | "lost"
  | "lost-applied"
  | "500"
  | "wrong-identity";

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
  const waits = new Map<string, string[]>();
  const faults: Record<string, Fault[]> = {};
  const writes: unknown[] = [];
  const requests: unknown[] = [];
  const needsRows: { kind: string; identity?: string }[] = [];
  const apply = (body: {
    item: {
      add?: { id: string; title: string };
      edit?: { item: string; title?: string; "waits-for"?: string[]; "no-waits-for"?: string[] };
    };
  }) => {
    const { add, edit } = body.item;
    if (add) plan.set(add.id, add.title);
    if (edit?.title !== undefined) plan.set(edit.item, edit.title);
    if (edit?.["waits-for"])
      waits.set(edit.item, [...new Set([...(waits.get(edit.item) ?? []), ...edit["waits-for"]])]);
    if (edit?.["no-waits-for"])
      waits.set(
        edit.item,
        (waits.get(edit.item) ?? []).filter((id) => !edit["no-waits-for"]?.includes(id)),
      );
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
    if (
      fault === "redundant-dependency" ||
      fault === "busy" ||
      fault === "self-link" ||
      fault === "changed-since" ||
      fault === "wrong-identity"
    )
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
          state: "open",
          notes: null,
          initiative: null,
          links: { waits_for: waits.get(id) ?? [], waited_on_by: [] },
        })),
      );
    if (name === "initiative list")
      return answer({
        initiatives: [{ id: "init", title: "Init", counts: { items: 0 } }],
        no_initiative: { items: [...plan.keys()] },
      });
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
function open(backend = server(), storage: Ports["storage"] = memory(), planStorage: Ports["planStorage"] = memory()) {
  const moves: Intent[] = [];
  const tracker = createTracker({
    fetcher: backend.fetcher,
    storage,
    planStorage,
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
    backend.fail("item add", "self-link");
    expect(await tracker.send(addCommand(added), { select: "n", pin: null }, { kind: "add", item: added })).toBe(false);
    expect(moves.at(-1)).toMatchObject({ kind: "redraft", adding: { parent: null, draft: { title: "New" } } });
    expect(tracker.snapshot()).toMatchObject({ pending: null, busy: false, notice: { lasts: "refusal" } });
  });

  it("reads the initiatives with the plan, and the ids of the items in none", async () => {
    const { tracker } = open();
    await tracker.refresh();
    expect(tracker.snapshot().load).toMatchObject({
      initiatives: [{ id: "init", title: "Init", items: 0 }],
      noInitiative: ["a"],
    });
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
    backend.needsRows.push({ kind: "done-waiting", identity: "opaque-core-entry" });
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

describe("redundant dependencies", () => {
  it.each([false, true])("requires the confirmation answer %s before retrying", async (accept) => {
    const confirm = vi.fn(() => accept);
    vi.stubGlobal("window", { confirm });
    try {
      const { tracker, backend } = open();
      await tracker.refresh();
      backend.fail("item edit", "redundant-dependency");
      const body = { item: { edit: { item: "a", "waits-for": ["b"] } } };
      expect(await tracker.send(body, { select: "a", pin: null })).toBe(accept);
      expect(confirm).toHaveBeenCalledOnce();
      expect(backend.writes).toHaveLength(accept ? 2 : 1);
      if (accept) expect(backend.writes[1]).toMatchObject({ item: { edit: { "allow-redundant": true } } });
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

it.each([false, true])("keeps confirmed link Undo across reload, busy retry: %s", async (busy) => {
  vi.stubGlobal("window", { confirm: () => true });
  try {
    const backend = server({ a: "A", b: "B" });
    const storage = memory();
    const { tracker } = open(backend, storage);
    await tracker.refresh();
    backend.fail("item edit", "redundant-dependency");
    if (busy) backend.fail("item edit", "busy");
    const saved = await tracker.send(
      { item: { edit: { item: "a", "waits-for": ["b"] } } },
      { select: "a", pin: null, undo: true },
    );
    expect(saved).toBe(!busy);
    if (busy) expect(await tracker.retry()).toBe(true);
    expect(tracker.snapshot().notice).toMatchObject({ offer: "undo" });
    const again = open(backend, storage);
    expect(again.tracker.snapshot().notice).toMatchObject({ offer: "undo", restored: true });
    await again.tracker.refresh();
    expect(again.tracker.snapshot().notice).toMatchObject({ offer: "undo" });
    await again.tracker.undo();
    expect(backend.writes.at(-1)).toMatchObject({ item: { edit: { item: "a", "no-waits-for": ["b"] } } });
  } finally {
    vi.unstubAllGlobals();
  }
});

describe("a tracker's kept plan", () => {
  it("keeps the plan once a read is shown, and shows it at the next start as stale, read-only, until a read lands", async () => {
    const plans = memory();
    const first = open(server({ a: "A" }), memory(), plans);
    expect(first.tracker.snapshot()).toMatchObject({ load: { state: "loading" }, stale: false });
    await first.tracker.refresh();
    expect(plans.kept.has(PLAN_KEY)).toBe(true);
    expect(JSON.parse(plans.kept.get(PLAN_KEY) ?? "")).not.toHaveProperty("unread");

    const backend = server({ a: "B" });
    const again = open(backend, memory(), plans);
    const { load, stale, reads } = again.tracker.snapshot();
    expect(stale).toBe(true);
    expect(reads).toBe(0);
    expect(load).toMatchObject({ state: "loaded", items: [{ id: "a", title: "A" }] });
    expect(await again.tracker.send(rename("a", "Z"), { select: null, pin: null })).toBe(false);
    expect(backend.writes).toEqual([]);

    await again.tracker.refresh();
    expect(again.tracker.snapshot()).toMatchObject({ stale: false, reads: 1, load: { items: [{ title: "B" }] } });
    expect(JSON.parse(plans.kept.get(PLAN_KEY) ?? "").items[0].title).toBe("B");
  });

  it("stays shown and read-only, offering a refresh, when the first read fails", async () => {
    const plans = memory();
    await open(server(), memory(), plans).tracker.refresh();
    const backend = server();
    const again = open(backend, memory(), plans);
    backend.fail("item list", "500");
    await again.tracker.refresh();
    expect(again.tracker.snapshot()).toMatchObject({
      load: { state: "loaded" },
      stale: true,
      notice: { offer: "refresh" },
    });
    await again.tracker.refresh();
    expect(again.tracker.snapshot()).toMatchObject({ stale: false, reads: 2 });
  });

  it("is dropped, and not shown, when the server says it isn't serving this user", async () => {
    const plans = memory();
    await open(server(), memory(), plans).tracker.refresh();
    const backend = server();
    const again = open(backend, memory(), plans);
    backend.fail("item list", "wrong-identity");
    await again.tracker.refresh();
    expect(again.tracker.snapshot()).toMatchObject({ load: { state: "failed", code: "wrong-identity" }, stale: false });
    expect(plans.kept.has(PLAN_KEY)).toBe(false);
  });

  it.each([
    ["unparseable", "{"],
    ["of another shape", JSON.stringify({ items: "no" })],
    ["empty", "null"],
  ])("is ignored when %s, and when storage throws", async (_, raw) => {
    const plans = memory();
    plans.kept.set(PLAN_KEY, raw);
    expect(open(server(), memory(), plans).tracker.snapshot()).toMatchObject({
      load: { state: "loading" },
      stale: false,
    });
    expect(open(server(), memory(), memory(true)).tracker.snapshot().load.state).toBe("loading");
  });
});
