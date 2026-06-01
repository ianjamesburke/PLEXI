# CLI Modularization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Break `src/cli.rs` (5309 lines, 200 kB) into a `src/cli/` module with 17 focused sub-files, one per feature domain.

**Architecture:** `src/cli.rs` becomes the module root — it stays in place and gains `pub mod` declarations. Sub-modules live in `src/cli/<name>.rs` (Rust 2021 resolution: `src/cli.rs` as root, sub-modules under `src/cli/`). Shared utilities (`send_to_socket`, `print_tip`, `binary_in_path`) are relocated to the top of `cli.rs` so every sub-module can reach them via `super::`. Tests travel with their section.

**Tech Stack:** Rust 2021, `cargo build` as the only test gate (no behavior changes — pure reorganization).

---

## Final Module Map

After completion `src/cli.rs` contains only: shared utilities + `pub mod` declarations.

| File | Approx lines cut from cli.rs | Key public items |
|---|---|---|
| `cli/run.rs` | 1–395 | `PlexiCommands`, `CommandEntry`, `run_list_commands`, `run_command` |
| `cli/routine.rs` | 396–500 | `routine_list`, `routine_run` |
| `cli/workspace.rs` | 501–934 | `workspace_init`, `workspace_secret_*` |
| `cli/app.rs` | 935–1562 + helpers at 2599–2629 | `app_init`, `app_install`, `app_run`, etc. |
| `cli/install.rs` | 1563–2348 | `install_cli`, `self_update_cli`, etc. |
| `cli/list.rs` | 2349–2475 | `list_cli`, `freeze_cli` |
| `cli/notify.rs` | 2476–2629 (minus helpers) + test at 4984–5080 | `notify_cli`, `parse_notify_choice` |
| `cli/pane.rs` | 2630–3135 | `pane_*_cli`, `print_json_output` |
| `cli/open.rs` | 3136–3545 | `open_cli`, `terminal_cli` |
| `cli/registry.rs` | 3546–3908 | inner `registry` mod contents |
| `cli/descriptor.rs` | 3909–4219 | inner `descriptor` mod contents |
| `cli/validate.rs` | 4220–4353 | `validate_cli` |
| `cli/context.rs` | 4354–4525 | `context_*_cli`, `resolve_path` |
| `cli/config.rs` | 4526–4636 | `config_check`, `config_edit`, `config_get`, `config_reset` |
| `cli/notes.rs` | 4637–4724 | `notes_list_cli`, `notes_open_cli` |
| `cli/demo.rs` | 4725–4882 | `demo_cli`, `poll_event` |
| `cli/completions.rs` | 4883–5080 | `completions_cli` |

Three helpers move from their current position to the **top of `cli.rs`** so sub-modules can call `super::fn_name()`:
- `print_tip` (currently at line ~98, stays because workspace + install use it)
- `send_to_socket` (currently at line ~4364, used by app, pane, open, context)
- `binary_in_path` (currently at line ~4637, used by config + notes)

---

## Task 0: Create `src/cli/` and relocate shared utilities in `cli.rs`

**Files:**
- Modify: `src/cli.rs` (reorganize header, move 3 functions to top)

- [ ] **Step 0a: Create the sub-module directory**

```bash
mkdir src/cli
```

- [ ] **Step 0b: Find the exact byte positions of the three shared helpers**

```bash
grep -n "^fn print_tip\|^fn send_to_socket\|^fn binary_in_path" src/cli.rs
```

Expected output (approximate lines):
```
98:fn print_tip(msg: &str) {
4364:fn send_to_socket(payload: serde_json::Value) -> i32 {
4637:fn binary_in_path(name: &str) -> bool {
```

- [ ] **Step 0c: Move all three functions to immediately after the top-level imports in `cli.rs`**

Cut `print_tip`, `send_to_socket`, and `binary_in_path` from their current locations and paste them right after the `CommandDef` struct (around line 65). Change their visibility to `pub(super)` so sub-modules can access them:

```rust
pub(super) fn print_tip(msg: &str) {
    // (existing body unchanged)
}

pub(super) fn send_to_socket(payload: serde_json::Value) -> i32 {
    // (existing body unchanged)
}

pub(super) fn binary_in_path(name: &str) -> bool {
    // (existing body unchanged)
}
```

- [ ] **Step 0d: Verify it still compiles**

```bash
cargo build 2>&1 | head -20
```

Expected: no errors (warnings about unused functions are ok at this stage).

---

## Task 1: Extract `cli/demo.rs`

Smallest, no dependencies on other cli sections. Good confidence-builder.

**Files:**
- Create: `src/cli/demo.rs`
- Modify: `src/cli.rs`

- [ ] **Step 1a: Create `src/cli/demo.rs`**

Cut lines 4725–4882 from `cli.rs` (the `demo_cli` function and `poll_event` helper) and write them into this new file with the following header:

```rust
use std::io;

pub fn demo_cli() -> i32 {
    // (paste the cut block here, unchanged)
}

fn poll_event<F>(path: &std::path::Path, mut offset: u64, mut predicate: F) -> std::io::Result<u64>
where
    F: FnMut(&str, &serde_json::Value) -> bool,
{
    use std::io::{Read, Seek, SeekFrom};
    // (paste the cut block here, unchanged)
}
```

The only import needed at the top is `use std::io;` (already implied by `io::Result` return type; `serde_json` is accessed via the `serde_json::` path inline).

- [ ] **Step 1b: Replace the cut block in `cli.rs` with a module declaration**

Where the demo functions used to be, add:

```rust
pub mod demo;
```

- [ ] **Step 1c: Build**

```bash
cargo build 2>&1 | head -30
```

Expected: clean build. Fix any `use` errors before continuing.

- [ ] **Step 1d: Commit**

```bash
git add src/cli.rs src/cli/demo.rs
git commit -m "refactor(cli): extract demo module"
```

---

## Task 2: Extract `cli/completions.rs`

Self-contained, has its own test module.

**Files:**
- Create: `src/cli/completions.rs`
- Modify: `src/cli.rs`

- [ ] **Step 2a: Create `src/cli/completions.rs`**

Cut lines 4883–5080 (from `pub fn completions_cli` through the end of `mod completions_tests`) and write:

```rust
pub fn completions_cli(shell: &str, binary_name: &str) -> i32 {
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};
    // (paste unchanged)
}

fn fix_zsh_workspace_path_conflict(script: &str) -> String {
    // (paste unchanged)
}

#[cfg(test)]
mod completions_tests {
    use super::fix_zsh_workspace_path_conflict;
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};
    // (paste unchanged)
}
```

- [ ] **Step 2b: Add `pub mod completions;` in `cli.rs` where the block was cut**

- [ ] **Step 2c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/completions.rs
git commit -m "refactor(cli): extract completions module"
```

---

## Task 3: Extract `cli/notes.rs`

Uses `super::binary_in_path` (relocated to top of `cli.rs` in Task 0).

**Files:**
- Create: `src/cli/notes.rs`
- Modify: `src/cli.rs`

- [ ] **Step 3a: Create `src/cli/notes.rs`**

Cut lines 4637–4724 (from `pub fn notes_list_cli` through `notes_open_cli`) and write:

```rust
pub fn notes_list_cli() -> i32 {
    // replace any bare `binary_in_path(` calls with `super::binary_in_path(`
    // (paste unchanged otherwise)
}

pub fn notes_open_cli() -> i32 {
    // replace any bare `binary_in_path(` calls with `super::binary_in_path(`
    // (paste unchanged otherwise)
}
```

- [ ] **Step 3b: Add `pub mod notes;` in `cli.rs`**

- [ ] **Step 3c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/notes.rs
git commit -m "refactor(cli): extract notes module"
```

---

## Task 4: Extract `cli/config.rs`

Uses `super::binary_in_path`.

**Files:**
- Create: `src/cli/config.rs`
- Modify: `src/cli.rs`

- [ ] **Step 4a: Create `src/cli/config.rs`**

Cut lines 4526–4636 (`config_check`, `config_edit`, `config_get`, `config_reset`) and write:

```rust
use std::process::Command;

pub fn config_check() -> i32 {
    // replace any bare `binary_in_path(` with `super::binary_in_path(`
    // (paste unchanged otherwise)
}

pub fn config_edit() -> i32 { /* paste */ }
pub fn config_get(key: &str) -> i32 { /* paste */ }
pub fn config_reset() -> i32 { /* paste */ }
```

- [ ] **Step 4b: Add `pub mod config;` in `cli.rs`**

- [ ] **Step 4c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/config.rs
git commit -m "refactor(cli): extract config module"
```

---

## Task 5: Extract `cli/context.rs`

Uses `super::send_to_socket`. `resolve_path` moves here (only used by context functions).

**Files:**
- Create: `src/cli/context.rs`
- Modify: `src/cli.rs`

- [ ] **Step 5a: Create `src/cli/context.rs`**

Cut lines 4354–4525 (`resolve_path`, all `context_*_cli` functions) and write:

```rust
fn resolve_path(path: Option<&str>) -> Result<std::path::PathBuf, String> {
    // (paste unchanged)
}

pub fn context_new_cli(name: Option<&str>, path: Option<&str>, parent: Option<&str>) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // replace bare `resolve_path(` — stays as `resolve_path(` (local)
    // (paste unchanged otherwise)
}

pub fn context_zoom_cli(context_id: u64) -> i32 { /* paste, super::send_to_socket */ }
pub fn context_zoom_out_cli() -> i32 { /* paste, super::send_to_socket */ }
pub fn context_open_cli(path: Option<&str>) -> i32 { /* paste, super::send_to_socket */ }
pub fn context_set_root_cli(path: Option<&str>) -> i32 { /* paste, super::send_to_socket */ }
pub fn context_describe_cli(text: &str) -> i32 { /* paste, super::send_to_socket */ }
pub fn context_current_cli() -> i32 { /* paste, super::send_to_socket */ }
```

- [ ] **Step 5b: Add `pub mod context;` in `cli.rs`**

- [ ] **Step 5c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/context.rs
git commit -m "refactor(cli): extract context module"
```

---

## Task 6: Extract `cli/validate.rs`

Uses `super::send_to_socket`.

**Files:**
- Create: `src/cli/validate.rs`
- Modify: `src/cli.rs`

- [ ] **Step 6a: Create `src/cli/validate.rs`**

Cut lines 4220–4353 (`validate_cli`) and write:

```rust
pub fn validate_cli(path: &str) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // (paste unchanged otherwise)
}
```

- [ ] **Step 6b: Add `pub mod validate;` in `cli.rs`**

- [ ] **Step 6c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/validate.rs
git commit -m "refactor(cli): extract validate module"
```

---

## Task 7: Extract `cli/notify.rs`

Also includes the `notify_tests` module (currently near line 4984 in `cli.rs`).

**Files:**
- Create: `src/cli/notify.rs`
- Modify: `src/cli.rs`

- [ ] **Step 7a: Find the notify test module**

```bash
grep -n "mod notify_tests" src/cli.rs
```

Expected: `4984:mod notify_tests {`

- [ ] **Step 7b: Create `src/cli/notify.rs`**

Cut lines 2476–2598 (`parse_notify_choice`, `notify_cli` — stop before `to_title_case`) plus lines 4984–5080 (`mod notify_tests`) and combine into:

```rust
use std::io::Write;

pub(crate) fn parse_notify_choice(raw: &str) -> Result<(String, String, Option<String>), String> {
    // (paste unchanged)
}

pub fn notify_cli(
    title: &str,
    body: &str,
    level: &str,
    choices: &[(String, String, Option<String>)],
    // ... remaining params
) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // (paste unchanged otherwise)
}

#[cfg(test)]
mod notify_tests {
    use super::parse_notify_choice;
    // (paste the test module body unchanged)
}
```

- [ ] **Step 7c: Add `pub mod notify;` in `cli.rs`**

- [ ] **Step 7d: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/notify.rs
git commit -m "refactor(cli): extract notify module"
```

---

## Task 8: Extract `cli/routine.rs`

Self-contained, no shared helpers.

**Files:**
- Create: `src/cli/routine.rs`
- Modify: `src/cli.rs`

- [ ] **Step 8a: Create `src/cli/routine.rs`**

Cut lines 396–500 (`routine_list`, `routine_run`) and write:

```rust
use std::process::Command;

pub fn routine_list() -> i32 {
    // (paste unchanged)
}

pub fn routine_run(name: &str) -> i32 {
    // (paste unchanged)
}
```

- [ ] **Step 8b: Add `pub mod routine;` in `cli.rs`**

- [ ] **Step 8c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/routine.rs
git commit -m "refactor(cli): extract routine module"
```

---

## Task 9: Extract `cli/list.rs`

Self-contained.

**Files:**
- Create: `src/cli/list.rs`
- Modify: `src/cli.rs`

- [ ] **Step 9a: Create `src/cli/list.rs`**

Cut lines 2349–2475 (`list_cli`, `freeze_cli`) and write:

```rust
pub fn list_cli() -> i32 {
    // (paste unchanged — uses crate::app_registry, crate::install directly)
}

pub fn freeze_cli(dest_path: &str) -> i32 {
    // (paste unchanged)
}
```

- [ ] **Step 9b: Add `pub mod list;` in `cli.rs`**

- [ ] **Step 9c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/list.rs
git commit -m "refactor(cli): extract list/freeze module"
```

---

## Task 10: Extract `cli/pane.rs`

Largest single-domain chunk. Uses `super::send_to_socket`.

**Files:**
- Create: `src/cli/pane.rs`
- Modify: `src/cli.rs`

- [ ] **Step 10a: Create `src/cli/pane.rs`**

Cut lines 2630–3135 (`pane_set_title_cli`, `print_json_output`, all `pane_*_cli`) and write:

```rust
use std::io::Write;
use std::process::Command;

fn print_json_output(json_str: &str) -> i32 {
    // (paste unchanged)
}

pub fn pane_set_title_cli(pane_id: Option<u64>, name: &str) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // (paste unchanged otherwise)
}

// paste all remaining pane_*_cli functions with the same super:: substitution
pub fn pane_list_cli(context: Option<u64>, current: bool) -> i32 { /* paste */ }
pub fn pane_self_cli() -> i32 { /* paste */ }
pub fn pane_info_cli() -> i32 { /* paste */ }
pub fn pane_focus_cli(pane_id: u64) -> i32 { /* paste */ }
pub fn pane_close_cli(pane_id: u64) -> i32 { /* paste */ }
pub fn pane_send_cli(pane_id: u64, text: &str) -> i32 { /* paste */ }
pub fn pane_key_cli(pane_id: u64, key: &str) -> i32 { /* paste */ }
pub fn pane_capture_cli(pane_id: Option<u64>, lines: usize, full_output: bool, from_cursor: Option<u64>) -> i32 { /* paste */ }
```

- [ ] **Step 10b: Add `pub mod pane;` in `cli.rs`**

- [ ] **Step 10c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/pane.rs
git commit -m "refactor(cli): extract pane module"
```

---

## Task 11: Extract `cli/open.rs`

Uses `super::send_to_socket`. Includes `read_secret_from_stdin` and `read_line_plain`.

**Files:**
- Create: `src/cli/open.rs`
- Modify: `src/cli.rs`

- [ ] **Step 11a: Create `src/cli/open.rs`**

Cut lines 3136–3545 (`open_github_ephemeral`, `open_cli`, `terminal_cli`, `read_secret_from_stdin`, `read_line_plain`) and write:

```rust
use std::io::{self, Write};
use std::process::Command;

fn read_line_plain() -> io::Result<String> {
    // (paste unchanged)
}

fn read_secret_from_stdin() -> io::Result<String> {
    // (paste unchanged — calls read_line_plain, local)
}

fn open_github_ephemeral(source: &str, layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // (paste unchanged otherwise)
}

pub fn open_cli(type_id: &str, args: &[String], layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // (paste unchanged otherwise)
}

pub fn terminal_cli(cmd: Option<&str>, ephemeral: bool, layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>, no_focus: bool) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // (paste unchanged otherwise)
}
```

- [ ] **Step 11b: Add `pub mod open;` in `cli.rs`**

- [ ] **Step 11c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/open.rs
git commit -m "refactor(cli): extract open/terminal module"
```

---

## Task 12: Extract `cli/registry.rs` and `cli/descriptor.rs`

These are currently inline `pub mod registry { }` and `pub mod descriptor { }` blocks inside `cli.rs`. They become top-level files inside `cli/`.

**Files:**
- Create: `src/cli/registry.rs`
- Create: `src/cli/descriptor.rs`
- Modify: `src/cli.rs`

- [ ] **Step 12a: Find exact block boundaries**

```bash
grep -n "^pub mod registry\|^pub mod descriptor\|^mod registry_watch_tests\|^mod descriptor_tests" src/cli.rs
```

Expected:
```
3546:pub mod registry {
3832:mod registry_watch_tests {
3909:pub mod descriptor {
4121:mod descriptor_tests {
```

- [ ] **Step 12b: Create `src/cli/registry.rs`**

Cut the **body** of `pub mod registry { ... }` (lines 3547–3831) plus `mod registry_watch_tests { ... }` (lines 3832–3908) and write the file with the inner content promoted to top-level:

```rust
// Everything that was inside `pub mod registry { }` in cli.rs, verbatim.
// The `use crate::cli_registry;` and other imports inside the old block
// become top-level use statements here.
use crate::cli_registry;
use std::collections::BTreeSet;
use std::process::Command;

pub trait CliInspector {
    // (paste verbatim from the old module body)
}

// ... all other items from the old registry mod body ...

#[cfg(test)]
mod registry_watch_tests {
    // (paste verbatim)
}
```

Replace the old `pub mod registry { ... }` block in `cli.rs` with:

```rust
pub mod registry;
```

- [ ] **Step 12c: Create `src/cli/descriptor.rs`**

Same pattern — cut the body of `pub mod descriptor { ... }` (lines 3910–4120) plus `mod descriptor_tests { ... }` (lines 4121–4219) and write them as top-level items in the new file.

Replace the old `pub mod descriptor { ... }` block in `cli.rs` with:

```rust
pub mod descriptor;
```

- [ ] **Step 12d: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/registry.rs src/cli/descriptor.rs
git commit -m "refactor(cli): extract registry and descriptor modules"
```

---

## Task 13: Extract `cli/workspace.rs`

Includes secret helpers. Has `secret_set_tests` and `workspace_init_tests`.

**Files:**
- Create: `src/cli/workspace.rs`
- Modify: `src/cli.rs`

- [ ] **Step 13a: Find test module lines**

```bash
grep -n "mod secret_set_tests\|mod workspace_init_tests" src/cli.rs
```

Expected: lines 5081 and 5190 (approximately; they've shifted as earlier code was cut).

- [ ] **Step 13b: Create `src/cli/workspace.rs`**

Cut: `workspace_init`, `require_workspace`, `workspace_secret_set`, `workspace_secret_list`, `workspace_secret_get`, `workspace_secret_delete` (original lines 501–934) plus `mod secret_set_tests` and `mod workspace_init_tests` (originally lines 5081–5243) and write:

```rust
use std::io::{self, Write};
use std::process::Command;

fn require_workspace() -> Result<(std::path::PathBuf, crate::workspace::WorkspaceConfig), String> {
    // replace bare `print_tip(` with `super::print_tip(`
    // (paste unchanged otherwise)
}

fn read_line_plain() -> io::Result<String> {
    // (paste unchanged — this fn is also in open.rs; local duplicate is fine,
    //  or factor into super:: if you prefer)
}

fn read_secret_from_stdin() -> io::Result<String> {
    // (paste unchanged)
}

pub fn workspace_init() -> i32 {
    // replace bare `print_tip(` with `super::print_tip(`
    // (paste unchanged otherwise)
}

pub fn workspace_secret_set(friendly: &str, from_env: bool, global: bool, alias: Option<&str>) -> i32 { /* paste */ }
pub fn workspace_secret_list() -> i32 { /* paste */ }
pub fn workspace_secret_get(friendly: &str, global: bool) -> i32 { /* paste */ }
pub fn workspace_secret_delete(friendly: &str) -> i32 { /* paste */ }

#[cfg(test)]
mod secret_set_tests { /* paste */ }

#[cfg(test)]
mod workspace_init_tests { /* paste */ }
```

> **Note on `read_secret_from_stdin`/`read_line_plain`:** These were physically in the `open` section in `cli.rs` but only called from workspace. They're already moved to `open.rs` in Task 11. Instead of duplicating, call `crate::cli::open::read_secret_from_stdin()`. Check with `grep -n "read_secret_from_stdin" src/cli.rs` after Task 11 completes to confirm the exact path.

- [ ] **Step 13c: Add `pub mod workspace;` in `cli.rs`**

- [ ] **Step 13d: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/workspace.rs
git commit -m "refactor(cli): extract workspace module"
```

---

## Task 14: Extract `cli/install.rs`

Large section with install/update functions. Uses `super::print_tip` and `super::send_to_socket` (indirectly via `app_run`).

**Files:**
- Create: `src/cli/install.rs`
- Modify: `src/cli.rs`

- [ ] **Step 14a: Create `src/cli/install.rs`**

Cut lines 1563–2348 (`is_bare_id`, `is_github_shorthand`, `resolve_registry_id`, `install_cli`, `install_pack_cli`, `install_workspace_pack_cli`, `plexi_uninstall_cli`, `update_cli`, `self_update_cli`) and write:

```rust
use std::process::Command;

fn is_bare_id(s: &str) -> bool { /* paste */ }
fn is_github_shorthand(s: &str) -> bool { /* paste */ }
fn resolve_registry_id(id: &str) -> Result<String, String> { /* paste */ }

pub fn install_cli(spec: &str) -> i32 {
    // replace bare `print_tip(` with `super::print_tip(`
    // (paste unchanged otherwise)
}

pub fn install_pack_cli(spec: &str) -> i32 { /* paste, super::print_tip */ }
pub fn install_workspace_pack_cli() -> i32 { /* paste */ }
pub fn plexi_uninstall_cli(keep_data: bool, assume_yes: bool) -> i32 { /* paste */ }
pub fn update_cli(maybe_id: Option<&str>) -> i32 { /* paste */ }
pub fn self_update_cli() -> i32 { /* paste */ }
```

- [ ] **Step 14b: Add `pub mod install;` in `cli.rs`**

- [ ] **Step 14c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/install.rs
git commit -m "refactor(cli): extract install/update module"
```

---

## Task 15: Extract `cli/app.rs`

Largest section. Uses `super::send_to_socket` (in `app_run`). `to_title_case` and `to_struct_name` were physically in the notify section in the original file — move them here.

**Files:**
- Create: `src/cli/app.rs`
- Modify: `src/cli.rs`

- [ ] **Step 15a: Create `src/cli/app.rs`**

Cut lines 935–1562 (`ensure_plexi_sdk`, `app_is_python`, `app_init_config_dir`, `app_init`, `scaffold_python_app`, `scaffold_rust_app`, `app_uninstall`, `app_install`, `copy_dir_all`, `app_run`, `app_info`, `app_list`, `app_render`, `parse_render_size`) plus `to_title_case` and `to_struct_name` (currently at original lines 2599–2629, already cut if Task 7 used exact boundaries — confirm with grep) and the `app_run_tests` module (originally at ~5162). Write:

```rust
use std::io::{self, Write};
use std::process::Command;

fn to_title_case(s: &str) -> String { /* paste */ }
fn to_struct_name(s: &str) -> String { /* paste */ }

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> io::Result<()> { /* paste */ }

fn ensure_plexi_sdk() -> bool { /* paste */ }
fn app_is_python(app_dir: &std::path::Path) -> bool { /* paste */ }
fn app_init_config_dir() -> String { /* paste */ }
fn parse_render_size(s: &str) -> Option<(u32, u32)> { /* paste */ }

fn scaffold_python_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> { /* paste */ }
fn scaffold_rust_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> { /* paste */ }

pub fn app_init(name: &str, lang: &str, from_pane_id: Option<u64>) -> i32 {
    // (paste unchanged — uses local to_title_case/to_struct_name)
}
pub fn app_uninstall(id: &str, assume_yes: bool) -> i32 { /* paste */ }
pub fn app_install(path: &str) -> i32 {
    // replace bare `print_tip(` with `super::print_tip(`
    // (paste unchanged otherwise)
}
pub fn app_run(path: &str, from_pane_id: Option<u64>) -> i32 {
    // replace bare `send_to_socket(` with `super::send_to_socket(`
    // (paste unchanged otherwise)
}
pub fn app_info(id: &str) -> i32 { /* paste */ }
pub fn app_list() -> i32 { /* paste */ }
pub fn app_render(id: &str, size: &str, state: Option<&str>, output: Option<&str>) -> i32 { /* paste */ }

#[cfg(test)]
mod app_run_tests { /* paste */ }
```

- [ ] **Step 15b: Add `pub mod app;` in `cli.rs`**

- [ ] **Step 15c: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/app.rs
git commit -m "refactor(cli): extract app module"
```

---

## Task 16: Extract `cli/run.rs`

Contains the command-parsing types and `run_list_commands`/`run_command`. These types (`PlexiCommands`, `CommandEntry`, etc.) may be used elsewhere — verify with grep first.

**Files:**
- Create: `src/cli/run.rs`
- Modify: `src/cli.rs`

- [ ] **Step 16a: Verify type usage**

```bash
grep -rn "PlexiCommands\|CommandEntry\|CommandDef\|SecretsConfig" src/ | grep -v "cli\.rs\|cli/run"
```

If any matches appear outside `cli.rs`, those files need `crate::cli::run::PlexiCommands` (or add re-exports in `cli.rs` — see Step 16d).

- [ ] **Step 16b: Create `src/cli/run.rs`**

Cut lines 1–395 from `cli.rs` (all types + `list_global_scripts`, `is_executable`, `run_list_commands`, `run_command`) plus `mod command_parse_tests` (originally at ~5244). Write:

```rust
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Write};
use std::process::Command;

const APP_ID: &str = "plexi-run";
const COMMANDS_FILE: &str = ".plexi/commands.toml";

#[derive(Deserialize)]
pub struct PlexiCommands {
    // (paste unchanged)
}

#[derive(Deserialize, Default)]
pub struct SecretsConfig {
    // (paste unchanged)
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum CommandEntry {
    // (paste unchanged)
}

impl CommandEntry { /* paste */ }

#[derive(Deserialize)]
pub struct CommandDef { /* paste */ }

fn list_global_scripts(scripts_dir: &std::path::Path) -> Vec<String> { /* paste */ }
fn is_executable(path: &std::path::Path) -> bool { /* paste */ }

pub fn run_list_commands() -> i32 {
    // replace bare `print_tip(` with `super::print_tip(`
    // (paste unchanged otherwise)
}

pub fn run_command(command_name: &str) -> i32 {
    // replace bare `print_tip(` with `super::print_tip(`
    // (paste unchanged otherwise)
}

#[cfg(test)]
mod command_parse_tests {
    use super::{CommandEntry, PlexiCommands};
    // (paste unchanged)
}
```

- [ ] **Step 16c: Add `pub mod run;` in `cli.rs`**

- [ ] **Step 16d: Re-export types if needed**

If Step 16a found external callers, add to `cli.rs`:

```rust
pub use run::{PlexiCommands, SecretsConfig, CommandEntry, CommandDef};
```

- [ ] **Step 16e: Build and commit**

```bash
cargo build 2>&1 | head -30
git add src/cli.rs src/cli/run.rs
git commit -m "refactor(cli): extract run/command module"
```

---

## Task 17: Final cleanup of `cli.rs`

After all 16 extractions, `cli.rs` should contain only: imports, the three shared utility functions, and `pub mod` declarations.

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 17a: Review what remains in `cli.rs`**

```bash
wc -l src/cli.rs
grep -c "^pub fn\|^fn " src/cli.rs
```

Expected: under 80 lines total, only 3 `fn` declarations (`print_tip`, `send_to_socket`, `binary_in_path`).

- [ ] **Step 17b: Clean up any orphaned imports**

`cli.rs` may have leftover `use` statements that are now in sub-modules. Remove any that produce warnings:

```bash
cargo build 2>&1 | grep "unused import"
```

For each reported unused import, remove it from `cli.rs`.

- [ ] **Step 17c: Final build and test**

```bash
cargo build 2>&1
cargo test --bin plexi 2>&1 | tail -20
```

Expected: clean build, all tests passing.

- [ ] **Step 17d: Commit**

```bash
git add src/cli.rs
git commit -m "refactor(cli): final cleanup — cli.rs is now just shared utils + pub mods"
```

---

## Self-Review

**Spec coverage:** All 17 sub-modules mapped. `demo`, `completions`, `notes`, `config`, `context`, `validate`, `notify`, `routine`, `list`, `pane`, `open`, `registry`, `descriptor`, `workspace`, `install`, `app`, `run` — covered.

**Shared helpers:** `print_tip`, `send_to_socket`, `binary_in_path` explicitly relocated to cli.rs root in Task 0. Every sub-module that calls them uses `super::fn_name()`.

**Type re-exports:** Task 16a has an explicit grep step to catch any external callers of `PlexiCommands`/`CommandEntry` before the extraction.

**Test migration:** Each test module travels with its section. Tasks 2, 7, 13, 15, 16 each name which test module they carry.

**`read_secret_from_stdin` duplication:** Task 13 explicitly calls this out — check whether to reference `crate::cli::open::read_secret_from_stdin` or keep a local copy. Either is fine; the plan flags the decision point.

**No behavior changes:** This is pure reorganization. `cargo build` green at each step is the only gate.
