import { describe, expect, it } from "vitest";
import { designHref, readDesign } from "./look";

/** A storage that holds what it's given, as the browser's does. */
function storage(): Storage {
  const held = new Map<string, string>();
  return {
    getItem: (key) => held.get(key) ?? null,
    setItem: (key, value) => void held.set(key, value),
  } as Storage;
}

describe("the design compared", () => {
  it("is the address's, else the one remembered, else the first, and ignores what it doesn't know", () => {
    const kept = storage();
    expect(readDesign("", kept)).toEqual({ look: "a", home: "map" });
    expect(readDesign("?look=b&home=overview", kept)).toEqual({ look: "b", home: "overview" });
    expect(readDesign("?needs", kept)).toEqual({ look: "b", home: "overview" });
    expect(readDesign("?look=z", kept)).toEqual({ look: "a", home: "overview" });
    expect(readDesign("?look=b", null)).toEqual({ look: "b", home: "map" });
  });

  it("is switched by an address that keeps every other parameter, such as a notification's", () => {
    const href = designHref({ pathname: "/", search: "?needs&look=a", hash: "" }, { look: "b", home: "map" });
    expect(new URLSearchParams(href.slice(2)).has("needs")).toBe(true);
    expect(href).toContain("look=b");
    expect(href).toContain("home=map");
  });
});
