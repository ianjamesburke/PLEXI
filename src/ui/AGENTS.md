# src/ui — Host UI Kit Contract

**Read before editing anything under `src/ui/`:** this file, plus the root `AGENTS.md`.

For new host-level UI chrome work, read [`docs/prm/host-ui-kit.md`](../../docs/prm/host-ui-kit.md) first.

## Primitives (use these, don't re-roll)

Host overlays should use these instead of raw egui layout wherever a primitive exists:

- `overlay::ModalShell` for modal frame, scrim, title, body, scroll body, and dismissal.
- `typography` for modal titles, section labels, body text, captions, and muted text.
- `list::ListRow`, `row`, `text_field::TextField`, `button`, `hints::HintBar`, and `surface` for repeated chrome.

## Design Tokens (`src/style.rs`)

Spacing scale (`SPACE_SM/MD/XL`), typography scale (`TEXT_HINT/CAPTION/BODY/TITLE_XL`), corner radii (`RADIUS_MD/LG`), modal widths, button heights, overlay chrome. Use these everywhere. Never hard-code magic numbers.

## Reusable Widgets (`src/widgets.rs`)

- `key_chip(ui, label, colors)` — single keyboard key as a styled rounded-rect chip.
- `key_combo(ui, keys, colors)` — sequence of `key_chip`s with `INTRA_COMBO_GAP`.
- `key_combo_list(ui, combos, trailing, colors)` — multiple combos inline with trailing description. **Use this for any shortcut hint row.** Do not render shortcuts as plain `Label` text.

## Overlay Layout Primitives

- `section_header(ui, label, is_active, colors)` — group/context label.
- `pane_type_badge(ui, kind, colors)` — `"term"` or `"app"` chip.
- `status_chip(ui, status, colors)` — status color mapping (`"running"` → accent, `"crashed"` → danger, etc.).
- `description_label(ui, text, colors)` — single-line hint label. **Always wrap in `ui.scope()` and `set_max_width(n)` inside the scope** to avoid corrupting sibling layout.

## Rules

- Components own geometry. Callers should not hand-place modal title gaps, row baselines, scroll-body heights, focus rings, or text styles.
- Do not add hard-coded spacing, text sizes, radii, or button heights in overlay code. Add a token to `style.rs` or a component API here.
- Do not render host chrome with direct `RichText::new(...).size(...)` unless this module has no matching semantic helper yet. Add the helper first.
- New modal/body layout behavior belongs on `ModalShell`, not in one overlay.
- Add or update a `PlexiUiHarness` smoke/screenshot test when changing a visible host UI primitive.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.
