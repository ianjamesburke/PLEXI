// Phone-side seals. Bundled to static/e2e.js.
// X25519, HKDF-SHA256, and ChaCha20-Poly1305 are @noble. This file only
// lays out the envelope the desktop opens in src/cli/relay_crypto.rs.

import { x25519 } from "@noble/curves/ed25519.js";
import { chacha20poly1305 } from "@noble/ciphers/chacha.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";

const MAGIC = new Uint8Array([0x50, 0x31]); // P1
const INFO = new TextEncoder().encode("plexi-phone-e2e-v1");
const KIND_HANDSHAKE = 0x01;
const KIND_DATA = 0x02;
const DIR_PHONE = 1;
const DIR_DESKTOP = 2;

const DESKTOP_PUB = "plexiDesktopPub";
const PHONE_PRIV = "plexiPhonePriv";
const SEND = "plexiSealSend";
const RECV = "plexiSealRecv";
const HANDSHAKEN = "plexiSealHandshake";
const BOUND = "plexiSealDesktop";
const OPENED = "plexiSealOpened";

export function b64uEncode(bytes) {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function b64uDecode(text) {
  const pad = "=".repeat((4 - (text.length % 4)) % 4);
  const binary = atob(text.replaceAll("-", "+").replaceAll("_", "/") + pad);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

export function generateIdentity() {
  const privateKey = x25519.utils.randomSecretKey();
  return { privateKey, publicKey: x25519.getPublicKey(privateKey) };
}

export function keyFingerprint(publicKey) {
  const digest = sha256(publicKey);
  return [...digest.subarray(0, 8)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function rememberDesktopKey() {
  if (typeof location === "undefined") return null;
  const params = new URLSearchParams(location.hash.replace(/^#/, ""));
  const token = params.get("k");
  if (token) sessionStorage.setItem(DESKTOP_PUB, token);
  const stored = sessionStorage.getItem(DESKTOP_PUB);
  if (!stored) return null;
  const bytes = b64uDecode(stored);
  return bytes.length === 32 ? bytes : null;
}

function concat(...parts) {
  const length = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

function u8(value) {
  return new Uint8Array([value]);
}

function counterBytes(counter) {
  const out = new Uint8Array(8);
  let rest = BigInt(counter);
  for (let i = 0; i < 8; i++) {
    out[i] = Number(rest & 0xffn);
    rest >>= 8n;
  }
  return out;
}

function readCounter(bytes) {
  let value = 0n;
  for (let i = 7; i >= 0; i--) value = (value << 8n) | BigInt(bytes[i]);
  return Number(value);
}

function nonce(direction, counter) {
  return concat(u8(direction), new Uint8Array(3), counterBytes(counter));
}

function plainBytes(text, joinDesktop, error) {
  const payload = { v: 1, text, join_desktop: !!joinDesktop };
  if (error) payload.error = error;
  return new TextEncoder().encode(JSON.stringify(payload));
}

function parsePlain(bytes) {
  const payload = JSON.parse(new TextDecoder().decode(bytes));
  if (!payload || payload.v !== 1 || typeof payload.text !== "string" || typeof payload.join_desktop !== "boolean") {
    throw new Error("tamper");
  }
  return payload;
}

function sessionKey(shared, desktopPub, phonePub) {
  return hkdf(sha256, shared, concat(desktopPub, phonePub), INFO, 32);
}

function seal(key, nonceBytes, plaintext, aad) {
  return chacha20poly1305(key, nonceBytes, aad).encrypt(plaintext);
}

function open(key, nonceBytes, ciphertext, aad) {
  try {
    return chacha20poly1305(key, nonceBytes, aad).decrypt(ciphertext);
  } catch {
    throw new Error("tamper");
  }
}

export function deriveSessionKey(phonePriv, desktopPub) {
  const phonePub = x25519.getPublicKey(phonePriv);
  const shared = x25519.getSharedSecret(phonePriv, desktopPub);
  return { phonePub, key: sessionKey(shared, desktopPub, phonePub) };
}

export function sealHandshake(key, phonePub, desktopPub, text, joinDesktop) {
  const aad = concat(MAGIC, u8(KIND_HANDSHAKE), phonePub, desktopPub);
  const body = seal(key, nonce(DIR_PHONE, 0), plainBytes(text, joinDesktop), aad);
  return concat(MAGIC, u8(KIND_HANDSHAKE), phonePub, body);
}

export function sealData(key, direction, counter, requestId, text, joinDesktop, error) {
  if (counter === 0) throw new Error("replay");
  const aad = concat(MAGIC, u8(KIND_DATA), u8(direction), counterBytes(counter), new TextEncoder().encode(requestId));
  const body = seal(key, nonce(direction, counter), plainBytes(text, joinDesktop, error), aad);
  return concat(MAGIC, u8(KIND_DATA), u8(direction), counterBytes(counter), body);
}

export function openData(key, raw, requestId, lastCounter, expectDirection) {
  if (raw.length < 12 + 16 || raw[0] !== MAGIC[0] || raw[1] !== MAGIC[1] || raw[2] !== KIND_DATA) {
    throw new Error("malformed");
  }
  const direction = raw[3];
  const counter = readCounter(raw.subarray(4, 12));
  if (direction !== expectDirection) throw new Error("tamper");
  if (counter === 0 || counter <= lastCounter) throw new Error("replay");
  const aad = concat(MAGIC, u8(KIND_DATA), u8(direction), counterBytes(counter), new TextEncoder().encode(requestId));
  const payload = parsePlain(open(key, nonce(direction, counter), raw.subarray(12), aad));
  return { payload, counter };
}

export function openHandshake(desktopPriv, desktopPub, raw) {
  if (raw.length < 3 + 32 + 16 || raw[0] !== MAGIC[0] || raw[1] !== MAGIC[1] || raw[2] !== KIND_HANDSHAKE) {
    throw new Error("malformed");
  }
  const phonePub = raw.subarray(3, 35);
  const shared = x25519.getSharedSecret(desktopPriv, phonePub);
  const key = sessionKey(shared, desktopPub, phonePub);
  const aad = concat(MAGIC, u8(KIND_HANDSHAKE), phonePub, desktopPub);
  const payload = parsePlain(open(key, nonce(DIR_PHONE, 0), raw.subarray(35), aad));
  return { payload, key, phonePub };
}

function loadState() {
  const desktop = sessionStorage.getItem(DESKTOP_PUB);
  if (!desktop) return null;
  if (sessionStorage.getItem(BOUND) !== desktop) {
    sessionStorage.removeItem(PHONE_PRIV);
    sessionStorage.setItem(SEND, "0");
    sessionStorage.setItem(RECV, "0");
    sessionStorage.setItem(HANDSHAKEN, "0");
    sessionStorage.setItem(OPENED, "{}");
    sessionStorage.setItem(BOUND, desktop);
  }
  let phonePriv = sessionStorage.getItem(PHONE_PRIV);
  if (!phonePriv) {
    phonePriv = b64uEncode(x25519.utils.randomSecretKey());
    sessionStorage.setItem(PHONE_PRIV, phonePriv);
  }
  const desktopPub = b64uDecode(desktop);
  const priv = b64uDecode(phonePriv);
  const derived = deriveSessionKey(priv, desktopPub);
  return {
    desktopPub,
    private: priv,
    public: derived.phonePub,
    key: derived.key,
    send: Number(sessionStorage.getItem(SEND) || "0"),
    recv: Number(sessionStorage.getItem(RECV) || "0"),
    handshook: sessionStorage.getItem(HANDSHAKEN) === "1",
    opened: JSON.parse(sessionStorage.getItem(OPENED) || "{}"),
  };
}

function saveState(state) {
  sessionStorage.setItem(SEND, String(state.send));
  sessionStorage.setItem(RECV, String(state.recv));
  sessionStorage.setItem(HANDSHAKEN, state.handshook ? "1" : "0");
  sessionStorage.setItem(OPENED, JSON.stringify(state.opened));
}

export function sealTurn(text, requestId, joinDesktop) {
  const state = loadState();
  if (!state) throw new Error("missing_desktop_key");
  let raw;
  if (!state.handshook) {
    raw = sealHandshake(state.key, state.public, state.desktopPub, text, joinDesktop);
    state.handshook = true;
  } else {
    state.send += 1;
    raw = sealData(state.key, DIR_PHONE, state.send, requestId, text, joinDesktop);
  }
  saveState(state);
  return b64uEncode(raw);
}

export function openReply(body, requestId) {
  const state = loadState();
  if (!state) throw new Error("missing_desktop_key");
  if (Object.prototype.hasOwnProperty.call(state.opened, requestId)) {
    return { text: state.opened[requestId], join_desktop: false };
  }
  const opened = openData(state.key, b64uDecode(body), requestId, state.recv, DIR_DESKTOP);
  state.recv = opened.counter;
  state.opened[requestId] = opened.payload.text;
  saveState(state);
  return opened.payload;
}
