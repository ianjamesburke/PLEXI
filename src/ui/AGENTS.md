# src/ui — Host UI Kit Contract

**Read before editing anything under `src/ui/`:** this file, plus the root `AGENTS.md`.

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

## Traps

- **TextEdit focus — two-layer problem.** `request_focus` is last-caller-wins per frame. Modal overlays lose focus to CentralPanel panes because overlays render first in `update()`. Fix requires both: (1) a one-shot `*_focus_requested` flag — fire `request_focus` after ALL overlay widgets render, then set the flag; (2) an every-frame re-request in `update()` after CentralPanel, mirroring the `palette_search`/`quick_note_text` re-focus block. Either fix alone is insufficient.
- **New theme presets must pass the WCAG contrast test.** `text_on_is_legible_on_every_preset_accent_and_danger` in `src/ui/theme.rs` asserts ≥3:1 on `accent` and `danger` fills for every entry in `preset_names()`. Add new presets to `preset_names()`, `canonical_preset_name()`, and `preset_colors()` — verify `text_on()` returns a legible color if the accent/danger is mid-luminance.
- **Canvas apps bypass the host's WCAG check.** `ctx.theme.muted` (`#6c7086`) is only ~2.6:1 against bg — fails WCAG for body text. Use `ctx.theme.fg` for primary labels, a deliberate readable subtext (e.g. `#a6adc8`) for captions. `dim()` is for fills/tracks/glows, never text.

## Rules

- Components own geometry. Callers should not hand-place modal title gaps, row baselines, scroll-body heights, focus rings, or text styles.
- Do not add hard-coded spacing, text sizes, radii, or button heights in overlay code. Add a token to `style.rs` or a component API here.
- Do not render host chrome with direct `RichText::new(...).size(...)` unless this module has no matching semantic helper yet. Add the helper first.
- New modal/body layout behavior belongs on `ModalShell`, not in one overlay.
- Add or update a `PlexiUiHarness` smoke/screenshot test when changing a visible host UI primitive.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.
