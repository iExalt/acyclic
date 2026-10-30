import { describe, expect, it } from "vitest";
import { spans } from "./Prose";

describe("spans", () => {
  it("sets backticked runs as code and leaves the rest as written", () => {
    expect(spans("Podman's helpers (`gvproxy`, `vfkit`)")).toEqual([
      { text: "Podman's helpers (", code: false },
      { text: "gvproxy", code: true },
      { text: ", ", code: false },
      { text: "vfkit", code: true },
      { text: ")", code: false },
    ]);
  });

  it("keeps a backtick with no partner, and an empty pair, as text", () => {
    expect(spans("an `` empty pair")).toEqual([{ text: "an `` empty pair", code: false }]);
    expect(spans("`one` and ` stray")).toEqual([
      { text: "one", code: true },
      { text: " and ` stray", code: false },
    ]);
    expect(spans("")).toEqual([]);
  });
});
