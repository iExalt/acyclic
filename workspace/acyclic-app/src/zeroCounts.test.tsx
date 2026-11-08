import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { NO_COUNTS } from "./api";
import { InitiativeSwitcher } from "./InitiativeSwitcher";
import { Overview, type OverviewProps } from "./Overview";

const empty: OverviewProps = {
  needs: [],
  lines: new Map(),
  titleOf: (id) => id,
  counts: new Map(),
  outline: null,
  scope: {
    order: [],
    context: null,
    titles: new Map(),
    ofThread: () => new Set([null]),
    ofNeed: () => new Set([null]),
    ofProject: () => new Set([null]),
  },
  items: new Map(),
  onNeeds: () => {},
  onPickThread: () => {},
  onSelectProject: () => {},
  onView: () => {},
};

const badges = (html: string) => [...html.matchAll(/class="count muted">([^<]+)</g)].map((match) => match[1]);

describe("decorative zero counts", () => {
  it("omits Overview badges for an absent or empty outline, keeping the empty explanations", () => {
    for (const outline of [null, { projects: [], others: [] }]) {
      const html = renderToStaticMarkup(<Overview {...empty} outline={outline} />);
      expect(badges(html)).toEqual([]);
      expect(html).toContain("No thread has work in progress or in review.");
      expect(html).toContain("No thread has work ready to start.");
      expect(html).toContain("No projects yet: add an item, and work inside it.");
    }
  });

  it("keeps positive totals, including the Needs you cap", () => {
    const html = renderToStaticMarkup(
      <Overview
        {...empty}
        needs={Array.from({ length: 100 }, (_, n) => ({ kind: "unknown", identity: String(n) }))}
        counts={
          new Map([
            ["working", { ...NO_COUNTS, leaves: 1, in_progress: 1 }],
            ["next", { ...NO_COUNTS, leaves: 1, ready: 1 }],
          ])
        }
        outline={{ projects: [{ id: "project", title: "Project", initiative: null, threads: [] }], others: [] }}
      />,
    );
    expect(badges(html)).toEqual(["99+", "1", "1", "1"]);
  });

  it("keeps empty initiatives as current, selectable buttons without a badge, and shows positive counts", () => {
    const html = renderToStaticMarkup(
      <InitiativeSwitcher
        rows={[
          { id: "empty", title: "Empty initiative", open: 0 },
          { id: "busy", title: "Busy initiative", open: 3 },
        ]}
        context="empty"
        onChoose={() => {}}
      />,
    );
    expect(html).toContain('data-initiative="empty" aria-current="true" title="Empty initiative: 0 open"');
    expect(html.match(/class="initiative-count"/g)).toHaveLength(1);
    expect(html).toContain('class="initiative-count">3<span class="open"> open</span>');
    expect(html).toContain("Empty initiative");
  });
});
