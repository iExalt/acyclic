// The Theme picked on this device, on the root before the first paint, so a page drawn in the other scheme never
// flashes; palette.ts's applyTheme, which the app runs as it starts, sets the bar's colour too. The key is
// palette.ts's THEME_KEY.
try {
  const theme = localStorage.getItem("acyclic.theme");
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
} catch {
  // Storage refused: the system's scheme it is.
}
