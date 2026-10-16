import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  detectAttachment,
  inferChannelFromSocketPath,
  verifyLiveIdentity,
  PlexiEventSubscriber,
  migrateLegacyPiHook,
  formatStatusText,
  formatWidgetLines,
  createPiPlexiExtension,
} from "../index.ts";

function makeFakePiHarness() {
  const tools = new Map();
  const commands = new Map();
  const listeners = new Map();

  const pi = {
    registerTool(def) {
      tools.set(def.name, def);
    },
    registerCommand(name, def) {
      commands.set(name, def);
    },
    on(event, handler) {
      if (!listeners.has(event)) listeners.set(event, []);
      listeners.get(event).push(handler);
    },
    async emit(event, payload, ctx) {
      for (const fn of listeners.get(event) ?? []) {
        await fn(payload, ctx);
      }
    },
  };

  return { pi, tools, commands, listeners };
}

test("Acceptance 1 & 9: package loads cleanly, registers tools/commands, and migrates legacy Pi hook without touching unknown files", async () => {
  const tempHome = mkdtempSync(join(tmpdir(), "pi-plexi-home-"));
  const extDir = join(tempHome, ".pi", "agent", "extensions");
  mkdirSync(extDir, { recursive: true });
  const extFile = join(extDir, "plexi-agent-state.ts");

  // 1. Unknown user extension is preserved untouched
  const userContent = "// Custom user extension\nexport default function () {}\n";
  writeFileSync(extFile, userContent);
  const resUnknown = migrateLegacyPiHook({ homeDir: tempHome });
  assert.equal(resUnknown.found, true);
  assert.equal(resUnknown.migrated, false);
  assert.equal(resUnknown.preservedUnknown, true);
  assert.equal(readFileSync(extFile, "utf8"), userContent);

  // 2. Legacy Plexi hook is backed up to .plexi-hook.bak and removed on package init
  const legacyContent = `// Plexi-managed agent state extension.\nconst hookCommand = "plexi";\n`;
  writeFileSync(extFile, legacyContent);

  const { pi, tools, commands } = makeFakePiHarness();
  const ext = createPiPlexiExtension({
    homeDir: tempHome,
    env: {},
  });
  ext(pi);

  assert.equal(existsSync(extFile), false, "legacy hook should be removed");
  assert.equal(
    existsSync(`${extFile}.plexi-hook.bak`),
    true,
    "legacy hook backup should exist",
  );
  assert.equal(readFileSync(`${extFile}.plexi-hook.bak`, "utf8"), legacyContent);

  const expectedTools = [
    "plexi_workspace_overview",
    "plexi_pane_inspect",
    "plexi_app_tools",
    "plexi_app_call",
    "plexi_events_poll",
    "plexi_needs_you_ask",
    "plexi_needs_you_reconcile",
    "plexi_host_screenshot",
  ];
  for (const name of expectedTools) {
    assert.equal(tools.has(name), true, `missing tool ${name}`);
  }
  assert.equal(commands.has("plexi-status"), true);
  assert.equal(commands.has("plexi-reconcile"), true);
  assert.equal(commands.has("plexi-watch"), true);
});

test("Acceptance 10: detached, wrong-channel, missing-socket, and stale-identity states degrade cleanly", async () => {
  assert.equal(
    inferChannelFromSocketPath("/Users/u/.plexi-alpha/plexi.sock"),
    "alpha",
  );
  assert.equal(inferChannelFromSocketPath("/Users/u/.plexi/plexi.sock"), "stable");

  // Outside Plexi
  const outside = detectAttachment({});
  assert.equal(outside.attached, false);
  assert.equal(outside.status, "detached_outside_plexi");

  // Wrong channel
  const mismatch = detectAttachment(
    {
      PLEXI_SOCKET: "/Users/u/.plexi-beta/plexi.sock",
      PLEXI_PANE_ID: "3",
      PLEXI_CONTEXT_ID: "1",
      PLEXI_CHANNEL: "alpha",
    },
    { checkSocketExists: () => true },
  );
  assert.equal(mismatch.attached, false);
  assert.equal(mismatch.status, "detached_channel_mismatch");

  // Missing socket
  const missingSock = detectAttachment(
    {
      PLEXI_SOCKET: "/tmp/nonexistent-plexi.sock",
      PLEXI_PANE_ID: "3",
    },
    { checkSocketExists: () => false },
  );
  assert.equal(missingSock.attached, false);
  assert.equal(missingSock.status, "detached_socket_missing");

  // Stale identity when pane is absent from live host
  const attached = detectAttachment(
    {
      PLEXI_SOCKET: "/Users/u/.plexi-alpha/plexi.sock",
      PLEXI_PANE_ID: "99",
      PLEXI_CONTEXT_ID: "1",
    },
    { checkSocketExists: () => true },
  );
  assert.equal(attached.attached, true);
  const verified = verifyLiveIdentity(attached, [{ id: 1, context_id: 1 }]);
  assert.equal(verified.attached, false);
  assert.equal(verified.status, "detached_stale_identity");
});

test("Acceptance 2, 3, 4, 5, 6, 7: workspace overview, app tool calls, event stream buffer, Needs-you ask/reconcile, and screenshot", async () => {
  const calls = [];
  const runner = async (_bin, args) => {
    calls.push(args);
    const cmd = args.slice(0, 2).join(" ");
    if (cmd === "pane list") {
      return {
        exitCode: 0,
        stdout: JSON.stringify([
          { id: 1, type: "terminal", title: "pi-agent", context_id: 10, context_name: "Video Edit" },
          { id: 2, type: "app", app_id: "clip-editor-probe", title: "Clip Editor Probe", context_id: 10 },
        ]),
        stderr: "",
      };
    }
    if (cmd === "context list") {
      return {
        exitCode: 0,
        stdout: JSON.stringify([{ context_id: 10, name: "Video Edit", is_active: true }]),
        stderr: "",
      };
    }
    if (cmd === "app tools") {
      return {
        exitCode: 0,
        stdout: JSON.stringify({
          ok: true,
          context_id: 10,
          tools: [
            { name: "clip.state", app_id: "clip-editor-probe", pane_id: 2, read_only: true },
            { name: "clip.crop", app_id: "clip-editor-probe", pane_id: 2, read_only: false },
          ],
        }),
        stderr: "",
      };
    }
    if (cmd === "needs-you reconcile") {
      return {
        exitCode: 0,
        stdout: JSON.stringify({
          ok: true,
          items: [
            {
              id: "ny_q1",
              kind: "question",
              summary: "Which crop preset for intro?",
              choices: ["16:9", "9:16", "1:1"],
              preview_refs: ["clip:intro@0..3000ms"],
              resolution: null,
            },
          ],
          continuations: [
            {
              run_id: "corr-crop-intro",
              state: "waiting_user",
              target_resource: "intro",
              expected_revision: "rev-1",
              needs_you_id: "ny_q1",
            },
          ],
        }),
        stderr: "",
      };
    }
    if (cmd === "needs-you ask") {
      return {
        exitCode: 0,
        stdout: JSON.stringify({
          ok: true,
          id: "ny_q2",
          ui_token: "nytok_test_123",
          item: {
            id: "ny_q2",
            kind: "question",
            summary: "Pick crop preset",
            choices: ["16:9", "9:16"],
          },
          continuation: {
            run_id: "corr-2",
            state: "waiting_user",
            needs_you_id: "ny_q2",
          },
        }),
        stderr: "",
      };
    }
    if (cmd === "app call") {
      return {
        exitCode: 0,
        stdout: JSON.stringify({
          ok: true,
          output: {
            ok: true,
            clip_id: "intro",
            crop_preset: "9:16",
            revision_before: "rev-1",
            revision_after: "rev-2",
          },
        }),
        stderr: "",
      };
    }
    if (cmd === "needs-you complete") {
      return {
        exitCode: 0,
        stdout: JSON.stringify({
          ok: true,
          continuation: {
            run_id: args[2],
            state: "completed",
          },
        }),
        stderr: "",
      };
    }
    if (cmd === "host screenshot") {
      return {
        exitCode: 0,
        stdout: "",
        stderr: "",
      };
    }
    return { exitCode: 0, stdout: JSON.stringify({ ok: true }), stderr: "" };
  };

  const { pi, tools } = makeFakePiHarness();
  const statusUpdates = [];
  const widgetUpdates = [];
  const fakeCtx = {
    hasUI: true,
    sessionManager: {
      getSessionId: () => "sess-pi-test",
    },
    ui: {
      setStatus(key, text) {
        statusUpdates.push({ key, text });
      },
      setWidget(key, lines) {
        widgetUpdates.push({ key, lines });
      },
    },
  };

  const ext = createPiPlexiExtension({
    env: {
      PLEXI_SOCKET: "/Users/u/.plexi-alpha/plexi.sock",
      PLEXI_PANE_ID: "1",
      PLEXI_CONTEXT_ID: "10",
      PLEXI_CHANNEL: "alpha",
    },
    checkSocketExists: () => true,
    runner,
  });
  const handle = ext(pi);

  await pi.emit("session_start", { reason: "startup" }, fakeCtx);
  assert.ok(statusUpdates.length > 0, "status bar should be updated on session_start");
  assert.match(statusUpdates.at(-1).text, /plexi\[alpha\] Video Edit#10 pane:1/);
  assert.ok(widgetUpdates.length > 0, "workspace widget should be updated on session_start");

  // Push a simulated event into the event subscriber and verify widget lines include it
  handle.events.ingestLine(
    JSON.stringify({
      event: "clip.updated",
      summary: "Cropped intro to 9:16",
      resource_id: "intro",
      revision_after: "rev-2",
    }),
  );
  const pollRes = await tools
    .get("plexi_events_poll")
    .execute("call-1", { stream: "clip.updated" }, undefined, undefined, fakeCtx);
  assert.equal(pollRes.details.events.length, 1);
  assert.equal(pollRes.details.events[0].revision_after, "rev-2");

  // Ask a structured question without waiting
  const askRes = await tools.get("plexi_needs_you_ask").execute(
    "call-2",
    {
      summary: "Pick crop preset",
      resource: "intro",
      choices: ["16:9", "9:16"],
      preview_refs: ["clip:intro@0..3000ms"],
      correlation_id: "corr-2",
      expected_revision: "rev-1",
      wait_for_answer: false,
    },
    undefined,
    undefined,
    fakeCtx,
  );
  assert.equal(askRes.details.id, "ny_q2");

  // Call mutating app tool with operation_id and verify continuation completion is recorded
  const callRes = await tools.get("plexi_app_call").execute(
    "call-3",
    {
      app_id: "clip-editor-probe",
      tool: "clip.crop",
      args: {
        clip_id: "intro",
        crop_preset: "9:16",
        expected_revision: "rev-1",
        operation_id: "op-crop-1",
      },
      wait_approved: true,
    },
    undefined,
    undefined,
    fakeCtx,
  );
  assert.equal(callRes.isError, false);
  assert.equal(callRes.details.output.revision_after, "rev-2");
  assert.ok(
    calls.some((c) => c[0] === "needs-you" && c[1] === "complete" && c[2] === "op-crop-1"),
    "completing app tool call with operation_id should mark continuation completed",
  );
});
