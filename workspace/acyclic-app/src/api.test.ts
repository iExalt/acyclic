import { describe, expect, it } from "vitest";
import { CommandError, command, items } from "./api";

/** A server that answers each command from a table, recording what it was sent. */
function server(answers: Record<string, unknown>) {
  const sent: object[] = [];
  const fetcher = async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as Record<string, unknown>;
    sent.push(body);
    const name = Object.keys(body)[0] ?? "";
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
    await expect(command({ list: {} }, fetcher)).resolves.toEqual([]);
    await expect(command({ show: { items: ["a"] } }, fetcher)).rejects.toEqual(
      new CommandError("not-found", "no item a"),
    );
  });

  it("names the status when the answer isn't an envelope", async () => {
    const fetcher = async () => new Response("unpaired", { status: 401 });
    await expect(command({ list: {} }, fetcher)).rejects.toMatchObject({ code: "http" });
  });
});

describe("items", () => {
  it("shows every listed id in one request, and sends none for an empty tracker", async () => {
    const shown = [{ id: "a" }, { id: "b" }];
    const full = server({ list: { ok: true, data: [{ id: "a" }, { id: "b" }] }, show: { ok: true, data: shown } });
    await expect(items(full.fetcher)).resolves.toEqual(shown);
    expect(full.sent).toEqual([{ list: {} }, { show: { items: ["a", "b"] } }]);

    const empty = server({ list: { ok: true, data: [] } });
    await expect(items(empty.fetcher)).resolves.toEqual([]);
    expect(empty.sent).toHaveLength(1);
  });
});
