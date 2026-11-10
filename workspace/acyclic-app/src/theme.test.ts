/// <reference types="node" />
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const script = readFileSync(new URL("../public/theme.js", import.meta.url), "utf8");
describe("the scheme published before paint", () => {
  for (const system of ["light", "dark"]) {
    for (const kept of [null, "light", "dark", "invalid", "refused"]) {
      it(`${kept ?? "system"} on a ${system} system`, () => {
        const meta = { content: "light dark" };
        const root = { dataset: {} };
        runInNewContext(script, {
          localStorage: {
            getItem: () => {
              if (kept === "refused") throw new Error("refused");
              return kept;
            },
          },
          matchMedia: () => ({ matches: system === "light" }),
          document: { documentElement: root, querySelector: () => meta },
        });
        expect(meta.content).toBe(kept === "light" || kept === "dark" ? kept : system);
      });
    }
  }
});
