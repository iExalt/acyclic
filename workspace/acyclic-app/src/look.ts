// Which of the two designs being compared the app wears, and which view it opens on: `?look=a|b&home=map|overview`,
// remembered on this device. Temporary: once one design is chosen, the other goes, and this with it.

export type Look = "a" | "b";
export type Home = "map" | "overview";

export interface Design {
  look: Look;
  home: Home;
}

const KEY = "acyclic.design";

/** The design the address asks for, else the one remembered, else the first; remembered for next time. */
export function readDesign(search: string, stored: Storage | null): Design {
  const asked = new URLSearchParams(search);
  let kept: Partial<Design> = {};
  try {
    kept = JSON.parse(stored?.getItem(KEY) ?? "{}") as Partial<Design>;
  } catch {
    kept = {};
  }
  const pick = <T extends string>(name: keyof Design, allowed: readonly T[]): T => {
    const value = asked.get(name) ?? kept[name];
    return allowed.includes(value as T) ? (value as T) : (allowed[0] as T);
  };
  const design = { look: pick("look", ["a", "b"] as const), home: pick("home", ["map", "overview"] as const) };
  try {
    stored?.setItem(KEY, JSON.stringify(design));
  } catch {
    // Private browsing may refuse storage; the address still says.
  }
  return design;
}

/** The address for `design`, keeping every other parameter, such as `needs`. */
export function designHref(location: Pick<Location, "pathname" | "search" | "hash">, design: Design): string {
  const params = new URLSearchParams(location.search);
  params.set("look", design.look);
  params.set("home", design.home);
  return `${location.pathname}?${params}${location.hash}`;
}

/** Read once, as the page loads; main.tsx marks the root with the look before the first paint. */
export const DESIGN: Design =
  typeof window === "undefined"
    ? { look: "a", home: "map" }
    : readDesign(
        window.location.search,
        (() => {
          try {
            return window.localStorage;
          } catch {
            return null;
          }
        })(),
      );
