import { describe, expect, it } from "vitest";
import {
  CommandError,
  command,
  countsText,
  failure,
  globalLevel,
  items,
  proposals,
  settle,
  thread,
  threads,
} from "./api";
import { examples } from "./commands.examples.json";

/** The command the server's tests run under this name. */
const example = (name: string) => examples.find((one) => one.name === name)?.command;

/** An item as `show` returns it, unconflicted. */
function raw(id: string) {
  return {
    id,
    title: id.toUpperCase(),
    parent: null,
    thread: null,
    state: "backlog",
    notes: null,
    gate: null,
    size: null,
    archived: false,
    links: { waits_for: [], waited_on_by: [], relates_to: [], related_from: [] },
  };
}

/** A server that answers each command from a table, recording what it was sent. */
function server(answers: Record<string, unknown>) {
  const sent: unknown[] = [];
  const fetcher = async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as unknown;
    sent.push(body);
    const name = typeof body === "string" ? body : (Object.keys(body as object)[0] ?? "");
    return new Response(JSON.stringify(answers[name]), { status: 200 });
  };
  return { fetcher, sent };
}

describe("command", () => {
  it("returns the envelope's data and throws its error code", async () => {
    const { fetcher } = server({
      list: { ok: true, data: [] },
      show: { ok: false, error: { code: "not-found", message: "no item a", details: null } },
    });
    await expect(command("list", fetcher)).resolves.toEqual([]);
    await expect(command({ show: { items: ["a"] } }, fetcher)).rejects.toEqual(
      new CommandError("not-found", "no item a"),
    );
  });

  it("names the status when the answer isn't an envelope", async () => {
    const fetcher = async () => new Response("unpaired", { status: 401 });
    await expect(command("list", fetcher)).rejects.toMatchObject({ code: "http" });
  });
});

describe("items", () => {
  it("shows every listed id in one request, and sends none for an empty tracker", async () => {
    const shown = [raw("a"), raw("b")];
    const full = server({
      list: { ok: true, data: { items: [{ id: "a" }, { id: "b" }] } },
      show: { ok: true, data: shown },
    });
    await expect(items(full.fetcher)).resolves.toEqual(shown.map(settle));
    // A command without arguments goes as its bare name, as the server parses it.
    expect(full.sent).toEqual([example("list"), example("show")]);

    const empty = server({ list: { ok: true, data: { items: [] } } });
    await expect(items(empty.fetcher)).resolves.toEqual([]);
    expect(empty.sent).toHaveLength(1);
  });
});

describe("threads", () => {
  it("lists threads, titling one with a conflicted title by its first value", async () => {
    const { fetcher, sent } = server({
      thread: {
        ok: true,
        data: {
          threads: [
            { id: "t", title: "T", waits_for: [] },
            { id: "u", title: { conflict: [null, "V"] }, waits_for: ["t"] },
          ],
          waves: [["t"], ["u"]],
          anytime: [],
        },
      },
    });
    await expect(threads(fetcher)).resolves.toEqual({
      threads: [
        { id: "t", title: "T", waitsFor: [] },
        { id: "u", title: "V", waitsFor: ["t"] },
      ],
      waves: [["t"], ["u"]],
      anytime: [],
    });
    expect(sent).toEqual([example("threads")]);
  });
});

describe("thread, proposals and globalLevel", () => {
  it("read a thread for its panel, the proposals and the global level, as the server's tests send them", async () => {
    const { fetcher, sent } = server({
      thread: {
        ok: true,
        data: [
          {
            id: "t",
            title: "T",
            brief: { conflict: ["x", "y"] },
            approved: false,
            level: null,
            heads: ["a"],
            members: [{ id: "a", title: "A" }],
          },
        ],
      },
      proposal: { ok: true, data: { proposals: [] } },
      level: { ok: true, data: { global: "blockers", threads: [] } },
    });
    await expect(thread("t", fetcher)).resolves.toMatchObject({ title: "T", brief: null, level: null });
    await expect(proposals(fetcher)).resolves.toEqual([]);
    await expect(globalLevel(fetcher)).resolves.toBe("blockers");
    expect(sent).toEqual([example("a thread"), example("proposals"), example("levels")]);
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
    const told = await command("list", lost).catch(failure);
    expect(told).toEqual({ code: "network", message: "Can't reach the server" });
    // A bug in reading an answer isn't a lost connection.
    const malformed = async () => Response.json({ ok: true, data: null });
    const bug = await items(malformed).then(() => null, failure);
    expect(bug?.code).toBe("network");
    expect(bug?.message).not.toBe("Can't reach the server");
    expect(failure(new CommandError("busy", "try again"))).toEqual({ code: "busy", message: "Try again" });
  });

  it("tells a browser the server no longer knows to pair again, on the server's own port", () => {
    const refused = new CommandError("no-session", "the request carries no session: present the one acyclic up issued");
    expect(failure(refused, "7479").message).toBe(
      "This browser isn't paired with acyclic up any more. Run acyclic up --link --port 7479 where it runs, and open the link it prints within five minutes",
    );
    expect(failure(refused, "").message).toContain("Run acyclic up --link where it runs");
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
