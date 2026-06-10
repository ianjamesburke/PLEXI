# Plexi Host UI Kit PRM

Status: planning source for host-level UI chrome.
Last updated: 2026-06-10.

This PRM defines the path from hand-built egui overlays to a Plexi-owned host UI kit. It covers modals, palettes, pickers, rows, text fields, buttons, hint bars, focus capture, and overlay lifecycle.

It does not cover PGAP app authoring, SDK component trees, app marketplace UI, or WebView/native app rendering. Those belong to the app framework PRM.

## Progress

GitHub issue state is the source of truth. This table is the PRM reader's map so finished work is visible without re-deriving the sequence from closed issues.

When a PRD-backed issue lands, update its row in this table before treating the issue as closed out. If a step is superseded, mark it `Superseded` and link the replacement issue. If a step reopens, move it back to `Ready`, `In progress`, or `Blocked`.

| Step | Issue | Status | Completion note |
|---|---|---|---|
| 1. ListRow + NotesPicker | [#2122](https://github.com/ianjamesburke/PLEXI/issues/2122) | Done | Landed in PR #2129; NotesPicker rows use host ListRow |
| 2. ModalShell + HintBar | [#2123](https://github.com/ianjamesburke/PLEXI/issues/2123) | Done | Landed in PR #2129; NotesPicker uses ModalShell and HintBar |
| 3. TextField focus registration | [#2124](https://github.com/ianjamesburke/PLEXI/issues/2124) | Done | CommandPalette search uses host TextField focus registration |
| 4. CommandPalette migration | [#2125](https://github.com/ianjamesburke/PLEXI/issues/2125) | Blocked | Blocked by #2122, #2123, #2124 |
| 5. QuickNote menu migration | [#2126](https://github.com/ianjamesburke/PLEXI/issues/2126) | Blocked | Blocked by #2122, #2123, #2124 |
| 6. Host UI gallery | [#2127](https://github.com/ianjamesburke/PLEXI/issues/2127) | Blocked | Blocked by #2122, #2123, #2124 |

## Purpose

Host UI should feel like Plexi, not like default egui.

Today the host uses egui directly in too many places. Each modal decides its own scrim, frame, padding, row height, text baseline, input chrome, button paint, footer hints, and focus behavior. That makes small visual bugs common and expensive. The Notes picker row alignment bug is a symptom: the row allocates a fixed rect, paints a background, then lays out labels inside a child `Ui` without a vertical-centering contract.

The fix is not a new GUI framework. Plexi should keep egui for rendering, input, hit testing, and text editing, but stop exposing default egui widget behavior to host chrome. The host needs a small, opinionated UI kit that owns Plexi's modal/list/input/button grammar.

## North Star Fit

This is Phase 1 work: stabilize and polish. `NORTH_STAR.md` names a unified modal system, focus system unification, shared QuickNote input handling, notification polish, and core theming consistency as active Phase 1 needs.

The work also prepares Phase 2 by applying the same rule to host chrome that PGAP will apply to apps: callers describe structure and intent; the renderer owns layout, spacing, theme, focus, and hit testing.

## Non-Goals

- Do not replace egui.
- Do not create a general-purpose Rust UI framework.
- Do not rewrite PGAP, SDK v2, or app component rendering as part of this lane.
- Do not migrate every overlay in one PR.
- Do not add a second source of truth for colors, spacing, or typography outside `src/ui/style.rs` and `src/ui/theme.rs`.

## Current Truth

- `src/ui/style.rs` has the right start: shared spacing, type sizes, radii, modal padding, and button heights.
- `src/ui/widgets.rs` already has several host primitives: selectable rows, key chips, key combo lists, styled single-line input, copy button, description label, and a dismissable modal helper.
- The existing widget layer is too shallow. Callers still hand-roll modal shells, row geometry, input visuals, button paint, footer hint rows, and focus behavior.
- `src/overlays/notes_picker.rs` hand-paints a `28px` list row and then lays out labels in a child `Ui`, which causes visible vertical misalignment.
- `src/overlays/command_palette.rs` uses `selectable_row`, but still builds its own modal shell and search input visuals.
- `src/overlays/quick_note.rs` repeats scrim/frame/menu-row/footer-hint patterns across compose, destination, and submenu modals.
- `src/overlays/notification_modal.rs` already custom-paints option and primary buttons because default egui buttons do not provide the needed fixed-rect label centering.
- `src/app/mod.rs` owns a large `FocusLayer` switch for overlay key dispatch and rendering.
- `src/app/render.rs` has a post-CentralPanel hard-coded focus re-request list for text-owning overlays because egui focus is last-writer-wins.
- `GOTCHAS.md` documents the two-layer egui TextEdit focus problem. New text-owning overlays are easy to break unless they are wired into both the one-shot overlay focus path and the post-CentralPanel re-focus path.

## Design Principles

- Host chrome is Plexi-owned. egui is the backend, not the design language.
- Components own geometry. A caller should not know the magic row height, baseline offset, focus ring stroke, or trailing-action gutter.
- Components own state-specific paint. Normal, hover, selected, focused, disabled, danger, and active states should be named component states, not ad hoc color choices.
- Overlay focus is part of the modal contract. A text field in a modal should not require a caller to remember `request_focus` ordering details.
- Migrations must be vertical slices. Each PR should move one visible surface onto the kit and leave the app usable.
- The first migration should fix a real bug. NotesPicker is the proving ground.

## Component Inventory

### ModalShell

Owns the scrim, area anchor, frame fill, border, radius, width, padding, title slot, body slot, footer slot, click-away behavior, Escape behavior, and `FocusLayer` registration.

It should support at least:

- centered modal
- top-centered palette
- fixed width and responsive width
- click-away dismiss
- modal title and optional subtitle
- footer hint row
- optional focused field registration

### ListRow

Owns row height, padding, selected/hover states, vertical centering, body layout, secondary text, leading chip/icon, trailing action, truncation, and click hit target.

It should support at least:

- single-line rows
- two-line rows
- key-chip leading slot
- trailing icon/action button
- selected and hovered states
- scroll-to-selected integration

### TextField

Wraps egui `TextEdit` but owns Plexi's field visual treatment.

It should support at least:

- single-line search/input field
- multiline note/text editor field where needed by host overlays
- hint text styling
- accent cursor
- active/inactive border
- focus registration with the overlay system
- submit/cancel metadata where useful

### Button

Owns Plexi button paint and hit targets.

It should support at least:

- primary
- secondary
- destructive
- icon-only
- full-width option
- selected option with shortcut hint

### HintBar

Owns footer shortcut display and spacing.

It should build on the existing key chip primitives, but callers should pass semantic hint groups instead of hand-laying out `key_combo_list` calls.

### Paint Primitives

Low-level shared painting helpers should live under `src/ui` and back the components above. These should cover centered text in rects, filled/stroked rounded rects, focus rings, key chips, badges, button faces, and row backgrounds.

## Migration Plan

### 1. Prove the Row Contract with NotesPicker

Add host row primitives and migrate NotesPicker to them. This fixes the visible row text alignment issue and gives the UI kit a real first user.

Done when:

- NotesPicker rows vertically center filename, preview, and delete action.
- Row height and padding come from the new row primitive.
- NotesPicker no longer creates a child `Ui` inside a fixed row rect for row content.
- `cargo build` passes.

### 2. Add ModalShell and HintBar

Move shared modal shell behavior into `src/ui`. Use it in NotesPicker after the row migration, then one more small overlay.

Done when:

- A caller can render a centered modal without hand-writing scrim `Area`, frame fill/stroke/radius, padding, or click-away handling.
- A caller can render a footer hint row without manually spacing `key_combo_list` calls.
- Existing keyboard behavior is unchanged.
- `cargo build` passes.

### 3. Add TextField Focus Registration

Move text-field focus ownership into the host UI kit. Keep egui `TextEdit` internally, but make overlay callers register focused field intent with the shell instead of adding hard-coded IDs to the post-CentralPanel block.

Done when:

- At least command palette search or rename pane focus is owned by the UI kit path.
- The post-CentralPanel focus block starts consuming a registry instead of only hard-coded field IDs.
- Existing focus regression tests still pass.
- `cargo build` passes.

### 4. Migrate CommandPalette

Move the command palette shell, search field, selected rows, section header, background/running badges, and empty state onto the kit.

Done when:

- CommandPalette no longer hand-writes its modal shell or search field visuals.
- Context and app rows use the shared row component.
- The selected row scroll behavior still works.
- `cargo build` passes.

### 5. Migrate QuickNote Menus

Move QuickNote destination and submenu modals onto ModalShell, ListRow, TextField where applicable, and HintBar.

Done when:

- QuickNote compose keeps its paste and Enter behavior.
- Destination and submenu rows use the shared row component.
- Footer hints use HintBar instead of raw monospace strings.
- `cargo build` passes.

### 6. Add Host UI Gallery

Add a developer-only host UI gallery surface that renders every primitive in every important state.

Done when:

- The gallery shows modal shell, rows, buttons, text fields, key chips, badges, and hint bars.
- It includes normal, hover-equivalent, selected, focused, disabled, and danger states where relevant.
- The gallery is reachable by a dev-only command or debug overlay.
- `cargo build` passes.

## Known Gotchas

- egui focus is last-writer-wins within a frame. Text-owning overlays need both one-shot focus after overlay widgets render and post-CentralPanel re-focus.
- Rendering interactive widgets after `request_focus()` can steal the same-frame request.
- Overlay rendering currently happens before `CentralPanel`; app panes can steal focus later in the frame.
- `FocusLayer` entries can survive if cleanup only pops the top layer. Use retain-style cleanup when the source state closes.
- Rows that reserve a background shape before content renders must keep their content inside the same layout scope that owns the row rect.

## Issue Bundle

The work should be filed as focused issues:

1. [#2122](https://github.com/ianjamesburke/PLEXI/issues/2122) - Add host `ListRow` primitives and migrate NotesPicker.
2. [#2123](https://github.com/ianjamesburke/PLEXI/issues/2123) - Add `ModalShell` and `HintBar`.
3. [#2124](https://github.com/ianjamesburke/PLEXI/issues/2124) - Add host `TextField` focus registration.
4. [#2125](https://github.com/ianjamesburke/PLEXI/issues/2125) - Migrate CommandPalette to the host UI kit.
5. [#2126](https://github.com/ianjamesburke/PLEXI/issues/2126) - Migrate QuickNote destination and submenu UI to the host UI kit.
6. [#2127](https://github.com/ianjamesburke/PLEXI/issues/2127) - Add a host UI gallery for design review.

The first issue is the best starting point. It fixes the screenshot bug and tests whether the abstraction is right before the modal/focus work grows.
