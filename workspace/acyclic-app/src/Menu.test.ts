import { describe, expect, it } from "vitest";
import { placed } from "./Menu";

describe("placed", () => {
  // A menu 230 tall under a button at 820–850, in a panel whose foot is at 1045, in a window 1106 tall.
  const button = { top: 820, bottom: 850, left: 700, height: 30 };
  const menu = (top: number) => ({ top, bottom: top + 230, left: 530, height: 230 });

  it("leaves a menu that is whole where it opened", () => {
    expect(placed(menu(600), button, 1045, 1106)).toBeNull();
  });

  it("opens one that would run past the panel's foot above its button, or on the roomier side, scrolling", () => {
    expect(placed(menu(856), button, 1045, 1106)).toMatchObject({ top: "auto", bottom: "292px", maxHeight: "806px" });
    const low = { ...button, top: 120, bottom: 150 };
    expect(placed(menu(156), low, 300, 1106)).toMatchObject({ top: "156px", bottom: "auto", maxHeight: "136px" });
    expect(placed(menu(156), { ...low, top: 200, bottom: 230 }, 300, 1106)).toMatchObject({
      top: "auto",
      maxHeight: "186px",
    });
  });
});
