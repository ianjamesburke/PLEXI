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
    registerVirtualModel() {},
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
    "plexi_run",
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
    if (cmd === "run --json") {
      return {
        exitCode: 0,
        stdout: JSON.stringify([
          {
            name: "hello",
            scope: "workspace",
            description: "Say hello",
            run: "echo hi",
            secrets: ["API_KEY"],
          },
          {
            name: "deploy-global",
            scope: "global",
            description: null,
            run: null,
            secrets: [],
          },
        ]),
        stderr: "",
      };
    }
    if (args[0] === "run" && args[1] === "hello") {
      return {
        exitCode: 0,
        stdout: "hi\n",
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

  // List and run commands via plexi_run (shells to `plexi run --json` and `plexi run <name>`)
  const listRunRes = await tools
    .get("plexi_run")
    .execute("call-4", {}, undefined, undefined, fakeCtx);
  assert.equal(listRunRes.isError, false);
  assert.equal(listRunRes.details.commands.length, 2);
  assert.equal(listRunRes.details.commands[0].name, "hello");
  assert.equal(listRunRes.details.commands[0].scope, "workspace");
  assert.deepEqual(listRunRes.details.commands[0].secrets, ["API_KEY"]);

  const execRunRes = await tools
    .get("plexi_run")
    .execute("call-5", { command: "hello", args: ["world"] }, undefined, undefined, fakeCtx);
  assert.equal(execRunRes.isError, false);
  assert.equal(execRunRes.details.stdout, "hi\n");
  assert.ok(
    calls.some(
      (c) => c[0] === "run" && c[1] === "hello" && c[2] === "--" && c[3] === "world",
    ),
    "plexi_run with command and args should shell to `plexi run <name> -- <args>`",
  );
});

test("Stint 0815: session_start and session_shutdown omit sessionId until session file exists on disk or first turn starts", async () => {
  const tempDir = mkdtempSync(join(tmpdir(), "pi-plexi-sessions-"));
  const missingSessionFile = join(tempDir, "2026-10-17_unpersisted-uuid.jsonl");
  const persistedSessionFile = join(tempDir, "2026-10-17_persisted-uuid.jsonl");

  const reportCalls = [];
  const runner = async (_bin, args) => {
    const cmd = args.slice(0, 2).join(" ");
    if (cmd === "agent report") {
      reportCalls.push(args);
      return { exitCode: 0, stdout: "", stderr: "" };
    }
    if (cmd === "pane list") {
      return {
        exitCode: 0,
        stdout: JSON.stringify([{ id: 1, type: "terminal", context_id: 10 }]),
        stderr: "",
      };
    }
    if (cmd === "context list") {
      return {
        exitCode: 0,
        stdout: JSON.stringify([{ context_id: 10, name: "Main", is_active: true }]),
        stderr: "",
      };
    }
    if (cmd === "app tools") {
      return {
        exitCode: 0,
        stdout: JSON.stringify({ ok: true, context_id: 10, tools: [] }),
        stderr: "",
      };
    }
    if (cmd === "needs-you reconcile") {
      return {
        exitCode: 0,
        stdout: JSON.stringify({ ok: true, items: [], continuations: [] }),
        stderr: "",
      };
    }
    return { exitCode: 0, stdout: JSON.stringify({ ok: true }), stderr: "" };
  };

  const { pi } = makeFakePiHarness();
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
  ext(pi);

  // 1. Fresh session whose getSessionFile() does not exist on disk:
  //    session_start and session_shutdown report idle WITHOUT --session-id.
  const freshCtx = {
    hasUI: false,
    sessionManager: {
      getSessionId: () => "unpersisted-uuid",
      getSessionFile: () => missingSessionFile,
    },
  };

  await pi.emit("session_start", { reason: "startup" }, freshCtx);
  assert.equal(reportCalls.length, 1);
  assert.deepEqual(reportCalls[0], ["agent", "report", "--state", "idle", "--agent", "pi"]);

  await pi.emit("session_shutdown", {}, freshCtx);
  assert.equal(reportCalls.length, 2);
  assert.deepEqual(reportCalls[1], ["agent", "report", "--state", "idle", "--agent", "pi"]);

  // 2. Starting a turn (before_agent_start / agent_start) reports the sessionId,
  //    and subsequent agent_end / session_shutdown also report the sessionId.
  reportCalls.length = 0;
  await pi.emit("session_start", { reason: "startup" }, freshCtx);
  assert.deepEqual(reportCalls[0], ["agent", "report", "--state", "idle", "--agent", "pi"]);

  await pi.emit("before_agent_start", {}, freshCtx);
  assert.deepEqual(reportCalls[1], [
    "agent",
    "report",
    "--state",
    "working",
    "--agent",
    "pi",
    "--session-id",
    "unpersisted-uuid",
  ]);

  await pi.emit("agent_start", {}, freshCtx);
  assert.deepEqual(reportCalls[2], [
    "agent",
    "report",
    "--state",
    "working",
    "--agent",
    "pi",
    "--session-id",
    "unpersisted-uuid",
  ]);

  await pi.emit("agent_end", {}, freshCtx);
  assert.deepEqual(reportCalls[3], [
    "agent",
    "report",
    "--state",
    "idle",
    "--agent",
    "pi",
    "--session-id",
    "unpersisted-uuid",
  ]);

  await pi.emit("session_shutdown", {}, freshCtx);
  assert.deepEqual(reportCalls[4], [
    "agent",
    "report",
    "--state",
    "idle",
    "--agent",
    "pi",
    "--session-id",
    "unpersisted-uuid",
  ]);

  // 3. Resumed session whose getSessionFile() exists on disk reports sessionId on session_start and session_shutdown.
  writeFileSync(persistedSessionFile, '{"role":"user","content":"hello"}\n');
  const persistedCtx = {
    hasUI: false,
    sessionManager: {
      getSessionId: () => "persisted-uuid",
      getSessionFile: () => persistedSessionFile,
    },
  };

  reportCalls.length = 0;
  await pi.emit("session_start", { reason: "resume" }, persistedCtx);
  assert.deepEqual(reportCalls[0], [
    "agent",
    "report",
    "--state",
    "idle",
    "--agent",
    "pi",
    "--session-id",
    "persisted-uuid",
  ]);

  await pi.emit("session_shutdown", {}, persistedCtx);
  assert.deepEqual(reportCalls[1], [
    "agent",
    "report",
    "--state",
    "idle",
    "--agent",
    "pi",
    "--session-id",
    "persisted-uuid",
  ]);
});


test("agent tool synthesis binds allowed roles and preserves structured question replies", async () => {
  const { buildAgentTools } = await import("../src/tools.ts");
  const calls = [];
  const client = { async runJson(args, timeout) {
    calls.push({ args, timeout });
    return { ok: true, data: { status: "question", pane_id: 81, question: "Which branch?" } };
  } };
  const tools = buildAgentTools(client, ["tester", "code-review"], ["writer"]);
  assert.deepEqual(tools.map(t => t.name), ["agent_call_tester", "agent_call_code-review", "agent_message", "agent_handoff"]);
  const result = await tools[0].execute("call", { task: "Verify", worktree: "feature/check" });
  assert.equal(result.details.status, "question");
  assert.deepEqual(calls[0].args, ["agent", "call", "tester", "--task", "Verify", "--worktree", "feature/check"]);
  await tools[2].execute("reply", { pane_id: 81, text: "alpha", wait: true });
  assert.deepEqual(calls[1].args, ["agent", "message", "81", "alpha", "--wait"]);
  const refused = await tools[3].execute("handoff", { role: "tester", task: "Continue" });
  assert.equal(refused.isError, true);
  assert.equal(calls.length, 2);
  await tools[3].execute("handoff", { role: "writer", task: "Continue" });
  assert.deepEqual(calls[2].args, ["agent", "handoff", "writer", "--task", "Continue"]);
});

test("session start synthesizes agent tools from the live pane role", async () => {
  const { pi, tools } = makeFakePiHarness();
  const calls = [];
  const runner = async (_bin, args) => {
    calls.push(args);
    let data = [];
    if (args[0] === "pane" && args[1] === "list") data = [{ id: 1, type: "terminal", context_id: 10, agent_role: "lead" }];
    if (args[0] === "agent" && args[1] === "show") data = { can_call: ["tester"], can_handoff: ["writer"] };
    return { exitCode: 0, stdout: JSON.stringify(data), stderr: "" };
  };
  createPiPlexiExtension({
    env: { PLEXI_RUNNING: "1", PLEXI_PANE_ID: "1", PLEXI_CONTEXT_ID: "10", PLEXI_SOCKET: "/tmp/plexi-alpha/notify.sock", PLEXI_CHANNEL: "alpha" },
    runner, checkSocketExists: () => true,
    homeDir: mkdtempSync(join(tmpdir(), "pi-agent-tools-")),
  })(pi);
  await pi.emit("session_start", {}, {});
  assert.equal(tools.has("agent_call_tester"), true);
  assert.equal(tools.has("agent_call_writer"), false);
  assert.equal(tools.has("agent_message"), true);
  assert.ok(calls.some(args => args.join(" ") === "agent show lead --json"));
  await pi.emit("session_shutdown", {}, {});
});
