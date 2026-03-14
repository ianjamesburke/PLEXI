import { describe, expect, test } from "bun:test";
import { createApplicationMenu, QUIT_ACCELERATOR } from "../../src/bun/application-menu";

describe("application menu", () => {
  test("maps quit to CommandOrControl+Q in both native quit entries", () => {
    const menu = createApplicationMenu();
    const quitItems = menu
      .flatMap((item) => ("submenu" in item && item.submenu ? item.submenu : []))
      .filter((item) => "role" in item && item.role === "quit");

    expect(quitItems).toHaveLength(2);

    for (const item of quitItems) {
      expect(item.accelerator).toBe(QUIT_ACCELERATOR);
    }
  });
});
