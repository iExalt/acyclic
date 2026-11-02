import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { fromKeyboard, typingIn } from "./keys";
import "./app.css";
import { applyTheme, readTheme } from "./palette";

// Before the first render, so a Theme picked on this device shows from the start.
applyTheme(readTheme());

const root = document.getElementById("root");
if (root === null) throw new Error("index.html has no #root");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Key hints show with a mouse, and elsewhere only once a key from a physical keyboard has been pressed: a phone with a
// stylus has a fine pointer too, but no keys.
const seeKeyboard = (event: KeyboardEvent) => {
  if (!fromKeyboard(event, typingIn(event.target))) return;
  document.documentElement.dataset.keyboard = "true";
  window.removeEventListener("keydown", seeKeyboard, true);
};
window.addEventListener("keydown", seeKeyboard, true);

// Registered from the root, so its scope is the whole app.
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/sw.js").catch((err: unknown) => console.error("the service worker failed", err));
}
