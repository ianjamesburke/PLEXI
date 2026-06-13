# PRM: CLI-Native PGAP Apps + Board/Console Primitives

Status: proposal / planning source for "a CLI ships its own Plexi UI."
Last updated: 2026-06-13.
Reference implementation: the **stint board** app, shipped inside the `stint` repo.
Tracked in `.stint` as task **`0184`** (`status: backlog`, sprint `s32`); split into per-milestone tasks when promoted.

This PRM defines how an arbitrary installed CLI tool can ship a **bespoke, beautiful Plexi app** that any Plexi user opens with one command — not the generic auto-generated form UI. It is demonstrated end-to-end by a keyboard-driven, drag-and-drop **stint sprint board** that lives in the `stint` source tree.

It spans **two repos**:

- **PLEXI** (this repo): protocol/SDK/host changes — two new UI primitives, a routing fix, a capability split, and SDK ergonomics.
- **stint** (`~/Documents/GitHub/stint`): a `--plexi` descriptor, new non-interactive mutation commands, `--json` reads, the bundled PGAP app, and embed/extract distribution.

It supersedes the older "CLI-backed app = auto-renderer + linked terminal" framing for the case where a CLI wants to ship a *custom* app. The auto-renderer remains the fallback for CLIs that ship no app of their own.

---

## 1. Problem & Vision

Today a CLI integrates with Plexi in exactly one way: `plexi app open --cli <bin>` builds a **descriptor** (from `<bin> --plexi`, a registry, or a `--help` crawl) and feeds it to a builtin **auto-renderer** (`cli-renderer`) that draws a generic command list + argument forms and pipes execution to a **linked terminal pane**. That is great for "wrap any CLI for free," but it can never be a *designed* experience. You cannot build a kanban board out of an auto-generated form.

We want the other half: **a CLI author ships a hand-built Plexi app inside their own repo, and `plexi app open <cli>` launches it.** The canonical proof is `stint`:

> A beautiful, keyboard-driven sprint board. Tasks are cards; sprints are columns. You move a task between sprints by dragging it or with a keystroke. You search tasks, filter by area, and watch the underlying `stint` commands execute in an in-app console — no separate terminal pane.

This is the same UX class as the `github-issues` Core 9 app (`apps/github-issues/main.py`), but embedded in and distributed by the `stint` CLI, and richer (a true board with drag, not a single list).

### What this is NOT

- Not a replacement for the auto-renderer. CLIs that ship no app still get the generic UI.
- Not Python sandboxing. A CLI-native app is a reviewed native process, same trust model as every PGAP app (see `docs/prm/app-framework-marketplace.md` §Trust).
- Not a marketplace feature. Distribution rides the CLI's own install (`cargo install`, `brew`, etc.), not the Plexi registry.

---

## 2. Current Truth (grounded in code, 2026-06-13)

Re-check before implementing. Most of the spawn mechanism already exists.

- **`plexi_app` is already a descriptor field.** `src/app/plexi_descriptor.rs:44-52`: `plexi_app: Option<String>` ("shell command to spawn as a PGAP process instead of rendering the auto-generated form UI") plus `capabilities: Vec<String>`.
- **The host already honors it ("Tier 4").** `src/pane_ops/create.rs:981-1136` (`try_launch_cli_pgap_app`): `plexi app open <id>` → `launch_app_by_id` → after registry miss, runs `<id> --plexi`, parses the descriptor, and if `plexi_app` is present spawns it as a real `ProcessApp` with `capabilities` from the descriptor and `cwd` = current dir. **No linked terminal is forced on this path** — only the auto-renderer requests one.
- **The `--cli` flag does NOT reach Tier 4.** `src/cli/open.rs:269-283` (`open_cli_by_name`) → `open_descriptor_in_renderer` (`open.rs:230`) hard-routes to `cli-renderer` and ignores `plexi_app`. So `plexi app open --cli stint` always gets the auto-renderer; only the bare `plexi app open stint` form can hit Tier 4. **This split is a bug to fix, not a feature.**
- **Tier-chain resolution is shared and tested.** `src/cli/descriptor.rs:131-200` (`resolve_cli`): Tier 1 native `--plexi` → Tier 2 registry → Tier 3 `--help` crawl, with a disk cache. `descriptor probe` prints `plexi_app` when present (`descriptor.rs:260`).
- **Subprocess streaming already exists.** Host command `StreamProcess { correlation_id, terminal_pane_id, command, channel }` spawns a process and streams stdout/stderr/structured back over a pipe; `CancelProcess` aborts it. Gated today by `terminal.bindings`. The SDK already has a `ScrollLog` widget. So "run a command, stream output into the app" is *composable today* — it just isn't ergonomic or terminal-free.
- **No drag affordance.** Mouse primitives exist (`on_mouse_down/up/move`, `set_mouse_tracking`, `Interactive` nodes firing `ComponentEvent`), but there is no native drag/drop, no board, and the host-native `ListView` owns its own coordinates (it will not tell an app which row the pointer is over). A multi-column drag board must be a host primitive or be hand-rolled with `Raw` + manual hit-testing.
- **stint surface.** `stint 0.1.5`, repo `~/Documents/GitHub/stint`, crates `stint-cli` + `stint-core`. Tasks are YAML-frontmatter `.md` files in `.stint/tasks/` (`sprint`, `area`, `status`, `blocked_by`, `tags`, timing). Commands: `init add list show edit remove start done log archive next ready defer sprint gates check status`. `next` has `--json`; **`list` does not**, and there is **no `move`/`set` command** — moving a task between sprints today means hand-editing frontmatter or `stint edit` in `$EDITOR`.

**Implication:** the heavy lifting (descriptor → custom-app spawn) is built. The new work is: one routing fix, two UI primitives, a capability split, SDK polish, and the stint-side app + commands + bundling.

---

## 3. The Reframed Model

```
plexi app open stint            plexi app open --cli stint
        │                                 │
        ▼                                 ▼
   stint --plexi  ───────────────────────┘   (both paths converge)
        │
        ├─ descriptor has plexi_app?  ── yes ─▶  spawn custom PGAP app (Tier 4)
        │                                          • self-contained, full pane
        │                                          • NO linked terminal
        │                                          • runs CLI under the hood,
        │                                            streams to in-app Console
        │
        └─ no plexi_app  ─────────────────────▶  auto-renderer (cli-renderer)
                                                   • generic command list + forms
                                                   • in-app Console by default,
                                                     linked terminal now opt-in
```

Three rules:

1. **`--cli` and bare-id converge.** Both resolve the descriptor through the full tier chain and both prefer `plexi_app` when present. `--cli` becomes "force CLI resolution semantics," not "force the auto-renderer."
2. **A custom CLI app is terminal-free by default.** It is a normal `ProcessApp`. If it wants to show command output, it uses the new **Console** widget + `process.run`, rendered inside its own pane.
3. **The auto-renderer also adopts the in-app Console** as its default execution surface; the linked terminal becomes an opt-in (`ui_hint` or a descriptor flag) rather than the mandatory model. This realizes "the `--cli` app doesn't necessarily link to a terminal."

---

## 4. New Protocol Primitives

### 4.1 `Board` — host-rendered drag-and-drop columns (the headline primitive)

A reusable L1 component: N labeled columns, each holding ordered cards. Host owns layout, rendering, keyboard navigation, **and** pointer drag. The app supplies data and reacts to a single `move` event. This is a Host UI Kit primitive (see `docs/prm/host-ui-kit.md` — reuse before rolling your own); every future board (issues, crew, files) consumes it.

**`UiNode` (Rust, `src/protocol/ui_nodes.rs`):**

```rust
Board {
    node_id: String,
    columns: Vec<BoardColumn>,
    /// (column_index, row_index) of the focused card, for keyboard nav.
    selected: Option<(u32, u32)>,
}

struct BoardColumn {
    id: String,          // stable column key (e.g. sprint id "s5")
    title: String,
    subtitle: Option<String>,   // e.g. "9 tasks · 3 done"
    accent: Option<String>,     // hex
    rows: Vec<BoardCard>,
}

struct BoardCard {
    id: String,                 // stable card key (e.g. task "0146")
    primary: String,
    secondary: Option<String>,
    leading: Option<LeadingBadge>,   // reuse existing ListRow adornments
    chips: Vec<RowChip>,
    tone: Option<String>,            // status color via status_chip mapping
}
```

**Events emitted (`ComponentEvent { node_id, event_type, payload }`):**

| event_type | payload | Fired when |
|---|---|---|
| `select`   | `{ column, index, card_id }` | keyboard h/j/k/l or click changes focus |
| `activate` | `{ column, index, card_id }` | Enter / double-click a card |
| `move`     | `{ card_id, from_col, to_col, to_index }` | drag-drop, or keyboard move (`[`/`]` or `m`) |

**Host behavior:**

- **Layout:** horizontal `Stack` of columns; each column an internal vertical `Scroll` of cards. Reuses `style.rs` tokens and `ListRow`/`status_chip`/`pane_type_badge` from `src/widgets.rs`.
- **Keyboard:** `h/l` (or `←/→`) move focus between columns; `j/k` within a column; `Enter` activates; a move modifier (`m` → pick target, or `[`/`]` to shuffle into adjacent column) emits `move`. Fully keyboard-complete with zero mouse — satisfies "keyboard driven."
- **Drag:** `mouse_down` on a card begins a drag (host renders a floating ghost via existing draw commands), `mouse_move` (host enables tracking internally) shows an insertion marker in the hovered column, `mouse_up` emits `move` with the resolved `to_col`/`to_index`. Host owns all hit-testing; the app never sees raw coordinates. Drag is **optimistic**: the host visually relocates the card immediately and the app confirms (or the app re-renders authoritative state on the next frame).
- **Empty columns** are valid drop targets (drop at index 0).

**Python SDK (`sdk/python/plexi_sdk/ui.py`):**

```python
Board(
    node_id="sprints",
    columns=[
        BoardColumn(id="s5", title="S5", subtitle="9 · 3 done", rows=[
            BoardCard(id="0146", primary="CLI-backed app contract",
                      chips=[RowChip("cli", ...)], tone="busy"),
            ...
        ]),
        ...
    ],
    selected=(0, 2),
    on_move=self._on_move,        # (card_id, from_col, to_col, to_index)
    on_select=self._on_select,
    on_activate=self._on_activate,
)
```

The app's `on_move` calls `stint move <card_id> --sprint <to_col>` (§6) and re-fetches. Card order within a column is presentation-only for v1 unless stint grows an explicit ordering field (see Open Decisions).

### 4.2 `Console` + the `process.run` capability — terminal-free command execution

The user requirement: the app "wraps the CLI and executes under the hood, then pipes the command to a console within the app itself." This is `StreamProcess` made ergonomic and decoupled from terminal panes.

**New capability `process.run`** (distinct from `terminal.bindings`): "spawn a subprocess and stream its output into the app." `terminal.bindings` stays for apps that genuinely drive a *linked terminal pane*; `process.run` is the correct boundary for an app that just runs a command and shows the result inside itself. `StreamProcess`/`CancelProcess` are re-gated to accept either capability (back-compat for the auto-renderer) but new apps declare `process.run`.

**SDK ergonomic (`_emitter.py`):**

```python
async for line in emit.run("stint", "move", "0146", "--sprint", "s5"):
    self._console.append(line)          # streamed stdout/stderr lines
# or one-shot:
result = await emit.run_to_string("stint", "list", "--json", "--sprint", "s5")
```

`emit.run(...)` wraps `StreamProcess` + the pipe read loop that apps currently hand-write; it yields lines and resolves with the exit code. No `terminal_pane_id` required — the host runs it detached and routes output to the app's pipe.

**`Console` L1 component** (host-rendered, `ui_nodes.rs` + `components.rs`): a monospace, auto-scrolling output surface with a run banner (`$ stint move 0146 --sprint s5`), exit-code badge, and a bounded ring buffer. Conceptually `ScrollLog` polished into a first-class "command console." For v1 this MAY be implemented as a thin SDK composite over the existing `ScrollLog` + `Badge` (no new host node) — see Open Decisions; the host node is the long-term form.

### 4.3 Routing fix: `--cli` honors `plexi_app`

`open_cli_by_name`/`open_descriptor_in_renderer` (`src/cli/open.rs`) must, before falling back to `cli-renderer`, check `resolved.descriptor.plexi_app` and route to the Tier-4 custom-app spawn (`try_launch_cli_pgap_app` logic, lifted to a shared entry point so both `app open <id>` and `app open --cli <id>` use one code path). Add a `HostHarness` test asserting a descriptor with `plexi_app` spawns a `ProcessApp` and never spawns `cli-renderer` or a linked terminal.

---

## 5. Distribution: bundling a PGAP app inside a Rust CLI

`cargo install` ships only the binary. The bundled app must travel with it. The fork:

**Option A — Python app, embedded + extracted (recommended for v1).**
The PGAP app is Python (reuses the entire existing SDK, the `github-issues` reference, and the new `Board`/`Console` wrappers). stint embeds the app directory into its binary with `include_dir!`, and on `stint --plexi` (or first launch) extracts it, version-keyed, to a cache dir (`~/.stint/plexi-app/<version>/`). The descriptor then declares:

```
plexi_app = "uv run --quiet --project ~/.stint/plexi-app/<ver> main.py"
capabilities = ["process.run"]
```

- Pros: maximal SDK reuse; the `Board`/`Console` work lands in the Python SDK where every app benefits; fastest path to a beautiful result; single `cargo install` still "ships" the UI.
- Cons: a Python/`uv` runtime is required to open the UI (degrade gracefully: if absent, fall back to the auto-renderer with a hint).

**Option B — Rust-native PGAP app (north star, own epic).**
stint implements the app in Rust as a hidden subcommand (`plexi_app = "stint __plexi-app"`), single binary, zero runtime deps. This requires a **Rust PGAP SDK**, which does not exist today (the SDK is Python-only). That is a large, independently valuable platform investment ("any Rust CLI ships a Plexi app with no extra runtime").

**Recommendation:** Ship the stint board on **Option A** to prove the UX and exercise the new primitives now. File **Option B (Rust PGAP SDK)** as a separate north-star epic; it is the correct long-term distribution story but must not block this work. Embed/extract via `include_dir!` is the clean, no-fragile-paths way to make "the UI ships with the binary" true under Option A.

---

## 6. stint Repo Work (`~/Documents/GitHub/stint`)

1. **`stint --plexi`** — emit a `PlexiDescriptor` (JSON) with `name`, `version`, a minimal `commands` list (so the auto-renderer still works as a fallback), `plexi_app` (the extract path from §5), and `capabilities = ["process.run"]`. Add embed (`include_dir!`) + version-keyed extract on this path.
2. **`stint move <id> --sprint <sprint>`** — non-interactive sprint reassignment: rewrite the task's `sprint:` frontmatter via `stint-core`'s existing serializer (`crates/stint-core/src/serialize.rs`), preserving all other fields and passing `stint check`. This is the write that drag-and-drop calls. Generalize to `stint set <id> --field value` if cheap, but `move` is the required minimum.
3. **`stint list --json`** — machine-readable task list (currently only `next` has `--json`). The app reads through this, never parsing `.md` files directly (keeps the app decoupled from on-disk format; see §7 data contract).
4. **The bundled PGAP app** (`stint/plexi-app/`, Python): the board UX in §8.
5. **Graceful degradation:** if Plexi/`uv` is unavailable, `stint --plexi` still returns a valid descriptor so the auto-renderer path works.

**Data contract decision:** the app **reads via `stint list --json` / `stint show --json` and writes via `stint move`** — it never touches `.stint/*.md` directly. This survives stint storage changes and keeps `stint check`/gates authoritative. (Direct file read+write is explicitly rejected: it couples the app to the frontmatter format and bypasses validation.)

---

## 7. The stint Board App — UX Spec

Mirror `apps/github-issues/main.py` architecture: pure-render-each-frame, async fetch on a thread, `schedule_render()` after every mutation, host-native widgets.

**Views (state machine):**
- **BOARD** (primary): `Board` of sprint columns × task cards. Card = id badge + title + area chips + status tone. Column header = sprint id + task/done count.
- **DETAIL**: full task (frontmatter + markdown body via `ctx.markdown`), reusing the github-issues detail pattern.
- **SEARCH/FILTER**: an `Input` overlay for fuzzy task search; an area-filter picker (multi-select, like the github-issues label picker at `main.py:_draw_picker`).
- **CONSOLE** (slide-in or footer region): the `Console` showing the last `stint` command + streamed output + exit code.

**Interactions:**
- **Move between sprints:** drag a card to another column → `on_move` → `stint move <id> --sprint <col>` streamed into the Console → re-fetch. Keyboard equivalent: select card, `m`, pick target sprint (or `[`/`]` to adjacent column).
- **Search:** `/` opens fuzzy search over title/id; matches highlight/scope the board.
- **Filter by area:** `f` opens the area picker; selected areas AND-filter visible cards. `c` clears.
- **Other:** `Enter` → detail; `r` → refresh; `s` could cycle column grouping (by sprint ↔ by status ↔ by area) as a stretch.
- **FooterKeys** documents every binding and is the basis for the `on_key` dispatch, exactly as github-issues does.

**Beauty bar:** uses `style.rs` tokens, `key_combo_list` for the hint row, `status_chip`/`pane_type_badge`/`LeadingBadge` adornments — no hand-rolled chrome. This is meant to be the showcase of "a designed UI built inside Plexi."

---

## 8. Milestones

| M | Scope | Repo | Done when |
|---|---|---|---|
| **M1** | Routing fix: `--cli` + bare-id converge and honor `plexi_app`; shared spawn entry point | PLEXI | `HostHarness` test: descriptor with `plexi_app` spawns `ProcessApp`, no `cli-renderer`, no linked terminal, on both open paths |
| **M2** | `process.run` capability + `emit.run()/run_to_string()` ergonomics + `Console` (SDK composite over `ScrollLog` for v1) | PLEXI | A dev POC app runs a command and shows streamed output with no terminal pane; capability gating tested |
| **M3** | `Board` L1 primitive: node, host render, keyboard nav, drag, `move` event; Python `Board/BoardColumn/BoardCard` wrappers | PLEXI | `PlexiUiHarness` smoke test (open → focus → keyboard move → drag move → `move` event asserted); scene test for visual layout |
| **M4** | stint: `stint --plexi` descriptor + embed/extract; `stint move`; `stint list --json` | stint | `stint move 0146 --sprint s5` rewrites frontmatter and passes `stint check`; `stint --plexi` emits a valid descriptor |
| **M5** | The bundled stint board app (board + search + area filter + console) | stint | `plexi app open stint` launches the board; drag a card → sprint changes on disk via `stint move`; keyboard-only path complete |
| **M6** | Auto-renderer adopts in-app Console; linked terminal demoted to opt-in | PLEXI | `cli-renderer` shows output inline by default; terminal still reachable via opt-in |

M1–M2 unblock everything. M3 (Board) is the largest single piece. M4 can proceed in parallel with M1–M3 in the stint repo. M5 depends on M3 + M4. M6 is independent cleanup.

**North-star (separate epic, not in this PRM's critical path):** Rust PGAP SDK enabling Option B single-binary distribution.

---

## 9. Testing Strategy

- **Host logic** (routing, capability gating, spawn) → `HostHarness` tests, written first per `CLAUDE.md` implementation discipline.
- **Board + Console rendering/interaction** → `PlexiUiHarness` smoke tests in `src/ui_tests.rs` (open → step → assert) plus TOML scene(s) in `tests/scenes/` for visual diffs.
- **Drag** → a `HostHarness`/`PlexiUiHarness` test that injects `mouse_down`→`mouse_move`→`mouse_up` across columns and asserts the emitted `move` payload.
- **stint** → unit tests for `move` (frontmatter rewrite round-trips, other fields preserved, `check` passes) and a `--json` schema snapshot.
- **End-to-end** → scene/manual: open the board on a real `.stint`, drag a task, confirm the `.md` `sprint:` field changed and the board reflects it after refresh.

---

## 10. Open Decisions (need your call)

1. **App language for v1 — Option A (Python, embedded+extracted) vs Option B (Rust, needs new Rust SDK).** Recommendation: **A now, B as a north-star epic.** This is the one genuinely load-bearing fork; everything else follows from it.
2. **`Console` as a real host `UiNode` now, or an SDK composite over `ScrollLog` for v1?** Recommendation: **SDK composite for v1** (ship faster, no host node churn), promote to a host node if/when a second consumer appears.
3. **Card ordering within a column.** stint has no per-task ordering field. v1: order columns by existing sort (id/created); `move` only changes `sprint`, not intra-column index. A real ordering field in stint is a separate enhancement. Recommendation: **defer intra-column ordering.**
4. **Capability split timing.** Introduce `process.run` now (correct boundary) vs reuse `terminal.bindings` for v1. Recommendation: **introduce `process.run` now** — it is the 100-year-correct boundary and the whole point is decoupling from terminals.

---

## 11. File / Area Index

| Concern | Path |
|---|---|
| Descriptor schema (`plexi_app`, `capabilities`) | `src/app/plexi_descriptor.rs:27-53` |
| Tier-4 custom-app spawn | `src/pane_ops/create.rs:981-1136` |
| `--cli` open path (routing fix target) | `src/cli/open.rs:230-283` |
| Tier-chain resolver | `src/cli/descriptor.rs:131-200` |
| UI node types (add `Board`, maybe `Console`) | `src/protocol/ui_nodes.rs` |
| Component renderer | `src/render/components.rs` |
| Host widgets to reuse (chips, badges, key combos) | `src/widgets.rs`, `src/style.rs` |
| Subprocess streaming (`StreamProcess`/`CancelProcess`) | `src/protocol/commands.rs` |
| Python SDK UI components | `sdk/python/plexi_sdk/ui.py` |
| Python SDK emitter (add `run()`) | `sdk/python/plexi_sdk/_emitter.py` |
| Reference app to mirror | `apps/github-issues/main.py` |
| Host UI Kit rules | `docs/prm/host-ui-kit.md` |
| stint CLI commands | `~/Documents/GitHub/stint/crates/stint-cli/src/main.rs` |
| stint task (de)serialization | `~/Documents/GitHub/stint/crates/stint-core/src/serialize.rs` |
