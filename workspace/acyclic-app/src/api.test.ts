import { describe, expect, it } from "vitest";
import { CommandError, command, countsText, failure, items, NO_COUNTS, settle, thread, threads } from "./api";
import { examples } from "./commands.examples.json";
import { ID } from "./map/fixture";

/** The command the server's tests run under this name. */
const example = (name: string) => examples.find((one) => one.name === name)?.command;

/** An item as `item show` returns it, unconflicted. */
function raw(id: string) {
  return {
    id,
    title: id.toUpperCase(),
    parent: null,
    thread: null,
    state: "backlog",
    notes: null,
    size: null,
    archived: false,
    links: { waits_for: [], waited_on_by: [] },
  };
}

/** A server that answers each command from a table by its name, such as `item show`, recording what it was sent. */
function server(answers: Record<string, unknown>) {
  const sent: unknown[] = [];
  const fetcher = async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as unknown;
    sent.push(body);
    const [group, verb] = typeof body === "string" ? [body, null] : (Object.entries(body as object)[0] ?? ["", null]);
    const name = verb === null ? group : `${group} ${typeof verb === "string" ? verb : Object.keys(verb as object)[0]}`;
    return new Response(JSON.stringify(answers[name]), { status: 200 });
  };
  return { fetcher, sent };
}

describe("command", () => {
  it("returns the envelope's data and throws its error code", async () => {
    const { fetcher } = server({
      "item list": { ok: true, data: [] },
      "item show": { ok: false, error: { code: "not-found", message: "no item a", details: null } },
    });
    await expect(command({ item: "list" }, fetcher)).resolves.toEqual([]);
    await expect(command({ item: { show: { items: ["a"] } } }, fetcher)).rejects.toEqual(
      new CommandError("not-found", "no item a"),
    );
  });

  it("names the status when the answer isn't an envelope", async () => {
    const fetcher = async () => new Response("forbidden", { status: 403 });
    await expect(command({ item: "list" }, fetcher)).rejects.toMatchObject({ code: "http" });
  });

  it("sends Undo expectations beside the command", async () => {
    let sent: unknown;
    const fetcher = async (_url: string, init: RequestInit) => {
      sent = JSON.parse(String(init.body));
      return new Response(JSON.stringify({ ok: true, data: {} }));
    };
    const body = { item: { edit: { item: ID.a, title: "A" } } };
    const conditions = [
      { kind: "item-field" as const, item: ID.a, field: "title" as const, value: "B" },
      { kind: "waits-for" as const, item: ID.a, target: ID.b, present: false },
      { kind: "started-at-most" as const, item: ID.a, count: 0 },
    ];
    await command(body, fetcher, conditions);
    expect(sent).toEqual({ command: body, expect: conditions });
  });
});

describe("items", () => {
  it("shows every listed id in one request, and sends none for an empty tracker", async () => {
    const shown = [raw(ID.a), raw(ID.b)];
    const full = server({
      "item list": { ok: true, data: { items: [{ id: ID.a }, { id: ID.b }] } },
      "item show": { ok: true, data: shown },
    });
    await expect(items(full.fetcher)).resolves.toEqual(shown.map(settle));
    // A command without arguments goes as its bare name inside its group, as the server parses it.
    expect(full.sent).toEqual([example("list"), example("show")]);

    const empty = server({ "item list": { ok: true, data: { items: [] } } });
    await expect(items(empty.fetcher)).resolves.toEqual([]);
    expect(empty.sent).toHaveLength(1);
  });

  it("keeps the stage core gives a parent from its leaves, beside its progress", async () => {
    const counts = { ...NO_COUNTS, leaves: 2, done: 1, in_review: 1, ready: 1 };
    const parent = { ...raw(ID.a), stage: "in-progress", progress: { state: "in-progress", counts } };
    const { fetcher } = server({
      "item list": { ok: true, data: { items: [{ id: ID.a }] } },
      "item show": { ok: true, data: [parent] },
    });
    await expect(items(fetcher)).resolves.toMatchObject([{ stage: "in-progress", progress: { counts } }]);
  });
});

describe("threads", () => {
  it("lists threads with their leaves by stage, titling one with a conflicted title by its first value", async () => {
    const { fetcher, sent } = server({
      "thread list": {
        ok: true,
        data: {
          threads: [
            {
              id: "t",
              title: "T",
              waits_for: [],
              progress: { counts: { ...NO_COUNTS, leaves: 3, done: 2, in_review: 1, ready: 1 } },
            },
            { id: "u", title: { conflict: [null, "V"] }, waits_for: ["t"] },
          ],
          waves: [["t"], ["u"]],
          anytime: [],
        },
      },
    });
    await expect(threads(fetcher)).resolves.toEqual({
      threads: [
        { id: "t", title: "T", waitsFor: [], counts: { ...NO_COUNTS, leaves: 3, done: 2, in_review: 1, ready: 1 } },
        // Counts from a server that sends none: no leaves.
        { id: "u", title: "V", waitsFor: ["t"], counts: NO_COUNTS },
      ],
      waves: [["t"], ["u"]],
      anytime: [],
    });
    expect(sent).toEqual([example("threads")]);
  });
});

describe("thread", () => {
  it("reads a thread for its panel, as the server's tests send it", async () => {
    const { fetcher, sent } = server({
      "thread show": {
        ok: true,
        data: [
          {
            id: ID.t,
            title: "T",
            brief: { conflict: ["x", "y"] },
            heads: [ID.a],
            members: [{ id: ID.a, title: "A" }],
          },
        ],
      },
    });
    await expect(thread(ID.t, fetcher)).resolves.toMatchObject({ title: "T", brief: null });
    expect(sent).toEqual([example("a thread")]);
  });
});

describe("settle", () => {
  it("draws a conflicted field as its first value and keeps every value for resolving", () => {
    const item = settle({
      ...raw("a"),
      title: { conflict: [null, "Sow", "Plant"] },
      parent: { conflict: ["beds", null] },
      thread: { conflict: [null] },
      state: { conflict: [null, null] },
    });
    expect(item).toMatchObject({ title: "Sow", parent: "beds", thread: null, state: "backlog" });
    expect(item.conflicts).toEqual({
      title: [null, "Sow", "Plant"],
      parent: ["beds", null],
      thread: [null],
      state: [null, null],
    });
    expect(settle(raw("b")).conflicts).toEqual({});
  });

  it("falls back to the id when every title in a conflict is a clear", () => {
    expect(settle({ ...raw("a"), title: { conflict: [null, null] } }).title).toBe("a");
  });
});

describe("failure", () => {
  it("tells a request that never reached the server in the app's words, and keeps any other failure's own", async () => {
    const lost = async () => {
      throw new TypeError("Load failed");
    };
    const told = await command({ item: "list" }, lost).catch(failure);
    expect(told).toEqual({ code: "network", message: "Can't reach the server" });
    // A bug in reading an answer isn't a lost connection.
    const malformed = async () => Response.json({ ok: true, data: null });
    const bug = await items(malformed).then(() => null, failure);
    expect(bug?.code).toBe("network");
    expect(bug?.message).not.toBe("Can't reach the server");
    expect(failure(new CommandError("busy", "try again"))).toEqual({ code: "busy", message: "Try again" });
  });

  it("words a refusal the server names by id for people, and starts any other as a sentence", () => {
    const cycle = new CommandError("parent-cycle", "water can't go under itself or its own descendant");
    expect(failure(cycle)).toEqual({
      code: "parent-cycle",
      message: "An item can't go inside itself, or inside anything under it",
    });
    expect(failure(new CommandError("claimed", "thread t is claimed by session/1")).message).toBe(
      "Thread t is claimed by session/1",
    );
  });
});

describe("countsText", () => {
  const none = { done: 0, leaves: 0, in_review: 0, in_progress: 0, ready: 0, blocked: 0, parked: 0, unthreaded: 0 };

  it("names every stage, done ones apart from those in review, and leaves out stages with none", () => {
    const all = { done: 3, leaves: 9, in_review: 1, in_progress: 1, ready: 1, blocked: 1, parked: 1, unthreaded: 1 };
    expect(countsText(all)).toBe("2 done, 1 in review, 1 in progress, 1 ready, 1 blocked, 1 parked, 1 needs a thread");
    expect(countsText({ ...none, leaves: 2, blocked: 2 })).toBe("2 blocked");
  });
});
