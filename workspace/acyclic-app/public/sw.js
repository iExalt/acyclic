// The app's service worker. It has no fetch handler: every request goes to the server, so a page is never older than
// the server's answer. It exists for installation, and for Web Push.
/// <reference lib="webworker" />
const worker = /** @type {ServiceWorkerGlobalScope} */ (/** @type {unknown} */ (self));
worker.addEventListener("install", () => worker.skipWaiting());
worker.addEventListener("activate", (event) => event.waitUntil(worker.clients.claim()));
