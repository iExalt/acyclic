import { describe, expect, it } from "vitest";
import { marked, spans } from "./Prose";

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

describe("marked", () => {
  const run = (text: string, marks: Partial<{ code: boolean; strong: boolean; em: boolean }> = {}) => ({
    text,
    code: false,
    strong: false,
    em: false,
    ...marks,
  });

  it("sets a strong run that holds code, and emphasis, as Markdown does", () => {
    expect(marked("**Done (`fb405a2`):** a model, *mostly* _typed_")).toEqual([
      run("Done (", { strong: true }),
      run("fb405a2", { code: true, strong: true }),
      run("):", { strong: true }),
      run(" a model, "),
      run("mostly", { em: true }),
      run(" "),
      run("typed", { em: true }),
    ]);
  });

  it("leaves markers with no partner, inside a word or beside a space as written, and reads nothing else", () => {
    expect(marked("an odd ** marker")).toEqual([run("an odd ** marker")]);
    expect(marked("snake_case_name and 2 * 3 * 4")).toEqual([run("snake_case_name and 2 * 3 * 4")]);
    expect(marked("<b>not</b> [a](link)")).toEqual([run("<b>not</b> [a](link)")]);
    expect(marked("`**` stays code")).toEqual([run("**", { code: true }), run(" stays code")]);
  });
});
