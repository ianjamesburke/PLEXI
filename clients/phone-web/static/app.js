// Plexi phone shell. Talks to the local stub turn API on the same origin.
// No credentials are stored; the draft stays in the composer until the server accepts it.
// A relay page seals every body. The desktop public key arrives in the URL
// fragment and is never sent to the relay.

import { keyFingerprint, openReply, rememberDesktopKey, sealTurn } from "./e2e.js";

const POLL_MS = 1000;
const TERMINAL = new Set(["succeeded", "failed", "cancelled", "expired", "waiting_for_permission", "waiting_on_desktop"]);

function receiptLabel(state) {
  if (state === "waiting_for_permission" || state === "waiting_on_desktop") return "waiting on desktop";
  return state;
}

function connectionFromStatus(body) {
  mode = body.mode || "";
  if (body.host === "desktop_offline" || body.desktop === "offline") return ["offline", "desktop offline"];
  if (body.host === "not_connected") return ["online", "Online · local stub, no host"];
  if (mode === "relay" && !desktopKey) return ["offline", "Open the desktop QR to load the key"];
  return ["online", "Online"];
}

const els = {
  connection: document.getElementById("connection"),
  needsSection: document.getElementById("needs-you"),
  needs: document.getElementById("needs-list"),
  transcript: document.getElementById("transcript"),
  needs: document.getElementById("needs-list"),
  form: document.getElementById("composer"),
  message: document.getElementById("message"),
  send: document.getElementById("send"),
  cancel: document.getElementById("cancel"),
  desktop: document.getElementById("desktop"),
};

const DESKTOP_KEY = "plexiPhoneJoinDesktop";
els.desktop.checked = localStorage.getItem(DESKTOP_KEY) === "1";
els.desktop.addEventListener("change", () => {
  localStorage.setItem(DESKTOP_KEY, els.desktop.checked ? "1" : "0");
});

let cursor = 0;
let online = false;
let mode = "";
const desktopKey = rememberDesktopKey();
const sentKey = "plexiSentPlain";
function rememberSent(id, text) {
  const sent = JSON.parse(sessionStorage.getItem(sentKey) || "{}");
  sent[id] = text;
  sessionStorage.setItem(sentKey, JSON.stringify(sent));
}
function sentText(id) {
  return (JSON.parse(sessionStorage.getItem(sentKey) || "{}"))[id];
}
if (desktopKey) {
  const line = document.createElement("p");
  line.id = "key-fingerprint";
  line.textContent = "Key fingerprint " + keyFingerprint(desktopKey);
  document.querySelector("header").append(line);
}
const urlToken = new URLSearchParams(location.search).get("token");
if (urlToken) {
  sessionStorage.setItem("plexiPhoneToken", urlToken);
  history.replaceState({}, "", location.pathname + location.hash);
}
const token = sessionStorage.getItem("plexiPhoneToken");
function apiHeaders(extra = {}) { return token ? { ...extra, Authorization: `Bearer ${token}` } : extra; }
let sending = false;
// Turns sent from this page that have not reached a terminal state, oldest first.
const activeRequestIds = [];

async function refreshNeeds() {
  const res = await fetch("/api/needs-you", { cache: "no-store", headers: apiHeaders() });
  if (!res.ok) return;
  const body = await res.json();
  els.needs.replaceChildren();
  for (const item of body.items || []) {
    const li = document.createElement("li");
    const text = document.createElement("p");
    text.textContent = item.summary || item.kind || item.id;
    li.append(text);
    const mayApprove = item.kind === "question" || item.kind === "blocked_run";
    for (const decision of mayApprove ? ["approve", "deny"] : ["deny"]) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = decision;
      button.textContent = decision === "approve" ? "Approve" : "Deny";
      button.addEventListener("click", () => resolveNeeds(item.id, decision));
      li.append(button);
    }
    els.needs.append(li);
  }
}

async function resolveNeeds(id, decision) {
  const res = await fetch(`/api/needs-you/${encodeURIComponent(id)}/resolve`, {
    method: "POST",
    headers: apiHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ decision }),
  });
  if (!res.ok && res.status !== 409) {
    console.warn("needs-you resolve failed", res.status);
  }
  await refreshNeeds();
}

function requestId() {
  // crypto.randomUUID is only exposed in secure contexts; LAN http is not one.
  if (crypto.randomUUID) return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function setConnection(state, label) {
  online = state === "online";
  els.connection.dataset.state = state;
  els.connection.textContent = label;
  syncControls();
}

function syncControls() {
  els.send.disabled = !online || sending;
  els.cancel.disabled = !online || activeRequestIds.length === 0;
}

function render(event) {
  const li = document.createElement("li");
  li.className = event.kind;
  li.dataset.requestId = event.request_id;
  if (event.kind === "receipt") {
    const state = document.createElement("div");
    state.textContent = receiptLabel(event.state);
    li.append(state);
    const note = event.status || event.error;
    if (note) {
      const error = document.createElement("small");
      error.textContent = note.slice(0, 240);
      li.append(error);
    }
  } else {
    let text = sentText(event.request_id) || event.text || "";
    if (!sentText(event.request_id) && desktopKey && event.kind === "assistant_reply" && event.text) {
      try {
        text = openReply(text, event.request_id).text || "";
      } catch (err) {
        text = "sealed message rejected";
      }
    }
    li.textContent = text;
  }
  const scroller = els.transcript.parentElement;
  const pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40;
  els.transcript.append(li);
  if (pinned) scroller.scrollTop = scroller.scrollHeight;
  if (event.kind === "receipt" && TERMINAL.has(event.state)) {
    const i = activeRequestIds.indexOf(event.request_id);
    if (i !== -1) {
      activeRequestIds.splice(i, 1);
      syncControls();
    }
  }
}

function phoneCanApprove(item) {
  return item.phone_can_approve === true && item.kind !== "approval_click";
}

async function refreshNeeds() {
  if (!els.needs) return;
  const res = await fetch("/api/needs-you", { cache: "no-store", credentials: "same-origin", headers: apiHeaders() });
  if (!res.ok) return;
  const body = await res.json();
  const items = body.items || [];
  els.needs.replaceChildren();
  if (els.needsSection) els.needsSection.hidden = items.length === 0;
  for (const item of items) {
    const li = document.createElement("li");
    const text = document.createElement("p");
    text.textContent = item.summary || item.kind || item.id;
    li.append(text);
    const canApprove = phoneCanApprove(item);
    if (!canApprove) {
      const note = document.createElement("small");
      note.textContent = "waiting on desktop";
      li.append(note);
    }
    const decisions = canApprove ? ["approve", "deny"] : ["deny"];
    for (const decision of decisions) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = decision;
      button.textContent = decision === "approve" ? "Answer" : "Deny";
      button.addEventListener("click", () => resolveNeeds(item.id, decision));
      li.append(button);
    }
    els.needs.append(li);
  }
}

async function resolveNeeds(id, decision) {
  const res = await fetch(`/api/needs-you/${encodeURIComponent(id)}/resolve`, {
    method: "POST",
    credentials: "same-origin",
    headers: apiHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ decision }),
  });
  if (!res.ok && res.status !== 409) {
    console.warn("needs-you resolve failed", res.status);
  }
  await refreshNeeds();
}

async function poll() {
  try {
    const status = await fetch("/api/status", { cache: "no-store", credentials: "same-origin", headers: apiHeaders() });
    if (!status.ok) throw new Error(`status ${status.status}`);
    const body = await status.json();
    const [state, label] = connectionFromStatus(body);
    setConnection(state, label);
    await refreshNeeds();
    const res = await fetch(`/api/conversation?after=${cursor}`, { cache: "no-store", credentials: "same-origin", headers: apiHeaders() });
    if (!res.ok) throw new Error(`conversation ${res.status}`);
    const page = await res.json();
    page.events.forEach(render);
    cursor = page.cursor;
    await refreshNeeds();
  } catch (err) {
    console.warn("phone shell poll failed", err);
    setConnection("offline", "Offline · drafts are not sent");
  } finally {
    setTimeout(poll, POLL_MS);
  }
}

els.form.addEventListener("submit", async (e) => {
  e.preventDefault();
  const text = els.message.value.trim();
  if (!text || !online || sending) return;
  if (mode === "relay" && !desktopKey) {
    setConnection("offline", "Open the desktop QR to load the key");
    return;
  }
  const id = requestId();
  sending = true;
  syncControls();
  try {
    const joinDesktop = els.desktop.checked;
    const content = mode === "relay"
      ? [{ type: "sealed", body: sealTurn(text, id, joinDesktop) }]
      : [{ type: "text", text }];
    const res = await fetch("/api/turns", {
      method: "POST",
      credentials: "same-origin",
      headers: apiHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify({
        schema_version: 1,
        request_id: id,
        conversation_id: "local-stub",
        join_desktop: mode === "relay" ? false : joinDesktop,
        content,
      }),
    });
    const receipt = await res.json();
    if (!res.ok) {
      if (receipt.error === "desktop_offline") {
        setConnection("offline", "desktop offline");
        return;
      }
      throw new Error(receipt.error || `turn ${res.status}`);
    }
    if (!TERMINAL.has(receipt.state) && !activeRequestIds.includes(id)) activeRequestIds.push(id);
    if (mode === "relay") rememberSent(id, text);
    els.message.value = "";
  } catch (err) {
    console.warn("phone shell send failed", err);
    setConnection("offline", "Send failed · draft kept");
  } finally {
    sending = false;
    syncControls();
  }
});

els.message.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    els.form.requestSubmit();
  }
});

els.cancel.addEventListener("click", async () => {
  // Cancel addresses the newest unfinished turn by its exact request id.
  const id = activeRequestIds.at(-1);
  if (!id) return;
  try {
    const res = await fetch(`/api/turns/${encodeURIComponent(id)}/cancel`, { method: "POST", credentials: "same-origin", headers: apiHeaders() });
    if (!res.ok) throw new Error(`cancel ${res.status}`);
  } catch (err) {
    console.warn("phone shell cancel failed", err);
  }
});

syncControls();
poll();
