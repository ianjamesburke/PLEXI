import { describe, expect, test } from "bun:test";
import { createApplicationMenu } from "../../src/bun/application-menu";

describe("application menu", () => {
  test("uses an unlabeled top-level app menu for macOS", () => {
    const menu = createApplicationMenu();

    expect(menu[0]).not.toHaveProperty("label");
    expect(menu[0]).toHaveProperty("submenu");
  });

  test("keeps the native quit role only in the macOS app menu", () => {
    const menu = createApplicationMenu();
    const appMenuSubmenu = "submenu" in menu[0] && menu[0].submenu ? menu[0].submenu : [];
    const fileMenu = menu.find((item) => "label" in item && item.label === "File");
    const fileMenuSubmenu = fileMenu && "submenu" in fileMenu && fileMenu.submenu ? fileMenu.submenu : [];
    const quitItems = menu
      .flatMap((item) => ("submenu" in item && item.submenu ? item.submenu : []))
      .filter((item) => "role" in item && item.role === "quit");

    expect(appMenuSubmenu.some((item) => "role" in item && item.role === "quit")).toBeTrue();
    expect(fileMenuSubmenu.some((item) => "role" in item && item.role === "quit")).toBeFalse();
    expect(quitItems).toHaveLength(1);
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
