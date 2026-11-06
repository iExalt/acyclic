import { describe, expect, it } from "vitest";
import { metricsOf } from "./labelmetrics";

const fonts = { regular: (text: string) => [...text].length * 8, bold: (text: string) => [...text].length * 8 };
const normalized = (text: string) => text.replace(/`([^`]+)`/g, "$1").replace(/\s/g, "");

describe("map label metrics", () => {
  it("keeps an ordinary title at the existing tile size and adds its stage as words", () => {
    const measured = metricsOf(
      { title: "Fix the map", kind: "leaf", hidden: 0, stage: "ready", progress: null },
      fonts,
    );
    expect(measured).toMatchObject({ width: 200, height: 64, lines: ["Fix the map", "Ready"] });
  });

  it("wraps full titles, long unbroken tokens, multiline text and Unicode graphemes without truncating them", () => {
    const token = "supercalifragilisticexpialidocious".repeat(2);
    const title = `A long \`title\`\nwith 🧑🏽‍🚀 and ${token}`;
    const measured = metricsOf({ title, kind: "leaf", hidden: 0, stage: null, progress: null }, fonts);
    expect(normalized(measured.lines.join(""))).toBe(normalized(title));
    expect(measured.lines.some((line) => line.includes("🧑🏽‍🚀"))).toBe(true);
    expect(measured.textWidth).toBe(126);
    expect(measured.height).toBeGreaterThan(64);
    expect(measured.height).toBeGreaterThanOrEqual(Math.ceil(measured.lines.length * 16.25 + 20));
    expect(measured.lines.every((line) => fonts.regular(line.trimEnd()) <= measured.textWidth)).toBe(true);
    expect(
      metricsOf(
        { title: "Run `acyclic up` on the Mac", kind: "leaf", hidden: 0, stage: null, progress: null },
        fonts,
      ).lines.join(""),
    ).toBe("Run acyclic up on the Mac");
  });

  it("retains box stage, leaf counts and the collapsed hidden count", () => {
    const progress = {
      done: 1,
      leaves: 4,
      in_progress: 1,
      ready: 1,
      waiting: 1,
      parked: 0,
      unthreaded: 0,
    };
    const measured = metricsOf(
      { title: "Release", kind: "collapsed", hidden: 18, stage: "in-progress", progress },
      fonts,
    );
    expect(normalized(measured.lines.join(""))).toContain(normalized("Release (+18)"));
    expect(measured.lines).toContain("In progress");
    expect(normalized(measured.lines.join(""))).toContain(normalized("1 done, 1 in progress, 1 ready, 1 waiting"));
    expect(measured.lines.every((line) => fonts.bold(line.trimEnd()) <= measured.textWidth)).toBe(true);
  });

  it("keeps folded-thread names and the full done note", () => {
    const measured = metricsOf(
      {
        title: "A finished thread with a long name",
        kind: "collapsed",
        hidden: 0,
        stage: "done",
        progress: null,
        doneBox: { kind: "thread", thread: "release" },
        note: "12 done",
      },
      fonts,
    );
    expect(normalized(measured.lines.join(""))).toContain(normalized("A finished thread with a long name"));
    expect(measured.lines).toContain("Done");
    expect(normalized(measured.lines.join(""))).toContain(normalized("12 done"));
    expect(measured.lines.every((line) => fonts.bold(line.trimEnd()) <= measured.textWidth)).toBe(true);
  });

  it("shows Needs a thread even though a tile without a thread has no stage icon", () => {
    const measured = metricsOf(
      { title: "Unassigned", kind: "leaf", hidden: 0, stage: "unthreaded", progress: null },
      fonts,
    );
    expect(measured.lines).toEqual(["Unassigned", "Needs a thread"]);
    expect(measured.lines.every((line) => fonts.regular(line.trimEnd()) <= measured.textWidth)).toBe(true);
  });

  it("keeps a short box header and grows it to fit every title and status line", () => {
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
    expect(long.header).toBe(81);
    expect(long.height).toBe(85);
    expect(long.lines.every((line) => fonts.bold(line.trimEnd()) <= long.textWidth)).toBe(true);
  });
});
