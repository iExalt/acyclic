// Publish the active scheme before paint and extension detection; palette.ts keeps it in sync once the app starts.
let theme;
try {
  theme = localStorage.getItem("acyclic.theme");
} catch {
  // Storage refused: the system's scheme it is.
}
if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
const scheme =
  theme === "light" || theme === "dark"
    ? theme
    : matchMedia("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
document.querySelector('meta[name="color-scheme"]').content = scheme;
