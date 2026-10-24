// The service worker, run from its own file in a stand-in worker scope, so its handlers are tested as shipped.
/// <reference types="node" />

import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const ORIGIN = "https://acyclic.example";
const source = readFileSync(new URL("../public/sw.js", import.meta.url), "utf8");

/** Runs the worker with one window open, or none, and gives its handlers and what they did. */
function worker(open: boolean) {
  const handlers = new Map<string, (event: unknown) => void>();
  const did: unknown[][] = [];
  const window = {
    url: `${ORIGIN}/`,
    focus: async () => did.push(["focus"]),
    navigate: async (url: string) => did.push(["navigate", url]),
  };
  const self = {
    location: { origin: ORIGIN },
    addEventListener: (type: string, handler: (event: unknown) => void) => handlers.set(type, handler),
    skipWaiting: () => undefined,
    registration: {
      showNotification: async (title: string, options: unknown) => did.push(["show", title, options]),
    },
    clients: {
      claim: async () => undefined,
      matchAll: async () => (open ? [window] : []),
      openWindow: async (url: string) => did.push(["open", url]),
    },
  };
  runInNewContext(source, { self, URL });
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
    const { dispatch, did } = worker(true);
    const message = { title: "Needs you", body: "A proposal for B", url: "/?needs" };
    await dispatch("push", { data: { json: () => message } });
    expect(did).toEqual([
      ["show", "Needs you", { body: "A proposal for B", icon: "/icon.svg", tag: "needs", data: { url: "/?needs" } }],
    ]);
    // An unreadable message still says something needs the user.
    const unreadable = worker(true);
    await unreadable.dispatch("push", { data: { json: () => JSON.parse("{") } });
    expect(unreadable.did[0]?.[1]).toBe("Needs you");
  });

  it("opens what a notification names in the open window, or a new one", async () => {
    const open = worker(true);
    await open.dispatch("notificationclick", click("/?needs"));
    expect(open.did).toEqual([["focus"], ["navigate", `${ORIGIN}/?needs`]]);
    const none = worker(false);
    await none.dispatch("notificationclick", click("/?needs"));
    expect(none.did).toEqual([["open", `${ORIGIN}/?needs`]]);
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
      const { dispatch, did } = worker(false);
      await dispatch("notificationclick", click(url));
      expect(did, String(url)).toEqual([["open", `${ORIGIN}/?needs`]]);
    }
  });
});
