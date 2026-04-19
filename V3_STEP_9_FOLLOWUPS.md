# V3 Step 9 Follow-ups — Broker Work

**Context.** `V3_REFACTOR_PLAN.md` step 9 was scoped down in commit 2a4f049 to three items (env isolation, bold text, AppSpawned SDK hook). This file tracks the five broker / isolation items deferred from that step. Each is independently mergeable; each closes a specific hole before the v3.0 tag.

Current state on `v3` (HEAD: 96920a9). 74/74 Rust tests green. All 12 steps of the main plan shipped. Only this file blocks the v3.0 tag.

## Read before starting

- `V3_REFACTOR_PLAN.md` — the full 12-step plan. Steps 1–12 are all shipped.
- `DEV_LOG.md` — top two entries (step 9 partial, step 12) explain exactly what's wired and what isn't.
- `docs/specs/releases/plexi-v3.0.md §3` — PGAP wire format.
- `docs/specs/releases/plexi-v3.0.md §5` — isolation invariants (I-1 … I-10).

## The 5 deferred items

Ordered by blast radius. Do them in order; each step's ship gate is `cargo test --release` + `uv run pytest` + `scripts/smoke-test.sh` clean.

---

### 9a — Real HTTP broker

**Goal.** Replace the `StubNetService` in `src/host/services.rs` with a real blocking HTTP client. Route `DrawCommand::HttpRequest` in `src/process_app/routing.rs` through `services.net.http_*` and reply with `PlexiEvent::HttpResponse`. Kill the custom `http_mocks` machinery in `src/pgap_test_harness.rs` — tests move to `MockNetService` (already built in step 5).

**Touches.**

- `Cargo.toml`: add `ureq = { version = "2", default-features = false, features = ["tls"] }` (pure-Rust blocking client, ~3 deps). `reqwest` is fine too but pulls tokio; `ureq` is the lighter choice.
- `src/host/services.rs`: rename `StubNetService` → `UreqNetService`, implement against `ureq::get(url).call()`. Extend the trait:
  ```rust
  pub trait NetService: Send {
      fn http(&self, method: &str, url: &str, headers: &HashMap<String, String>, body: Option<&str>) -> HttpResponse;
  }
  ```
  Keep `http_get` as a default method wrapping `http("GET", url, &Default::default(), None)` so the existing mock tests don't break.
- `src/process_app/routing.rs`: the `DrawCommand::HttpRequest { request_id, url, method, headers, body }` arm currently returns 403 on denial and logs a stub warning otherwise. Replace the stub with:
  ```
  let resp = services.net.http(&method, &url, &headers, body.as_deref());
  self.outbound_events.push_back(PlexiEvent::HttpResponse {
      request_id, status: resp.status, body: resp.body, error: resp.error,
  });
  ```
  This requires threading `&mut HostServices` into `ProcessApp::route_command`. ProcessApp is instantiated per-pane; HostServices lives on PlexiApp. Cleanest seam: `ProcessApp` holds a `Arc<Mutex<HostServices>>` clone. Alternative: pass `&mut HostServices` at each `pump` call site.
- `src/pgap_test_harness.rs`: delete `http_mocks: HashMap<String, String>`, `reply_http_request`, `pre_drain_http`, `mock_http`. Replace every test that used `h.mock_http(...)` with `HostServices::mock()` wired via `MockNetService::with(url, body)`. The Layer-1 wikipedia test (`layer1_wikipedia_inject_results_renders`) no longer needs HTTP at all (it uses inject_state); the harness methods can just disappear.

**Acceptance.** New test `layer1_wikipedia_http_broker_end_to_end`: spawn wikipedia, inject `MockNetService` with the search URL → body, press "R"/"u"/"s"/"t"/Enter, assert results render. Without the custom `http_mocks` code path.

**Breaks if:** `wikipedia` hangs forever on search (broker regression). `ureq` features pull `tokio` transitively (unnecessary weight). A test that used `h.mock_http` silently compiles but never actually mocks.

**Effort.** ~3 hours.

---

### 9b — PipeSend peer routing

**Goal.** `PipeSend` currently round-trips the payload back to the sending app via `outbound_events.push_back(PlexiEvent::PipeMessage { ... })` — which is wrong. A pipe peer routing registry is needed so app A's `PipeSend { pipe_id: "foo" }` delivers to every *other* app that opened "foo" with `direction: "in"` or `"duplex"`.

**Touches.**

- `src/typed_pipes.rs`: `TypedPipeRegistry::send_json` already drops the payload on the floor (TODO at `routing.rs:303`). Extend the registry to track peer list per pipe_id:
  ```rust
  peers: HashMap<String, Vec<u64>>, // pipe_id → set of pane_ids subscribed as reader
  ```
  `open_json(pipe_id, direction, owning_pane_id)` inserts `owning_pane_id` into peers when direction is `In` or `Duplex`. `send_json(pipe_id, payload)` looks up the subscribers, returns them to the caller.
- `src/process_app/routing.rs::PipeSend`: after the pipe registry call, deliver `PlexiEvent::PipeMessage` to each peer pane instead of back to self. Needs a handle to dispatch events cross-pane — cleanest: `pending_commands.push(AppCommand::DeliverPipeMessage { pane_ids, pipe_id, payload })` and let PlexiApp fan out on its next pump.
- `src/app_trait.rs`: add `AppCommand::DeliverPipeMessage` variant.
- `src/app/mod.rs`: handle `AppCommand::DeliverPipeMessage` by queueing `PlexiEvent::PipeMessage` on each target pane's `ProcessApp::outbound_events`.
- Update `PipeOpen` call in routing.rs to pass `self.pane_id` (currently not threaded — check whether `ProcessApp` knows its own `pane_id`; if not, this is a small prerequisite refactor).

**Acceptance.** New test `layer1_pipe_peer_routing`: two apps open the same pipe_id (one as `out`, one as `in`). Sender sends `{"msg": "hello"}`. Assert the receiver's `on_pipe_message` hook fires within the next render cycle. Sender does NOT receive its own message back.

**Breaks if:** An app sending on a pipe it alone subscribes to leaks its own `PipeMessage` back. Opening a pipe twice in the same pane counts as two peers.

**Effort.** ~4 hours. This is the most involved 9x item because of the cross-pane event plumbing.

---

### 9c — RunUpdate round-trip on RunComplete

**Goal.** When an app emits `DrawCommand::RunComplete { run_id, result }`, the host currently just logs and calls `self.run_registry.complete(&run_id)`. Spec requires the host to reply to the *originating* app with `PlexiEvent::RunUpdate { run_id, status: "completed", payload: result }`.

**Touches.**

- `src/runs.rs`: `RunRegistry` already tracks `app_id` per run. Add a `lookup_originator(run_id) -> Option<String>` method.
- `src/process_app/routing.rs::RunComplete`: after `self.run_registry.complete(&run_id)`, look up the originator. If it matches `self.type_id`, enqueue `PlexiEvent::RunUpdate` directly. If it's a different app, push an `AppCommand::DeliverRunUpdate` that `PlexiApp` fans out (same pattern as PipeSend peer routing in 9b).

**Acceptance.** New Layer-1 test spawning an app that emits `RunGet` → `RunComplete` and asserts `on_run_update` fires with `status == "completed"`. Requires an example app to drive it; `quick-note` can grow a `run run my-intent` shortcut for the test, or a minimal `run-echo` fixture app.

**Breaks if:** An agent pane waits forever for `RunUpdate { status: "completed" }` after the run it owns finishes.

**Effort.** ~2 hours. Depends on 9b landing first (reuses the cross-pane event delivery plumbing).

---

### 9d — Image / Video / Audio broker plumbing

**Goal.** Currently routing.rs has placeholder branches for `HttpRequest`/`AudioPlay`/`AudioCapture` (capability-checked, then log-and-drop). The spec requires actual brokers:

- `DrawCommand::Image { src, x, y, w, h, fit }` — host reads the file (FsService), decodes via `image` crate, uploads to an egui texture, paints it. `src` is either a workspace-scoped path or a `data:` URL.
- `DrawCommand::VideoPlayer { source, rect, state }` — spawn a decoder thread (deferred beyond v3.0 in my opinion; re-add `examples/video-player/` only if/when this lands).
- `DrawCommand::AudioPlay { source | pipe_id, volume, state }` — wire up `rodio` playback.
- `DrawCommand::AudioCapture { pipe_id, sample_rate, buffer_size }` — host mic → PCM → binary pipe (typed_pipes.rs already supports binary mode).
- `DrawCommand::AudioMeter { rect, pipe_id }` — read amplitude from binary pipe, paint a meter.

**Recommendation — split this further:**

- **9d-image.** Lowest risk, highest leverage. Pure-Rust, no thread, no hardware. `examples/quick-note` could grow an image attachment preview as a smoke test.
- **9d-audio-capture.** Host mic broker via `cpal` (already a transitive dep of `rodio`). Unblocks `examples/audio-recorder`.
- **9d-audio-play.** Uses `rodio` (already in Cargo.toml). Small.
- **9d-video.** Defer. Decoder crate choice is a rabbit hole (ffmpeg-the-sys vs wgpu-video vs gstreamer).

**Touches.** Per subitem; concentrated in `src/process_app/mod.rs` (new `pending_frame` render arms) and `src/process_app/routing.rs` (new broker dispatch).

**Acceptance.** Each subitem ships an example app test that exercises the broker end-to-end via `PLEXI_AUDIO=mock://` / image fixture files.

**Breaks if:** `quick-note` tries to show an image and renders nothing. `audio-recorder` binary pipe stops delivering PCM.

**Effort.** 9d-image: ~3h. 9d-audio-capture: ~4h. 9d-audio-play: ~2h. 9d-video: out of scope for v3.0.

---

### 9e — FD CLOEXEC audit

**Goal.** Spec I-7: subprocess apps must not inherit any FD other than stdio. Today `ProcessApp::launch` calls `.env_clear()` (step 9) but doesn't audit FDs. Every `UnixListener::bind` in the host must close-on-exec; subprocesses must see `/proc/self/fd` (or `lsof -p`) = {0, 1, 2}.

**Touches.**

- `src/typed_pipes.rs`: every `UnixListener::bind(path)` call. Rust's stdlib sets `SOCK_CLOEXEC` by default on Linux (since 1.80) but NOT on macOS. Explicit `nix::fcntl::fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))` after bind is the portable fix. Add `nix` to `Cargo.toml` with feature `fcntl`.
- `src/event_log.rs`: the writer-thread FileWriter. Check + fix.
- Any other long-lived FD in the host. Grep `File::create`, `OpenOptions::new().open`, `UnixListener::bind`, `UnixStream::connect`.

**Acceptance.** New `fd_inheritance_test` in `pgap_test_harness.rs`: spawn snake (or any app), read its `/proc/self/fd` (Linux) or `lsof -p <pid>` (macOS) via subprocess, assert only fds 0/1/2 + any ephemeral tempfiles. On macOS, the `lsof` output should not list any host `UnixListener` socket.

**Breaks if:** `lsof -p $(pgrep snake.py)` shows the host's `events.jsonl` FD or a `/tmp/plexi-*.sock` UnixListener.

**Effort.** ~3 hours.

---

## Overall order + ship gate

```
9a (HTTP broker)       ← independent, easiest ship
9e (CLOEXEC)           ← independent, no deps
9b (Pipe peer routing) ← enables 9c
9c (RunUpdate)         ← needs 9b's cross-pane delivery
9d-image               ← independent
9d-audio-capture       ← independent
9d-audio-play          ← independent
```

9a + 9e can run in parallel (two fresh sessions). 9b before 9c. 9d subitems all independent.

**Ship gate per step:**
```
cargo test --release
uv run pytest -q
just install-v3     # runs scripts/smoke-test.sh
```

All three green = mergeable.

## After all 9x items ship

1. Tag `v3.0.0` on `main` (not `v3` — cut `v3` into `main` via merge PR).
2. Update `alpha` tag to `v2-last`, stop landing on it.
3. Delete this file.

## Non-goals for this follow-up

- No new pane variants (I-5 freeze holds).
- No WASM runtime (deferred to v3.1+; the I-1 test in step 12 gates regression).
- No SpacetimeDB sync (deferred; `effects.jsonl` bus from step 6 is the hook).
- No spatial canvas / fractal PGAP (explicit v3.0 anti-goals).

If any of these come up mid-execution, file under `.plexi/backlog/` and return to the step.

---

**End of follow-up plan.** Start with 9a.
