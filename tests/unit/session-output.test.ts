import { describe, expect, test } from "bun:test";
import {
  extractSessionOutputMetadata,
  formatPathLabel,
  inferHomeDirectory,
} from "../../src/mainview/session-output.js";

describe("session output metadata", () => {
  test("strips OSC 7 cwd sequences from terminal output and returns the last cwd", () => {
    // OSC 7 format: \e]7;file://hostname/path\a — standard protocol used by
    // iTerm2, Ghostty, Kitty, WezTerm, fish, and now Plexi's shell integration.
    const result = extractSessionOutputMetadata(
      "before\u001b]7;file://localhost/tmp/project\u0007after\u001b]7;file:///tmp/next\u0007",
    );

    expect(result.cleaned).toBe("beforeafter");
    expect(result.cwd).toBe("/tmp/next");
  });

  test("infers the home directory from a labeled prompt path", () => {
    expect(inferHomeDirectory("/Users/ian/project", "~/project")).toBe("/Users/ian");
    expect(inferHomeDirectory("/Users/ian", "~")).toBe("/Users/ian");
    expect(inferHomeDirectory("/tmp/project", "/tmp/project")).toBeNull();
  });

  test("formats cwd labels relative to the inferred home directory", () => {
    expect(formatPathLabel("/Users/ian/project", "/Users/ian")).toBe("~/project");
    expect(formatPathLabel("/tmp/project", "/Users/ian")).toBe("/tmp/project");
  });
});
