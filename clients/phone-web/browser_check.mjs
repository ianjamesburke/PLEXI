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
const nonce = Date.now().toString(36);
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

  // Unique text per run so leftovers from earlier runs on the same server cannot satisfy a wait.
  const hello = `hello from the phone shell ${nonce}`;
  const doomed = `cancel this one ${nonce}`;
  const idOf = (text) => `[...document.querySelectorAll("#transcript li.user")].find((li) => li.textContent === ${JSON.stringify(text)})?.dataset.requestId`;
  const receiptFor = (text, state) => `[...document.querySelectorAll("#transcript li.receipt")]
    .some((li) => li.dataset.requestId === ${idOf(text)} && li.textContent === ${JSON.stringify(state)})`;

  await typeAndSend(hello);
  await waitFor(`document.getElementById("message").value === ""`, "draft cleared after accept");
  await waitFor(`[...document.querySelectorAll("#transcript li.stub_reply")]
    .some((li) => li.textContent === ${JSON.stringify("Stub echo: " + hello)})`, "stub echo");
  await waitFor(receiptFor(hello, "succeeded"), "succeeded receipt");
  await waitFor(`document.getElementById("cancel").disabled`, "cancel disabled once nothing is pending");
  console.log("AFTER SEND", JSON.stringify(await transcript()));

  await typeAndSend(doomed);
  await waitFor(`!!(${idOf(doomed)})`, "second turn in transcript");
  await waitFor(`!document.getElementById("cancel").disabled`, "cancel enabled");
  await evaluate(`document.getElementById("cancel").click()`);
  await waitFor(receiptFor(doomed, "cancelled"), "cancelled receipt for the second turn");
  await sleep(2000);
  const final = await transcript();
  if (final.some((line) => line.includes("Stub echo: " + doomed))) throw new Error("cancelled turn was still echoed");
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
