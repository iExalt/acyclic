import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";

const root = document.getElementById("root");
if (root === null) throw new Error("index.html has no #root");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Registered from the root, so its scope is the whole app.
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/sw.js").catch((err: unknown) => console.error("the service worker failed", err));
}
