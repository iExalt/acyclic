// Whether the panes sit side by side, as app.css has them from 900 px wide, rather than stacked as on a phone.

import { useEffect, useMemo, useState } from "react";

/** Where the panes sit side by side, as app.css has it. */
export const SIDE_BY_SIDE = "(min-width: 900px)";

/** Where the Overview's lists stand in columns, from an upright tablet up, as app.css has it. */
export const COLUMNS = "(min-width: 700px)";

/** Whether the panes sit side by side now, or `media` holds, following the window as it changes. */
export function useWide(media: string = SIDE_BY_SIDE): boolean {
  const query = useMemo(() => (typeof matchMedia === "undefined" ? null : matchMedia(media)), [media]);
  const [wide, setWide] = useState(() => query?.matches ?? false);
  useEffect(() => {
    if (query === null) return;
    const change = () => setWide(query.matches);
    query.addEventListener("change", change);
    return () => query.removeEventListener("change", change);
  }, [query]);
  return wide;
}
