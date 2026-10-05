// Headless Chrome check of the phone shell against a running stub server.
// Emulates a phone viewport, types into the composer, sends, waits for the stub
// echo, then sends again and cancels before the echo lands.
//
// Run: node --experimental-websocket clients/phone-web/browser_check.mjs [http://127.0.0.1:8787] [screenshot.png]

import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const base = process.argv[2] ?? "http://127.0.0.1:8787";
const shotPath = process.argv[3];
const chrome = process.env.CHROME ?? "google-chrome";
const port = 9333;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const browser = spawn(chrome, [
  "--headless=new", "--no-sandbox", "--disable-gpu", `--remote-debugging-port=${port}`,
  `--user-data-dir=${mkdtempSync(join(tmpdir(), "phone-web-chrome-"))}`, "about:blank",
], { stdio: "ignore" });

async function target() {
  for (let i = 0; i < 50; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch { /* chrome still starting */ }
    await sleep(200);
  }
  throw new Error("chrome devtools endpoint never came up");
}

const ws = new WebSocket(await target());
await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
let nextId = 0;
const pending = new Map();
ws.onmessage = (msg) => {
  const data = JSON.parse(msg.data);
  if (data.id && pending.has(data.id)) {
    const { resolve, reject } = pending.get(data.id);
    pending.delete(data.id);
    data.error ? reject(new Error(JSON.stringify(data.error))) : resolve(data.result);
  }
};
const cdp = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++nextId;
  pending.set(id, { resolve, reject });
  ws.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression) =>
  (await cdp("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result.value;
async function waitFor(expression, label, ms = 8000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await evaluate(expression)) return;
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${label}`);
}
async function typeAndSend(text) {
  await evaluate(`document.getElementById("message").focus()`);
  await cdp("Input.insertText", { text });
  await evaluate(`document.getElementById("send").click()`);
}
const transcript = () => evaluate(
  `[...document.querySelectorAll("#transcript li")].map((li) => li.className + ": " + li.textContent)`);

let ok = false;
try {
  await cdp("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 3, mobile: true });
  await cdp("Emulation.setTouchEmulationEnabled", { enabled: true });
  await cdp("Page.enable");
  await cdp("Page.navigate", { url: base });
  await waitFor(`document.getElementById("connection")?.dataset.state === "online"`, "online state");

  const dom = await evaluate(`({
    connection: document.getElementById("connection").textContent,
    composer: !!document.querySelector("form#composer textarea#message"),
    composerLabel: document.querySelector('label[for="message"]')?.textContent,
    send: document.getElementById("send").textContent,
    cancel: document.getElementById("cancel").textContent,
    manifest: document.querySelector('link[rel="manifest"]')?.href,
    viewport: innerWidth + "x" + innerHeight,
  })`);
  console.log("DOM", JSON.stringify(dom));
  if (!dom.composer) throw new Error("composer missing from DOM");

  await typeAndSend("hello from the phone shell");
  await waitFor(`document.getElementById("message").value === ""`, "draft cleared after accept");
  await waitFor(`[...document.querySelectorAll("#transcript li.stub_reply")]
    .some((li) => li.textContent === "Stub echo: hello from the phone shell")`, "stub echo");
  console.log("AFTER SEND", JSON.stringify(await transcript()));

  await typeAndSend("cancel this one");
  await waitFor(`!document.getElementById("cancel").disabled`, "cancel enabled");
  await evaluate(`document.getElementById("cancel").click()`);
  await waitFor(`[...document.querySelectorAll("#transcript li.receipt")].some((li) => li.textContent === "cancelled")`, "cancelled receipt");
  await sleep(2000);
  const final = await transcript();
  if (final.some((line) => line.includes("Stub echo: cancel this one"))) throw new Error("cancelled turn was still echoed");
  console.log("AFTER CANCEL", JSON.stringify(final));

  if (shotPath) {
    const shot = await cdp("Page.captureScreenshot", { format: "png" });
    writeFileSync(shotPath, Buffer.from(shot.data, "base64"));
  }
  ok = true;
  console.log("PASS");
} catch (err) {
  console.error("FAIL", err.message);
} finally {
  ws.close();
  browser.kill();
  process.exit(ok ? 0 : 1);
}
