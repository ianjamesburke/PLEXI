# src/ui — Host UI Kit Contract

**Read before editing anything under `src/ui/`:** this file, plus the root `AGENTS.md`.

## Primitives (use these, don't re-roll)

Host overlays should use these instead of raw egui layout wherever a primitive exists:

- Buttons: `button::KitButton` is the one button (default `Primary`/`Md`; `ButtonKind`, `ButtonSize`). Every host, toast and plugin button is painted by it. `Primary` is the accent-filled, strongest button in every theme. `Ghost` has no fill and a faint outline at rest, with the theme hover fill; menu items (`menu_item`) and compact glyphs (`toolbar_button`) drop the outline. `selected(true)` marks the active member of a group with the active fill and an accent outline, so a selected item never reads as Primary. Labels are `&str`; request `text_size`/`text_color`/`monospace` explicitly. `KitButton::natural_width` is the one width measurement; measure and paint from it. Raw egui buttons are banned by clippy `disallowed-methods` in `clippy.toml` (CI `rust-host.yml` clippy job and `just clippy`/`just pr-candidate`); only `KitButton::ui` and the stint 1209 text editor `#[expect]`s construct one.
- `dropdown::select_value` for every single-choice dropdown: the kit owns trigger sizing and option geometry, egui owns popup placement and dismissal.
- `overlay::ModalShell` for modal frame, scrim, title, body, scroll body, and dismissal.
- `typography` for modal titles, section labels, body text, captions, and muted text.
- `list::ListRow`, `row`, `text_field::TextField`, `button`, `hints::HintBar`, and `surface` for repeated chrome.
- `badge::badge` / `badge::badge_size` / `badge::paint_badge` for every pill badge — the WIT `Badge` node, the sidebar notification count, the terminal outside-workspace tag. Owns padding, radius, label font, and the width floor.
- `text::elide` / `text::elided_galley` for any label that has to fit a width. Measured against the real font, `Side::Leading` for paths, `Side::Middle` for filenames/UUIDs/identifiers with distinguishing suffixes, and `Side::Trailing` for prose titles/descriptions, always U+2026. Never truncate by character count.
- `sidebar_row::SidebarRow` for context-list rows. Two tiers, the same convention `ListRow` uses for a secondary line: identity on top (index gutter, name, close action), pane state below (per-window pip capsules, overflow count, notification badge, root path). The active root additionally carries a wrapped ancestry breadcrumb above aligned pip rows; child levels are never numbered or draggable. `RowGeometry` measures breadcrumbs, strips and their portal connections together; never hand-place a piece of a sidebar row. Disclosure settles immediately, including keyboard and reduced-motion paths.

## Design Tokens (`src/ui/style.rs`)

Spacing scale (`SPACE_SM/MD/XL`), typography scale (`TEXT_HINT/CAPTION/BODY/TITLE_XL`), corner radii (one scale: `RADIUS_SMALL/MEDIUM/LARGE/PILL`; literal radii outside `style.rs` fail `radius_guard_tests`), modal widths, button heights, overlay chrome. Use these everywhere. Never hard-code magic numbers.

## Bounded composition

`bounded::allocate` owns finite row budgets. Measure complete required actions and their minimum hit targets before optional metadata and flexible text. Paint and hit-test from the allocated rectangles. Use `ToastShell::action`, `PaneHeader`, `ListRow`, `AppChrome`, and `ActionModal` for their respective surfaces. When required content cannot fit, reflow it, expose a named overflow action, or enforce the container minimum; never accept an empty action label as successful elision. Optional text may elide or deliberately hide; paths and identifiers keep the elision policies above.

Resolve text with `foreground::Surface` against the composited painted fill, including hover, selection and opacity; disabled (inactive) text is exempt from `Role::SmallText` (WCAG 1.4.3) but must still clear `layout_report::INACTIVE_TEXT_MIN` (2:1). Small text uses `Role::SmallText`; meaningful non-text control marks use `Role::ControlMark`. A token alone does not establish contrast.

Instrument migrated surfaces with `layout_report::Capture`, recording allocated text slots and required actions. Assertions inspect emitted paint as well as allocations. Only explicitly scrolling content may declare viewport clipping; a clip rectangle is not a fitting policy. Follow the matrix and screenshot contract in `src/testing/TESTING.md`.

`layout_surfaces.json` is the surface census and fixture registry. An explicit unresolved gap records debt, not passing coverage. `layout_raw_sites.json` retains the legacy raw-call inventory; new raw layout or painter calls require a scoped rationale and a registered fixture in its `approved` entries. Custom canvas code remains usable under that explicit responsibility. Run `just check-ui-layout`; do not refresh the legacy baseline to bypass a new-site failure. This internal host contract does not certify arbitrary canvas or external plugin composition.

## Reusable Widgets (`src/ui/shortcuts.rs`)

- `key_chip(ui, label, colors)` — single keyboard key as a styled rounded-rect chip.
- `key_combo(ui, keys, colors)` — sequence of `key_chip`s with `INTRA_COMBO_GAP`.
- `key_combo_list(ui, combos, trailing, colors)` — multiple combos inline with trailing description. **Use this for any shortcut hint row.** Do not render shortcuts as plain `Label` text.
- `display_tokens` / `chord_tokens(keys, mac)` / `key_text` — the one key-spelling convention (order Cmd/Ctrl, Alt, Shift, key; Mac glyphs, Ctrl/Alt words elsewhere). Every combo function routes through it, so pass raw tokens (`"cmd"`, `"Esc"`, `"Enter"`); use `key_text` for tooltips/badges that cannot draw chips. Never hand-type `⌘`/`Cmd+` in a hint.
- `chip_size(text_size)` — the one chip geometry. Every measure and paint site, headless included, sizes a chip through it; do not re-derive it from the padding tokens.

## Overlay Layout Primitives

- `section_header(ui, label, is_active, colors)` — group/context label.
- `pane_type_badge(ui, kind, colors)` — `"term"` or `"app"` chip.
- `status_chip(ui, status, colors)` — status color mapping (`"running"` → accent, `"crashed"` → danger, etc.).
- `description_label(ui, text, colors)` — single-line hint label. **Always wrap in `ui.scope()` and `set_max_width(n)` inside the scope** to avoid corrupting sibling layout.

## Traps

- **The sidebar Needs you badge reads `PermissionMonitor::loaded`.** Paint must not call `for_profile`; that opens a profile store on a frame where nothing is waiting. Expiry runs from `App::logic`, so a hidden window still auto-denies.
- **Never call egui `request_focus`/`surrender_focus` directly** (clippy `disallowed-methods` enforces this). Egui focus is a per-frame projection of the host-derived `InputOwner` (stint 0429): register a widget's id with `crate::ui::focus` (`TextField::surface`, `register_default_text_surface`, `claim_text_surface`) and the post-frame reconciler in `src/app/input_owner.rs` grants/surrenders focus. Overlays get their field auto-focused by registering it as the default surface of their `FocusKind`; panes register under `SurfaceKey::Pane(pane_id)` from `AppRenderContext::pane_id`.
- **New theme presets must pass the WCAG contrast test.** `text_on_is_legible_on_every_preset_accent_and_danger` in `src/ui/theme.rs` asserts ≥3:1 on `accent` and `danger` fills for every entry in `preset_names()`. Add new presets to `preset_names()`, `canonical_preset_name()`, and `preset_colors()` — verify `text_on()` returns a legible color if the accent/danger is mid-luminance.
- **Canvas apps bypass the host's WCAG check.** `ctx.theme.muted` (`#6c7086`) is only ~2.6:1 against bg — fails WCAG for body text. Use `ctx.theme.fg` for primary labels, a deliberate readable subtext (e.g. `#a6adc8`) for captions. `dim()` is for fills/tracks/glows, never text.
- **`setup_fonts` only binds the `ui-medium` family on the *next* frame.** `set_fonts` is applied at the following `begin_frame`, so laying out any `font_medium`/`ui-medium` text (declarative Button, ListRow) on the same frame the font is registered panics in epaint ("FontFamily::Name(\"ui-medium\") is not bound"). The live host never hits this (fonts set once at startup), but offscreen rasterizers must apply fonts on a content-less first frame before rendering — see `render_ui_tree_to_png` in `src/host/wasm_render.rs`. Canvas-only trees (e.g. `calc`) dodge it because they never lay out `ui-medium` text.

## Rules

- Components own geometry. Callers should not hand-place modal title gaps, row baselines, scroll-body heights, focus rings, or text styles.
- Do not add hard-coded spacing, text sizes, radii, or button heights in overlay code. Add a token to `style.rs` or a component API here.
- Do not render host chrome with direct `RichText::new(...).size(...)` unless this module has no matching semantic helper yet. Add the helper first.
- New modal/body layout behavior belongs on `ModalShell`, not in one overlay.
- Add or update a `PlexiUiHarness` smoke/screenshot test when changing a visible host UI primitive.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.


## Motion and elevation

| Frequency / action | Motion |
|---|---|
| Frequent keyboard actions (100+/day): modal invocation, pane focus, list cursor, scroll-to-row | Instant, zero frames; keep `ModalShell.fade_in(false)` and `.animated(false)` |
| Pointer button press | Immediate active fill and `scale(0.97)` about the center; retain the hit rectangle |
| Pointer release | `DURATION_PRESS_MS`, using `ease_out_cubic` |
| Disclosure and copy feedback | `DURATION_MICRO_MS`, using `ease_out_cubic` |
| Toast motion, when introduced | `DURATION_TOAST_MS` |

Use `style::ease_out_cubic` (cubic-bezier(0.23, 1, 0.32, 1)) for micro-motion. Never use accelerating entrance easing or durations above 300ms. Never `scale(0)`. A zero egui `animation_time` opts out of interpolation; deterministic test builds also bypass it. Press feedback still appears immediately. Use `hairline_border` and `elevation_shadow` for floating shells.

Text elision preserves grapheme boundaries and collapses line breaks. Use `format_count_plural` for count labels, `compact_count` for bounded badges, and `tabular_count` with a capacity suited to the domain to reserve a monospace counter width.
