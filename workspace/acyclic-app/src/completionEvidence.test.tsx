import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { type Item, NO_COUNTS } from "./api";
import { CompletionEvidence } from "./Inspector";

const item = { state: "done", conflicts: {} } as Item;
const show = (changes: Partial<Item>) => renderToStaticMarkup(<CompletionEvidence item={{ ...item, ...changes }} />);

describe("completion evidence", () => {
  it("shows missing and empty evidence and hides stale or derived proof", () => {
    expect(show({})).toContain("No completion evidence recorded");
    for (const evidence of [null, {}]) {
      expect(show({ evidence })).toContain("No completion evidence recorded");
    }
    expect(show({ state: "open", evidence: { text: "old" } })).toBe("");
    expect(show({ conflicts: { state: ["done", "open"] }, evidence: { text: "old" } })).toBe("");
    expect(show({ progress: { counts: NO_COUNTS }, evidence: { text: "old" } })).toBe("");
  });

  it("keeps each conflicting record and the cleared alternative, escaping untrusted values", () => {
    const html = show({
      evidence: {
        conflict: [{ text: "**Done** <script>", repository: "<repo>", commit: "abc" }, null, { text: "other" }],
      },
    });
    expect(html).toContain("Conflicting evidence");
    expect(html).toContain("<strong>Done</strong> &lt;script&gt;");
    expect(html).toContain("&lt;repo&gt;");
    expect(html).toContain("abc");
    expect(html).toContain("Cleared (no evidence)");
    expect(html).toContain("other");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<a ");
  });
});
