// Plexi phone shell. Talks to the local stub turn API on the same origin.
// No credentials are stored; the draft stays in the composer until the server accepts it.

const POLL_MS = 1000;
const TERMINAL = new Set(["succeeded", "failed", "cancelled", "expired"]);

const els = {
  connection: document.getElementById("connection"),
  transcript: document.getElementById("transcript"),
  form: document.getElementById("composer"),
  message: document.getElementById("message"),
  send: document.getElementById("send"),
  cancel: document.getElementById("cancel"),
};

let cursor = 0;
let online = false;
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
    state.textContent = event.state;
    li.append(state);
    if (event.error) {
      const error = document.createElement("small");
      error.textContent = event.error.slice(0, 240);
      li.append(error);
    }
  } else {
    li.textContent = event.text;
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

async function poll() {
  try {
    if (!online) {
      const status = await fetch("/api/status", { cache: "no-store", headers: apiHeaders() });
      if (!status.ok) throw new Error(`status ${status.status}`);
      const body = await status.json();
      setConnection("online", body.host === "not_connected" ? "Online · local stub, no host" : "Online");
    }
    const res = await fetch(`/api/conversation?after=${cursor}`, { cache: "no-store", headers: apiHeaders() });
    if (!res.ok) throw new Error(`conversation ${res.status}`);
    const page = await res.json();
    page.events.forEach(render);
    cursor = page.cursor;
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
  const id = requestId();
  sending = true;
  syncControls();
  try {
    const res = await fetch("/api/turns", {
      method: "POST",
      headers: apiHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify({
        schema_version: 1,
        request_id: id,
        conversation_id: "local-stub",
        content: [{ type: "text", text }],
      }),
    });
    const receipt = await res.json();
    if (!res.ok) throw new Error(receipt.error || `turn ${res.status}`);
    if (!TERMINAL.has(receipt.state) && !activeRequestIds.includes(id)) activeRequestIds.push(id);
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
    const res = await fetch(`/api/turns/${encodeURIComponent(id)}/cancel`, { method: "POST", headers: apiHeaders() });
    if (!res.ok) throw new Error(`cancel ${res.status}`);
  } catch (err) {
    console.warn("phone shell cancel failed", err);
  }
});

syncControls();
poll();
