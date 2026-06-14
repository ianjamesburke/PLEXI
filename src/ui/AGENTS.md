# Host UI Kit Contract

This directory is the Plexi host UI kit. Before adding or changing host chrome,
read `docs/prm/host-ui-kit.md`.

Host overlays should use these primitives instead of raw egui layout wherever a
primitive exists:

- `overlay::ModalShell` for modal frame, scrim, title, body, scroll body, and dismissal.
- `typography` for modal titles, section labels, body text, captions, and muted text.
- `list::ListRow`, `row`, `text_field::TextField`, `button`, `hints::HintBar`, and
  `surface` for repeated chrome.

Rules:

- Components own geometry. Callers should not hand-place modal title gaps,
  row baselines, scroll-body heights, focus rings, or text styles.
- Do not add hard-coded spacing, text sizes, radii, or button heights in overlay
  code. Add a token to `style.rs` or a component API here.
- Do not render host chrome with direct `RichText::new(...).size(...)` unless
  this module has no matching semantic helper yet. Add the helper first.
- New modal/body layout behavior belongs on `ModalShell`, not in one overlay.
- Add or update a `PlexiUiHarness` smoke/screenshot test when changing a visible
  host UI primitive.
