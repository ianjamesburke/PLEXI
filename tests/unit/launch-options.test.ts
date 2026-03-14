import { describe, expect, test } from "bun:test";
import { resolveLaunchOptions } from "../../src/bun/launch-options";

describe("launch options", () => {
  test("uses the default profile when no clean inputs are present", () => {
    const options = resolveLaunchOptions(
      ["bun", "main.js"],
      {},
    );

    expect(options).toEqual({
      clean: false,
      profile: "default",
    });
  });

  test("enables clean mode from the CLI flag", () => {
    const options = resolveLaunchOptions(
      ["bun", "main.js", "--clean"],
      {},
    );

    expect(options).toEqual({
      clean: true,
      profile: "clean",
    });
  });

  test("enables clean mode from the environment fallback", () => {
    const options = resolveLaunchOptions(
      ["bun", "main.js"],
      { PLEXI_CLEAN: "1" },
    );

    expect(options).toEqual({
      clean: true,
      profile: "clean",
    });
  });
});
