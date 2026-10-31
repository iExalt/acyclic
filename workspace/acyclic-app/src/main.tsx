import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./app.css";
import { DESIGN } from "./look";
import { PALETTES } from "./palette";

// The look being compared marks the root, which its palette and frame in app.css hang from.
document.documentElement.dataset.look = DESIGN.look;
// The bar's colour, which index.html gives for look A.
for (const meta of document.querySelectorAll<HTMLMetaElement>("meta[name=theme-color]")) {
  meta.content = PALETTES[DESIGN.look][meta.media.includes("light") ? "light" : "dark"].chrome;
}

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
