// Notifications on this device: whether the server pushes to it, and turning that on and off. The browser's parts come
// in as `Device`, so each path is tested with fakes.

import type { Envelope } from "./api";

/** What the device can do about notifications, as the button shows it. */
export type PushState =
  /** The browser has no Web Push, or the page isn't a secure context. */
  | "unsupported"
  /** The user refused notifications to this site; only the browser's settings can undo it. */
  | "blocked"
  | "off"
  | "on";

/** The parts of the browser pushing needs. */
export interface Device {
  /** `Notification.permission`. */
  permission: () => NotificationPermission;
  requestPermission: () => Promise<NotificationPermission>;
  /** The service worker's push manager, once the worker is ready; null without Web Push. */
  pushManager: () => Promise<Pick<PushManager, "getSubscription" | "subscribe"> | null>;
}

type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

/** The browser's own device, or null when it has no Web Push. */
export function browserDevice(): Device | null {
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) return null;
  return {
    permission: () => Notification.permission,
    requestPermission: () => Notification.requestPermission(),
    pushManager: async () => (await navigator.serviceWorker.ready).pushManager,
  };
}

async function call<T>(path: string, body: unknown, fetcher: Fetch): Promise<T> {
  const response = await fetcher(
    path,
    body === undefined
      ? undefined
      : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) },
  );
  const envelope = (await response.json().catch(() => null)) as Envelope<T> | null;
  if (envelope === null) throw new Error(`the server answered ${response.status} without an envelope`);
  if (!envelope.ok) throw new Error(envelope.error.message);
  return envelope.data;
}

/** Whether the server pushes to this device. A subscription the device holds is sent again, in case the server lost it. */
export async function pushState(device: Device | null, fetcher: Fetch = fetch): Promise<PushState> {
  if (device === null) return "unsupported";
  if (device.permission() === "denied") return "blocked";
  const manager = await device.pushManager();
  if (manager === null) return "unsupported";
  const held = await manager.getSubscription();
  if (held === null || device.permission() !== "granted") return "off";
  await call("/api/push/subscribe", held.toJSON(), fetcher);
  return "on";
}

/** Asks for permission, subscribes with the server's key, and gives the subscription to the server. */
export async function turnOn(device: Device, fetcher: Fetch = fetch): Promise<PushState> {
  const permission = await device.requestPermission();
  if (permission === "denied") return "blocked";
  if (permission !== "granted") return "off";
  const manager = await device.pushManager();
  if (manager === null) return "unsupported";
  const { key } = await call<{ key: string }>("/api/push/key", undefined, fetcher);
  const subscription = await manager.subscribe({ userVisibleOnly: true, applicationServerKey: fromBase64Url(key) });
  await call("/api/push/subscribe", subscription.toJSON(), fetcher);
  return "on";
}

/** Tells the server to forget this device, then drops the subscription. */
export async function turnOff(device: Device, fetcher: Fetch = fetch): Promise<PushState> {
  const held = await (await device.pushManager())?.getSubscription();
  if (held) {
    await call("/api/push/unsubscribe", { endpoint: held.endpoint }, fetcher);
    await held.unsubscribe();
  }
  return "off";
}

export function fromBase64Url(text: string): Uint8Array<ArrayBuffer> {
  const binary = atob(
    text
      .replaceAll("-", "+")
      .replaceAll("_", "/")
      .padEnd(Math.ceil(text.length / 4) * 4, "="),
  );
  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}
