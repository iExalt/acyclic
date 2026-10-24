import { describe, expect, it } from "vitest";
import { CommandError, command, items, settle } from "./api";

/** An item as `show` returns it, unconflicted. */
function raw(id: string) {
  return {
    id,
    title: id.toUpperCase(),
    parent: null,
    thread: null,
    state: "backlog",
    archived: false,
    links: { waits_for: [], relates_to: [] },
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
    expect(full.sent).toEqual(["list", { show: { items: ["a", "b"] } }]);

    const empty = server({ list: { ok: true, data: { items: [] } } });
    await expect(items(empty.fetcher)).resolves.toEqual([]);
    expect(empty.sent).toHaveLength(1);
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
