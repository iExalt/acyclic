// The service worker, run from its own file in a stand-in worker scope, so its handlers are tested as shipped.
/// <reference types="node" />

import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const ORIGIN = "https://acyclic.example";
const source = readFileSync(new URL("../public/sw.js", import.meta.url), "utf8");

/**
 * Runs the worker with the windows `open`, each answering a message or not, and gives its handlers and what they did. A
 * wait for an answer ends at once, so a window that doesn't answer costs the tests nothing.
 */
function worker(open: { answers: boolean; focused?: boolean; visibilityState?: string }[]) {
  const handlers = new Map<string, (event: unknown) => void>();
  const did: unknown[][] = [];
  const windows = open.map((one, n) => ({
    url: `${ORIGIN}/`,
    focused: one.focused ?? false,
    visibilityState: one.visibilityState ?? "hidden",
    focus: async () => did.push(["focus", n]),
    navigate: async (url: string) => did.push(["navigate", url]),
    postMessage: (message: unknown, [port]: MessagePort[]) => {
      did.push(["message", n, message]);
      if (one.answers) port?.postMessage("opened");
    },
  }));
  const self = {
    location: { origin: ORIGIN },
    addEventListener: (type: string, handler: (event: unknown) => void) => handlers.set(type, handler),
    skipWaiting: () => undefined,
    registration: {
      showNotification: async (title: string, options: unknown) => did.push(["show", title, options]),
    },
    clients: {
      claim: async () => undefined,
      matchAll: async () => windows,
      openWindow: async (url: string) => did.push(["open", url]),
    },
  };
  // No answer within the wait: a timer that fires once the answer, if any, has come.
  const setTimeout = (fire: () => void) => globalThis.setTimeout(fire, 5);
  runInNewContext(source, { self, URL, MessageChannel, setTimeout, clearTimeout });
  /** Dispatches an event to the worker and waits for what it waits on. */
  const dispatch = async (type: string, fields: object) => {
    const waits: Promise<unknown>[] = [];
    handlers.get(type)?.({ ...fields, waitUntil: (promise: Promise<unknown>) => waits.push(promise) });
    await Promise.all(waits);
  };
  return { dispatch, did };
}

const click = (url: unknown) => ({ notification: { data: { url }, close: () => undefined } });

describe("the service worker", () => {
  it("shows a push as the one notification that opens the needs-you list", async () => {
    const { dispatch, did } = worker([{ answers: true }]);
    const message = { title: "Needs you", body: "A proposal for B", url: "/?needs" };
    await dispatch("push", { data: { json: () => message } });
    expect(did).toEqual([
      ["show", "Needs you", { body: "A proposal for B", icon: "/icon.svg", tag: "needs", data: { url: "/?needs" } }],
    ]);
    // An unreadable message still says something needs the user.
    const unreadable = worker([{ answers: true }]);
    await unreadable.dispatch("push", { data: { json: () => JSON.parse("{") } });
    expect(unreadable.did[0]?.[1]).toBe("Needs you");
  });

  it("shows what a notification names in the app already open, without reloading it, or in a new window", async () => {
    const open = worker([{ answers: true }]);
    await open.dispatch("notificationclick", click("/?needs"));
    expect(open.did).toEqual([
      ["focus", 0],
      ["message", 0, { kind: "open", search: "?needs" }],
    ]);
    const none = worker([]);
    await none.dispatch("notificationclick", click("/?needs"));
    expect(none.did).toEqual([["open", `${ORIGIN}/?needs`]]);
  });

  it("reloads an open window that doesn't answer, and prefers the window in use, then one in view", async () => {
    const silent = worker([{ answers: false }]);
    await silent.dispatch("notificationclick", click("/?needs"));
    expect(silent.did.slice(1)).toEqual([
      ["message", 0, { kind: "open", search: "?needs" }],
      ["navigate", `${ORIGIN}/?needs`],
    ]);
    const several = worker([
      { answers: true },
      { answers: true, visibilityState: "visible" },
      { answers: true, focused: true },
    ]);
    await several.dispatch("notificationclick", click("/?needs"));
    expect(several.did[0]).toEqual(["focus", 2]);
    const seen = worker([{ answers: true }, { answers: true, visibilityState: "visible" }]);
    await seen.dispatch("notificationclick", click("/?needs"));
    expect(seen.did[0]).toEqual(["focus", 1]);
  });

  it("never leaves the app, however a URL is spelled", async () => {
    for (const url of [
      "//evil.example/x",
      "/\\evil.example/x",
      "/\t/evil.example",
      "https://evil.example/",
      "javascript:alert(1)",
      7,
    ]) {
      const { dispatch, did } = worker([]);
      await dispatch("notificationclick", click(url));
      expect(did, String(url)).toEqual([["open", `${ORIGIN}/?needs`]]);
    }
  });
});
