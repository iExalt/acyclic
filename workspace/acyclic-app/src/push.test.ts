import { describe, expect, it } from "vitest";
import { type Device, pushState, turnOff, turnOn } from "./push";

/** A device whose permission starts as `permission` and becomes `answer` when asked, with a push manager if `push`. */
function fake(permission: NotificationPermission, answer: NotificationPermission = permission, push = true) {
  let held: {
    endpoint: string;
    options: { applicationServerKey: ArrayBuffer | null };
    toJSON: () => unknown;
    unsubscribe: () => Promise<boolean>;
  } | null = null;
  const subscribed: unknown[] = [];
  const device: Device = {
    permission: () => permission,
    requestPermission: async () => {
      permission = answer;
      return answer;
    },
    pushManager: async () =>
      push
        ? {
            getSubscription: async () => held as PushSubscription | null,
            subscribe: async (options?: PushSubscriptionOptionsInit) => {
              subscribed.push(options);
              const endpoint = `https://push.example/d${subscribed.length}`;
              const key = options?.applicationServerKey as Uint8Array<ArrayBuffer>;
              held = {
                endpoint,
                options: { applicationServerKey: key.slice().buffer },
                toJSON: () => ({ endpoint, keys: { p256dh: "k", auth: "a" } }),
                unsubscribe: async () => {
                  held = null;
                  return true;
                },
              };
              return held as PushSubscription;
            },
          }
        : null,
  };
  return { device, subscribed, held: () => held };
}

/** A server that answers each path with `ok`, recording what it was sent; its key can change. */
function server() {
  const sent: { path: string; body: unknown }[] = [];
  const state = { key: "AQID_-8" };
  const fetcher = async (path: string, init?: RequestInit) => {
    sent.push({ path, body: init?.body === undefined ? undefined : JSON.parse(String(init.body)) });
    const data = path === "/api/push/key" ? { key: state.key } : null;
    return new Response(JSON.stringify({ ok: true, data }));
  };
  return { sent, fetcher, state };
}

describe("push", () => {
  it("turns on: asks, subscribes with the server's key, decoded, and hands the subscription to the server", async () => {
    const { device, subscribed } = fake("default", "granted");
    const { sent, fetcher } = server();
    expect(await pushState(device, fetcher)).toBe("off");
    expect(await turnOn(device, fetcher)).toBe("on");
    expect(subscribed).toEqual([{ userVisibleOnly: true, applicationServerKey: new Uint8Array([1, 2, 3, 255, 239]) }]);
    expect(sent.map((one) => one.path)).toEqual(["/api/push/key", "/api/push/subscribe"]);
    expect(sent[1]?.body).toMatchObject({ endpoint: "https://push.example/d1" });
    // Opening the app again sends the subscription again, in case the server lost it.
    expect(await pushState(device, fetcher)).toBe("on");
    expect(sent.at(-1)?.path).toBe("/api/push/subscribe");
  });

  it("replaces a subscription made with a key the server no longer has", async () => {
    const { device, subscribed, held } = fake("granted");
    const { sent, fetcher, state } = server();
    await turnOn(device, fetcher);
    state.key = "BAUG";
    expect(await pushState(device, fetcher)).toBe("on");
    expect(sent.slice(2).map((one) => one.path)).toEqual([
      "/api/push/key",
      "/api/push/unsubscribe",
      "/api/push/subscribe",
    ]);
    expect(sent[3]?.body).toEqual({ endpoint: "https://push.example/d1" });
    expect(subscribed[1]).toEqual({ userVisibleOnly: true, applicationServerKey: new Uint8Array([4, 5, 6]) });
    expect(held()?.endpoint).toBe("https://push.example/d2");
  });

  it("turns off: the server forgets the device before the subscription goes", async () => {
    const { device, held } = fake("granted");
    const { sent, fetcher } = server();
    await turnOn(device, fetcher);
    expect(await turnOff(device, fetcher)).toBe("off");
    expect(sent.at(-1)).toEqual({ path: "/api/push/unsubscribe", body: { endpoint: "https://push.example/d1" } });
    expect(held()).toBeNull();
  });

  it("says when notifications are refused, dismissed or impossible, and subscribes to nothing", async () => {
    const { sent, fetcher } = server();
    const refused = fake("default", "denied");
    expect(await turnOn(refused.device, fetcher)).toBe("blocked");
    expect(await pushState(refused.device, fetcher)).toBe("blocked");
    expect(await turnOn(fake("default", "default").device, fetcher)).toBe("off");
    expect(await pushState(fake("granted", "granted", false).device, fetcher)).toBe("unsupported");
    expect(await pushState(null, fetcher)).toBe("unsupported");
    expect(sent).toEqual([]);
  });

  it("refuses to call it on when the server refuses the subscription", async () => {
    const { device } = fake("granted");
    const refusing = async () =>
      new Response(JSON.stringify({ ok: false, error: { code: "push", message: "no", details: null } }));
    await expect(turnOn(device, refusing)).rejects.toThrow("no");
  });
});
