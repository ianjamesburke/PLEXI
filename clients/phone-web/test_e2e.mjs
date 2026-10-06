// Proves the phone bundle opens a Python seal and Python opens a phone seal.
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  b64uDecode,
  b64uEncode,
  generateIdentity,
  keyFingerprint,
  openData,
  openHandshake,
  sealData,
} from "./static/e2e.js";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const desk = generateIdentity();
const desktopPub = b64uEncode(desk.publicKey);
const python = spawnSync(
  "python3",
  ["services/relay/phone_crypto.py", "handshake-for", "--", desktopPub],
  { cwd: root, encoding: "utf8" },
);
if (python.status !== 0) {
  console.error(python.stderr);
  process.exit(1);
}
const value = JSON.parse(python.stdout);
const opened = openHandshake(desk.privateKey, desk.publicKey, b64uDecode(value.handshake));
if (opened.payload.text !== "hello-vector" || opened.payload.join_desktop !== false) {
  throw new Error("handshake plaintext mismatch");
}
const data = openData(opened.key, b64uDecode(value.data), value.request_id, 0, 1);
if (data.payload.text !== "second-vector" || data.payload.join_desktop !== true) {
  throw new Error("data plaintext mismatch");
}
let replayed = false;
try {
  openData(opened.key, b64uDecode(value.data), value.request_id, data.counter, 1);
} catch (error) {
  replayed = error.message === "replay";
}
if (!replayed) throw new Error("replay was accepted");
const tampered = b64uDecode(value.data);
tampered[tampered.length - 1] ^= 0x01;
let rejected = false;
try {
  openData(opened.key, tampered, value.request_id, 0, 1);
} catch (error) {
  rejected = error.message === "tamper";
}
if (!rejected) throw new Error("tampered body was accepted");
if (keyFingerprint(desk.publicKey).length !== 16) throw new Error("fingerprint length");

const reply = b64uEncode(sealData(opened.key, 2, 1, value.request_id, "from-js", false));
const back = spawnSync(
  "python3",
  [
    "services/relay/phone_crypto.py",
    "open-reply",
    "--desktop-pub",
    desktopPub,
    "--phone-priv",
    value.phone_priv,
    "--request-id",
    value.request_id,
    "--body",
    reply,
  ],
  { cwd: root, encoding: "utf8" },
);
if (back.status !== 0) {
  console.error(back.stderr);
  process.exit(1);
}
const body = JSON.parse(back.stdout);
if (body.text !== "from-js") throw new Error(`python opened ${body.text}`);
console.log("phone seal interop ok");
