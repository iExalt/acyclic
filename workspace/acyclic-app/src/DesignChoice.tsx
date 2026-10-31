// The menu's switch between the designs being compared, which reloads the app in the other. Temporary, as look.ts.

import { DESIGN, designHref } from "./look";

export function DesignChoice() {
  const other = DESIGN.look === "a" ? "b" : "a";
  return (
    <button
      type="button"
      data-testid="design"
      onClick={() => window.location.assign(designHref(window.location, { ...DESIGN, look: other }))}
    >
      Compare design {other.toUpperCase()}
    </button>
  );
}
