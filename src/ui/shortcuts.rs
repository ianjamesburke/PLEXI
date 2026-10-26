use egui::{Color32, Pos2, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

const INTER_COMBO_GAP: f32 = 10.0;

// ── Key token convention ───────────────────────────────────────────────────
// The ONE place that decides how a key is spelled and ordered on screen.
// Every hint surface (HintBar, key_combo*, dialogs, palette, attention queue,
// zoom) feeds raw tokens through `display_tokens`, so ordering and glyphs
// cannot drift between surfaces.
//
// Order: Cmd/Ctrl, Alt, Shift, key.
// Mac glyphs:   ⌘  ⌥  ⇧  Esc  ↵  ⌫  ← ↑ ↓ →
// Linux/Other:  Ctrl  Alt  Shift  Esc  Enter  Backspace  ← ↑ ↓ →

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Modifier {
    Primary,
    /// Mac Control (\u{2303}); folds into `Primary` (Ctrl) off Mac.
    Control,
    Alt,
    Shift,
}

fn modifier_of(token: &str, mac: bool) -> Option<Modifier> {
    match token.to_ascii_lowercase().as_str() {
        "cmd" | "command" | "super" | "\u{2318}" => Some(Modifier::Primary),
        "ctrl" | "control" | "\u{2303}" if mac => Some(Modifier::Control),
        "ctrl" | "control" | "\u{2303}" => Some(Modifier::Primary),
        "alt" | "opt" | "option" | "\u{2325}" => Some(Modifier::Alt),
        "shift" | "\u{21e7}" => Some(Modifier::Shift),
        _ => None,
    }
}

fn modifier_label(m: Modifier, mac: bool) -> &'static str {
    match (m, mac) {
        (Modifier::Primary, true) => "\u{2318}",
        (Modifier::Primary, false) => "Ctrl",
        (Modifier::Control, true) => "\u{2303}",
        (Modifier::Control, false) => "Ctrl",
        (Modifier::Alt, true) => "\u{2325}",
        (Modifier::Alt, false) => "Alt",
        (Modifier::Shift, true) => "\u{21e7}",
        (Modifier::Shift, false) => "Shift",
    }
}

fn key_label(token: &str, mac: bool) -> String {
    match token.to_ascii_lowercase().as_str() {
        "esc" | "escape" => "Esc".into(),
        "enter" | "return" | "\u{21b5}" | "\u{23ce}" | "\u{21a9}" => {
            if mac { "\u{21b5}" } else { "Enter" }.into()
        }
        "backspace" | "delete" | "del" | "\u{232b}" => {
            if mac { "\u{232b}" } else { "Backspace" }.into()
        }
        "up" | "arrowup" | "\u{2191}" => "\u{2191}".into(),
        "down" | "arrowdown" | "\u{2193}" => "\u{2193}".into(),
        "left" | "arrowleft" | "\u{2190}" => "\u{2190}".into(),
        "right" | "arrowright" | "\u{2192}" => "\u{2192}".into(),
        "tab" => "Tab".into(),
        "space" | "\u{2423}" => {
            if mac { "\u{2423}" } else { "Space" }.into()
        }
        _ if token.chars().count() == 1 => token.to_uppercase(),
        _ => token.to_string(),
    }
}

/// Split one raw token into atoms: `"cmd+shift+n"` -> cmd, shift, n and a
/// glued `"\u{2318}C"` -> \u{2318}, C. A lone `+` is a key, not a separator.
fn atoms(token: &str) -> Vec<String> {
    if token.chars().count() > 1 && token.contains('+') && token != "+" {
        return token
            .split('+')
            .filter(|t| !t.is_empty())
            .flat_map(atoms)
            .collect();
    }
    let mut out = Vec::new();
    let mut rest = token;
    while let Some(c) = rest.chars().next() {
        let glyph_mod = matches!(c, '\u{2318}' | '\u{2303}' | '\u{2325}' | '\u{21e7}');
        if glyph_mod && rest.chars().count() > 1 {
            out.push(c.to_string());
            rest = &rest[c.len_utf8()..];
        } else {
            break;
        }
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// Pure chord -> chip labels for the given platform. Accepts tokens in any
/// spelling (`"cmd"`, `"\u{2318}"`, `"Ctrl"`, `"esc"`, `"\u{21b5}"`) and any order;
/// returns modifiers first in Cmd/Ctrl, Alt, Shift order, then the key(s).
pub(crate) fn chord_tokens(keys: &[&str], mac: bool) -> Vec<String> {
    let mut mods: Vec<Modifier> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    for atom in keys.iter().flat_map(|k| atoms(k)) {
        match modifier_of(&atom, mac) {
            Some(m) => {
                if !mods.contains(&m) {
                    mods.push(m);
                }
            }
            None => rest.push(key_label(&atom, mac)),
        }
    }
    mods.sort();
    mods.into_iter()
        .map(|m| modifier_label(m, mac).to_string())
        .chain(rest)
        .collect()
}

pub(crate) fn host_is_mac() -> bool {
    cfg!(target_os = "macos")
}

/// Chip labels for the running platform. Every combo draw/measure goes here.
pub(crate) fn display_tokens(keys: &[&str]) -> Vec<String> {
    chord_tokens(keys, host_is_mac())
}

/// Plain-text spelling of a chord for tooltips and badges that cannot draw
/// chips: glyphs run together on Mac (`\u{2318}\u{21e7}N`), `+`-joined elsewhere.
pub(crate) fn chord_text(keys: &[&str], mac: bool) -> String {
    chord_tokens(keys, mac).join(if mac { "" } else { "+" })
}

pub(crate) fn key_text(keys: &[&str]) -> String {
    chord_text(keys, host_is_mac())
}

fn display_tokens_owned(keys: &[String]) -> Vec<String> {
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    display_tokens(&refs)
}

fn combo_chip_font() -> egui::FontId {
    egui::FontId::monospace(style::TEXT_CAPTION)
}

fn shortcut_key_color(colors: &Colors) -> egui::Color32 {
    colors.text_primary.gamma_multiply(0.78)
}

fn shortcut_label_color(colors: &Colors) -> egui::Color32 {
    colors.text_primary.gamma_multiply(0.70)
}

pub(crate) fn shortcut_hint_label(text: &str, colors: &Colors) -> egui::RichText {
    egui::RichText::new(text)
        .size(style::TEXT_HINT)
        .color(shortcut_label_color(colors))
}

/// Size of one key chip around an already-laid-out label. The width floors at
/// the chip height, so a single glyph stays square rather than reading as a
/// sliver, and again at `KEYCHIP_MIN_W`. Every measure and paint site — the
/// live kit, the app chrome footer, and the headless renderer — goes through
/// here so the three can never drift apart again.
pub(crate) fn chip_size(text_size: Vec2) -> Vec2 {
    let h = text_size.y + style::KEYCHIP_PAD_V * 2.0;
    let w = (text_size.x + style::KEYCHIP_PAD_H * 2.0)
        .max(h)
        .max(style::KEYCHIP_MIN_W);
    Vec2::new(w, h)
}

pub(crate) fn chip_row_height(ui: &egui::Ui) -> f32 {
    let text_size = ui.fonts_mut(|f| {
        f.layout_no_wrap("M".to_string(), combo_chip_font(), Color32::PLACEHOLDER)
            .size()
    });
    chip_size(text_size).y
}

pub(crate) fn key_chip(
    ui: &mut egui::Ui,
    label: &str,
    colors: &Colors,
    font_id: egui::FontId,
) -> egui::Response {
    let fg = shortcut_key_color(colors);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font_id, fg));
    let text_size = galley.size();
    let size = chip_size(text_size);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, crate::ui::style::RADIUS_SMALL, colors.bg_active);
    let text_pos = Pos2::new(
        rect.center().x - text_size.x / 2.0,
        rect.center().y - text_size.y / 2.0,
    );
    crate::ui::snap::galley_snapped(painter, text_pos, galley, fg);
    response
}

// Wrap at the caller's width so platform spellings cannot widen a grid cell.
pub(crate) fn key_combo(ui: &mut egui::Ui, keys: &[&str], colors: &Colors) {
    key_combo_list(ui, &[keys], None, colors);
}

pub(crate) fn key_combo_list(
    ui: &mut egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
    colors: &Colors,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        key_combo_list_inline(ui, combos, trailing, colors);
    });
}

pub(crate) fn key_combo_list_with_body<R>(
    ui: &mut egui::Ui,
    combos: &[&[&str]],
    colors: &Colors,
    add_trailing: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let row_h = chip_row_height(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(row_h);
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, keys) in combos.iter().enumerate() {
            if i > 0 {
                ui.add_space(INTER_COMBO_GAP);
            }
            for (j, key) in display_tokens(keys).iter().enumerate() {
                if j > 0 {
                    ui.add_space(style::KEYCHIP_GAP);
                }
                key_chip(ui, key, colors, combo_chip_font());
            }
        }
        ui.add_space(style::KEYCHIP_DESC_GAP);
        add_trailing(ui)
    })
}

/// Render a key-combo group directly into an existing horizontal row (caller sets
/// `item_spacing.x = 0.0`), returning the painted chip rects and trailing label rect.
pub(crate) fn key_combo_list_inline(
    ui: &mut egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
    colors: &Colors,
) -> (Vec<egui::Rect>, Option<egui::Rect>) {
    let mut chip_rects = Vec::new();
    for (i, keys) in combos.iter().enumerate() {
        if i > 0 {
            ui.add_space(INTER_COMBO_GAP);
        }
        for (j, key) in display_tokens(keys).iter().enumerate() {
            if j > 0 {
                ui.add_space(style::KEYCHIP_GAP);
            }
            chip_rects.push(key_chip(ui, key, colors, combo_chip_font()).rect);
        }
    }
    let label_rect = trailing.filter(|t| !t.is_empty()).map(|text| {
        ui.add_space(style::KEYCHIP_DESC_GAP);
        ui.add(
            egui::Label::new(shortcut_hint_label(text, colors))
                .wrap_mode(egui::TextWrapMode::Extend),
        )
        .rect
    });
    (chip_rects, label_rect)
}

/// Render an owned-string key combo directly into an existing horizontal row.
pub(crate) fn key_combo_owned_inline(
    ui: &mut egui::Ui,
    keys: &[String],
    trailing: Option<&str>,
    colors: &Colors,
) -> (Vec<egui::Rect>, Option<egui::Rect>) {
    let mut chip_rects = Vec::with_capacity(keys.len());
    for (j, key) in display_tokens_owned(keys).iter().enumerate() {
        if j > 0 {
            ui.add_space(style::KEYCHIP_GAP);
        }
        chip_rects.push(key_chip(ui, key, colors, combo_chip_font()).rect);
    }
    let label_rect = trailing.filter(|t| !t.is_empty()).map(|text| {
        ui.add_space(style::KEYCHIP_DESC_GAP);
        ui.add(
            egui::Label::new(shortcut_hint_label(text, colors))
                .wrap_mode(egui::TextWrapMode::Extend),
        )
        .rect
    });
    (chip_rects, label_rect)
}

/// Paint one key chip with its left edge at `left`, vertically centered on
/// `center_y`, without allocating layout space. Returns the chip width.
/// For painter-driven rows that place their own chips (modal action rows,
/// the tab-bar Esc hint).
pub(crate) fn key_chip_painted(
    ui: &egui::Ui,
    left: f32,
    center_y: f32,
    label: &str,
    colors: &Colors,
) -> f32 {
    paint_key_chip(ui.painter(), left, center_y, label, colors).width()
}

/// Painter-only form of [`key_chip_painted`]; returns the chip rect.
pub(crate) fn paint_key_chip(
    painter: &egui::Painter,
    left: f32,
    center_y: f32,
    label: &str,
    colors: &Colors,
) -> egui::Rect {
    let fg = shortcut_key_color(colors);
    let galley = painter.layout_no_wrap(label.to_string(), combo_chip_font(), fg);
    let text_w = galley.size().x;
    let size = chip_size(galley.size());
    let rect = egui::Rect::from_min_size(Pos2::new(left, center_y - size.y / 2.0), size);
    painter.rect_filled(rect, style::RADIUS_SMALL, colors.bg_active);
    let text_pos = Pos2::new(
        rect.center().x - text_w / 2.0,
        rect.min.y + style::KEYCHIP_PAD_V,
    );
    crate::ui::snap::galley_snapped(painter, text_pos, galley, fg);
    rect
}

/// Paint a whole key combo starting at `left`, centered on `center_y`.
/// Returns the total painted width. Pairs with [`key_combo_width`], which
/// measures the same geometry without painting.
pub(crate) fn key_combo_painted(
    ui: &egui::Ui,
    left: f32,
    center_y: f32,
    keys: &[&str],
    colors: &Colors,
) -> f32 {
    let mut x = left;
    for (i, key) in display_tokens(keys).iter().enumerate() {
        if i > 0 {
            x += style::KEYCHIP_GAP;
        }
        x += key_chip_painted(ui, x, center_y, key, colors);
    }
    x - left
}

/// Rendered width of one combo's chips and their gaps. Callers aligning a
/// column of combos (modal action rows) measure every combo with this and
/// lay the labels out past the widest.
pub(crate) fn key_combo_width(ui: &egui::Ui, keys: &[&str]) -> f32 {
    key_combo_list_width(ui, &[keys], None)
}

pub(crate) fn key_combo_list_width(
    ui: &egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
) -> f32 {
    let measure = |text: &str, font: egui::FontId| {
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER))
            .size()
    };
    let mut w = 0.0;
    for (i, keys) in combos.iter().enumerate() {
        if i > 0 {
            w += INTER_COMBO_GAP;
        }
        for (j, key) in display_tokens(keys).iter().enumerate() {
            if j > 0 {
                w += style::KEYCHIP_GAP;
            }
            w += chip_size(measure(key, combo_chip_font())).x;
        }
    }
    if let Some(text) = trailing.filter(|t| !t.is_empty()) {
        w += style::KEYCHIP_DESC_GAP;
        w += measure(text, egui::FontId::proportional(style::TEXT_HINT)).x;
    }
    w
}

pub(crate) fn key_combo_owned_width(ui: &egui::Ui, keys: &[String], trailing: Option<&str>) -> f32 {
    let measure = |text: &str, font: egui::FontId| {
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER))
            .size()
    };
    let mut w = 0.0;
    for (j, key) in display_tokens_owned(keys).iter().enumerate() {
        if j > 0 {
            w += style::KEYCHIP_GAP;
        }
        w += chip_size(measure(key, combo_chip_font())).x;
    }
    if let Some(text) = trailing.filter(|t| !t.is_empty()) {
        w += style::KEYCHIP_DESC_GAP;
        w += measure(text, egui::FontId::proportional(style::TEXT_HINT)).x;
    }
    w
}

#[cfg(test)]
mod tests {
    use super::{chord_text, chord_tokens};

    fn toks(keys: &[&str], mac: bool) -> Vec<String> {
        chord_tokens(keys, mac)
    }

    #[test]
    fn mac_orders_cmd_alt_shift_key_with_glyphs() {
        let want = ["\u{2318}", "\u{2325}", "\u{21e7}", "N"];
        assert_eq!(toks(&["shift", "alt", "cmd", "n"], true), want);
        assert_eq!(toks(&["cmd+shift+alt+n"], true), want);
        assert_eq!(
            toks(&["\u{21e7}", "\u{2318}", "n"], true),
            ["\u{2318}", "\u{21e7}", "N"]
        );
    }

    #[test]
    fn linux_shows_ctrl_and_alt_words() {
        assert_eq!(
            toks(&["cmd", "alt", "shift", "n"], false),
            ["Ctrl", "Alt", "Shift", "N"]
        );
        assert_eq!(
            toks(&["\u{2318}", "\u{2325}", "p"], false),
            ["Ctrl", "Alt", "P"]
        );
        assert_eq!(toks(&["Opt", "Ctrl", "x"], false), ["Ctrl", "Alt", "X"]);
    }

    #[test]
    fn special_keys_share_one_glyph_set() {
        assert_eq!(toks(&["esc"], true), ["Esc"]);
        assert_eq!(toks(&["Escape"], false), ["Esc"]);
        assert_eq!(toks(&["Enter"], true), ["\u{21b5}"]);
        assert_eq!(toks(&["\u{23ce}"], false), ["Enter"]);
        assert_eq!(toks(&["cmd", "Backspace"], true), ["\u{2318}", "\u{232b}"]);
        assert_eq!(toks(&["cmd", "Backspace"], false), ["Ctrl", "Backspace"]);
        assert_eq!(toks(&["up", "\u{2193}"], true), ["\u{2191}", "\u{2193}"]);
    }

    #[test]
    fn mac_control_is_distinct_and_folds_to_ctrl_elsewhere() {
        assert_eq!(
            toks(&["\u{2325}", "\u{2303}", "\u{2318}", "h"], true),
            ["\u{2318}", "\u{2303}", "\u{2325}", "H"]
        );
        assert_eq!(toks(&["\u{2318}", "\u{2303}", "h"], false), ["Ctrl", "H"]);
        assert_eq!(toks(&["\u{21a9}"], true), ["\u{21b5}"]);
        assert_eq!(toks(&["\u{2423}"], false), ["Space"]);
    }

    #[test]
    fn chord_text_joins_per_platform() {
        assert_eq!(chord_text(&["cmd", "shift", "n"], true), "\u{2318}\u{21e7}N");
        assert_eq!(chord_text(&["cmd", "shift", "n"], false), "Ctrl+Shift+N");
        assert_eq!(chord_text(&["cmd", "down"], false), "Ctrl+\u{2193}");
    }

    #[test]
    fn glued_glyph_tokens_split_and_plus_key_survives() {
        assert_eq!(toks(&["\u{2318}C"], true), ["\u{2318}", "C"]);
        assert_eq!(toks(&["\u{2318}C"], false), ["Ctrl", "C"]);
        assert_eq!(toks(&["cmd", "+"], true), ["\u{2318}", "+"]);
    }
}
