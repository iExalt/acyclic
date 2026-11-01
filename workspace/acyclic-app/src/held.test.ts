import { describe, expect, it } from "vitest";
import { heldState } from "./held";

describe("heldState", () => {
  it("holds every action while a write is out, and marks busy only the one pressed", () => {
    expect(heldState(true, "approve", "approve")).toEqual({ "aria-disabled": true, "aria-busy": true });
    expect(heldState(true, "approve", "revoke")).toEqual({ "aria-disabled": true, "aria-busy": undefined });
    // A write from elsewhere holds them all, none busy.
    expect(heldState(true, null, "approve")).toEqual({ "aria-disabled": true, "aria-busy": undefined });
    expect(heldState(false, "approve", "approve")).toEqual({ "aria-disabled": undefined, "aria-busy": undefined });
  });
});
