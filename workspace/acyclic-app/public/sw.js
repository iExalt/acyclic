// The app's service worker. It has no fetch handler: every request goes to the server, so a page is never older than
// the server's answer. It exists for installation, and for Web Push: the server's message, decrypted by the browser,
// shows as a notification that opens the needs-you list, in the app already open if there is one.
/// <reference lib="webworker" />
const worker = /** @type {ServiceWorkerGlobalScope} */ (/** @type {unknown} */ (self));
worker.addEventListener("install", () => worker.skipWaiting());
worker.addEventListener("activate", (event) => event.waitUntil(worker.clients.claim()));

/**
 * The path and query of `url` when it resolves to the app's own origin, else the needs-you list: a message can't send
 * the user elsewhere, however its URL is spelled (a backslash or a tab can turn `/…` into another host).
 */
const inApp = (/** @type {unknown} */ url) => {
  if (typeof url === "string") {
    try {
      const resolved = new URL(url, worker.location.origin);
      if (resolved.origin === worker.location.origin) return resolved.pathname + resolved.search;
    } catch {
      // Not a URL at all.
    }
  }
  return "/?needs";
};

worker.addEventListener("push", (event) => {
  /** @type {{title?: string, body?: string, url?: string}} */
  let message = {};
  try {
    message = event.data?.json() ?? {};
  } catch {
    // An unreadable message still says something needs the user.
  }
  event.waitUntil(
    worker.registration.showNotification(message.title ?? "Needs you", {
      body: message.body ?? "",
      icon: "/icon.svg",
      // A newer message replaces the last, which it covers.
      tag: "needs",
      data: { url: inApp(message.url) },
    }),
  );
});

/**
 * Asks an open app to show what `search` names, as it would a link, rather than reloading it, which would lose what
 * the user is typing. Whether it answered within a second.
 */
const ask = (/** @type {Client} */ client, /** @type {string} */ search) =>
  new Promise((resolve) => {
    const channel = new MessageChannel();
    const done = (/** @type {boolean} */ answered) => {
      clearTimeout(timer);
      channel.port1.close();
      resolve(answered);
    };
    const timer = setTimeout(() => done(false), 1000);
    channel.port1.onmessage = () => done(true);
    client.postMessage({ kind: "open", search }, [channel.port2]);
  });

worker.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = new URL(inApp(event.notification.data?.url), worker.location.origin);
  event.waitUntil(
    (async () => {
      const open = await worker.clients.matchAll({ type: "window", includeUncontrolled: true });
      const ours = open.filter((one) => new URL(one.url).origin === worker.location.origin);
      // The window the user is in, else one they can see, else any.
      const client =
        ours.find((one) => one.focused) ?? ours.find((one) => one.visibilityState === "visible") ?? ours[0];
      if (client === undefined) return void (await worker.clients.openWindow(url.href));
      await client.focus();
      if (url.pathname === "/" && (await ask(client, url.search))) return;
      // A page the worker doesn't control can't be navigated: a new window it is.
      await client.navigate(url.href).catch(() => worker.clients.openWindow(url.href));
    })(),
  );
});
