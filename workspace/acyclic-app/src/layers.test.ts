import { describe, expect, it } from "vitest";
import { topLayer } from "./layers";

const none = { mode: false, adding: false, item: false, needs: false, thread: false };

describe("topLayer", () => {
  it("puts a waiting move over a form, a form over an item, an item over the needs list, and that over a thread", () => {
    const all = { mode: true, adding: true, item: true, needs: true, thread: true };
    expect(topLayer(all)).toBe("mode");
    expect(topLayer({ ...all, mode: false })).toBe("add");
    expect(topLayer({ ...all, mode: false, adding: false })).toBe("item");
    expect(topLayer({ ...none, needs: true, thread: true })).toBe("needs");
    expect(topLayer({ ...none, thread: true })).toBe("thread");
    expect(topLayer(none)).toBeNull();
  });
});
