import { describe, expect, it } from "vitest";
import { uuidv7 } from "./uuid";

describe("uuidv7", () => {
  it("is a lowercase hyphenated version 7 id that holds the time, and is fresh each call", () => {
    const at = 0x01900000abcd;
    const id = uuidv7(at);
    expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(id.replace("-", "").slice(0, 12)).toBe("01900000abcd");
    expect(uuidv7(at)).not.toBe(id);
  });
});
