import { describe, expect, test } from "bun:test";
import { createApplicationMenu } from "../../src/bun/application-menu";

describe("application menu", () => {
  test("uses an unlabeled top-level app menu for macOS", () => {
    const menu = createApplicationMenu();

    expect(menu[0]).not.toHaveProperty("label");
    expect(menu[0]).toHaveProperty("submenu");
  });

  test("does not assign custom accelerators to native role items", () => {
    const menu = createApplicationMenu();
    const roleItems = menu
      .flatMap((item) => ("submenu" in item && item.submenu ? item.submenu : []))
      .filter((item) => "role" in item);

    expect(roleItems.length).toBeGreaterThan(0);

    for (const item of roleItems) {
      expect(item.accelerator).toBeUndefined();
    }
  });
});
