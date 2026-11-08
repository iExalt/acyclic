import { describe, expect, it } from "vitest";
import { metricsOf, STUB_ROOM } from "./labelmetrics";

const fonts = { regular: (text: string) => [...text].length * 8, bold: (text: string) => [...text].length * 8 };
const normalized = (text: string) => text.replace(/`([^`]+)`/g, "$1").replace(/\s/g, "");

describe("map label metrics", () => {
  it("keeps an ordinary title at the existing tile size without repeating its stage", () => {
    const measured = metricsOf(
      { title: "Fix the map", kind: "leaf", hidden: 0, stage: "ready", progress: null },
      fonts,
    );
    expect(measured).toMatchObject({ width: 200, height: 86, lines: ["Fix the map"] });
  });

  it("keeps short, long, multiline and stubbed items the same size, with room for titles and ellipsis", () => {
    const leaf = { kind: "leaf", hidden: 0, stage: "ready", progress: null } as const;
    for (const title of [
      "Short",
      "A long title that needs many lines ".repeat(8),
      "supercalifragilisticexpialidocious".repeat(3),
      "First\nSecond\nThird\nFourth\nFifth",
      "🧑🏽‍🚀".repeat(30),
      "가".repeat(50),
    ]) {
      for (const outside of [[], [{ id: "g", title: "Gate", initiative: "Lab" }]]) {
        const measured = metricsOf({ ...leaf, title, outside }, fonts);
        expect(measured).toMatchObject({ width: 200, height: 86 });
        const limit = outside.length ? 2 : 4;
        expect(measured.lines.length).toBeLessThanOrEqual(limit);
        if (title !== "Short") expect(measured.lines.at(-1)?.endsWith("…")).toBe(true);
        expect(measured.lines.every((line) => fonts.regular(line.trimEnd()) <= measured.textWidth)).toBe(true);
        const last = measured.lines.at(-1)?.replace(/…$/, "") ?? "";
        if (title.startsWith("가")) expect(last.length % 2).toBe(0);
        expect(measured.lines.join("")).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])/u);
      }
      expect(metricsOf({ ...leaf, title, stage: null }, fonts).height).toBe(86);
    }
  });

  it("removes whole graphemes to leave room for the ellipsis", () => {
    const measured = metricsOf(
      {
        title: `a${"가".repeat(50)}`,
        kind: "leaf",
        hidden: 0,
        stage: "ready",
        progress: null,
        outside: [{ id: "g", title: "Gate", initiative: "Lab" }],
      },
      fonts,
    );
    expect(measured.lines[0]).toMatch(/^a(?:가)+$/u);
    expect(measured.lines[1]).toMatch(/^(?:가)+…$/u);
    expect(fonts.regular(measured.lines[0] ?? "")).toBeLessThanOrEqual(measured.textWidth);
  });

  it("retains leaf counts and the collapsed hidden count without repeating the stage", () => {
    const progress = {
      done: 1,
      leaves: 4,
      in_progress: 1,
      ready: 1,
      stalled: 0,
      waiting: 1,
      parked: 0,
      canceled: 0,
    };
    const measured = metricsOf(
      { title: "Release", kind: "collapsed", hidden: 18, stage: "in-progress", progress },
      fonts,
    );
    expect(normalized(measured.lines.join(""))).toContain(normalized("Release (+18)"));
    expect(measured.lines).not.toContain("In progress");
    expect(normalized(measured.lines.join(""))).toContain(normalized("1 done, 1 in progress"));
    expect(measured.lines.every((line) => fonts.bold(line.trimEnd()) <= measured.textWidth)).toBe(true);
  });

  it("fits collapsed titles, counts and stubs into the common tile size", () => {
    for (const title of ["Short", "A long epic title ".repeat(20)]) {
      for (const outside of [[], [{ id: "g", title: "Gate", initiative: "Lab" }]]) {
        for (const progress of [
          null,
          { done: 123, leaves: 456, in_progress: 1, ready: 2, stalled: 3, waiting: 4, parked: 5, canceled: 6 },
        ]) {
          const measured = metricsOf({ title, kind: "collapsed", hidden: 99, stage: "done", progress, outside }, fonts);
          expect(measured).toMatchObject({ width: 200, height: 86 });
          expect(measured.lines.length * 16.25 + 20 + measured.reserve).toBeLessThanOrEqual(measured.height);
          expect(measured.lines.every((line) => fonts.bold(line.trimEnd()) <= measured.textWidth)).toBe(true);
          if (progress !== null) expect(measured.lines.join(" ")).toContain("123 done");
        }
      }
    }
  });

  it("keeps a short box header and grows it to fit every title line", () => {
    const short = metricsOf({ title: "Box", kind: "box", hidden: 0, stage: null, progress: null }, fonts);
    const long = metricsOf(
      {
        title: "A box title that needs several lines",
        kind: "box",
        hidden: 0,
        stage: "waiting",
        progress: null,
      },
      fonts,
    );
    expect(short.header).toBe(52);
    expect(long.header).toBe(65);
    expect(long.height).toBe(69);
    expect(long.lines.every((line) => fonts.bold(line.trimEnd()) <= long.textWidth)).toBe(true);
  });
});

describe("a stub for waits outside the plan", () => {
  const leaf = { title: "Fix the map", kind: "leaf", hidden: 0, stage: "ready", progress: null } as const;
  const gate = { id: "g", title: "Reserve the ingress address", initiative: "Home lab" };

  it("takes room for a tile or box stub, including a row below an open box title band", () => {
    expect(metricsOf(leaf, fonts).stub).toBeNull();
    expect(metricsOf(leaf, fonts).reserve).toBe(0);
    const stubbed = metricsOf({ ...leaf, outside: [gate] }, fonts);
    expect(stubbed).toMatchObject({ reserve: STUB_ROOM, height: 86 });
    expect(metricsOf({ ...leaf, kind: "collapsed", outside: [gate] }, fonts).reserve).toBe(STUB_ROOM);
    const box = metricsOf({ ...leaf, kind: "box", outside: [gate] }, fonts);
    expect(box).toMatchObject({ reserve: STUB_ROOM, header: 52 + STUB_ROOM, stub: { target: "g" } });
    expect(box.height).toBe(64 + STUB_ROOM);
  });

  it("names the initiative and the item, cut to the chip, with the rest counted and every one in the hover", () => {
    const one = metricsOf({ ...leaf, outside: [gate] }, fonts).stub;
    expect(one?.target).toBe("g");
    expect(one?.text.startsWith("Home lab: ")).toBe(true);
    expect(one?.text.endsWith("…")).toBe(true);
    expect(one?.full).toBe("Home lab: Reserve the ingress address");
    const short = { id: "s", title: "Ping", initiative: "Lab" };
    expect(metricsOf({ ...leaf, outside: [short] }, fonts).stub?.text).toBe("Lab: Ping");
    const many = metricsOf({ ...leaf, outside: [short, gate, { ...gate, id: "h" }] }, fonts).stub;
    expect(many?.text.endsWith(" +2")).toBe(true);
    expect(many?.target).toBe("s");
    expect(many?.full.split("\n")).toHaveLength(3);
  });
});
