use super::*;
use crate::ui::theme::{
    contrast, luminance, preset_colors, preset_names, setup_fonts, setup_style,
};

fn widget_tree() -> UiTree {
    crate::host::wasm_python::decode_ui_tree(&serde_json::json!({
        "root":0,"nodes":[
            {"id":0,"key":"root","data":{"type":"Column","children":[1,3,4,5,6,7,8,9,10,12,13],"gap":8.0}},
            {"id":1,"key":"card","data":{"type":"Padding","card":true,"child":2,"top":12,"bottom":12,"left":12,"right":12}},
            {"id":2,"key":"card-text","data":{"type":"Text","text":"Default Card"}},
            {"id":3,"key":"ghost","data":{"type":"Button","label":"Ghost task","style":"ghost","on_click":"pick"}},
            {"id":4,"key":"primary","data":{"type":"Button","label":"Primary task","style":"primary","on_click":"pick"}},
            {"id":5,"key":"secondary","data":{"type":"Button","label":"Secondary task","style":"secondary","on_click":"pick"}},
            {"id":6,"key":"danger","data":{"type":"Button","label":"Danger task","style":"danger","on_click":"pick"}},
            {"id":7,"key":"disabled","data":{"type":"Button","label":"Disabled task","style":"ghost","disabled":true,"on_click":"pick"}},
            {"id":8,"key":"badge","data":{"type":"Badge","text":"Neutral badge","color":"neutral"}},
            {"id":9,"key":"input","data":{"type":"TextInput","value":"Input value","placeholder":"Search","on_change":"edit"}},
            {"id":10,"key":"list","data":{"type":"ListView","items":[11],"selected":0,"on_select":"pick"}},
            {"id":11,"key":"list-text","data":{"type":"Text","text":"Selected row"}},
            {"id":12,"key":"hint","data":{"type":"TextInput","value":"","placeholder":"Input placeholder","on_change":"hint"}},
            {"id":13,"key":"footer","data":{"type":"FooterKeys","entries":[{"keys":["Enter"],"description":"Open"}]}}
        ]
    }).to_string()).unwrap()
}

fn painted(tree: &UiTree, colors: &Colors) -> Vec<egui::epaint::ClippedShape> {
    let ctx = egui::Context::default();
    setup_fonts(&ctx);
    setup_style(&ctx, colors);
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 900.0),
            )),
            ..Default::default()
        },
        |ui| {
            render_ui_tree_with_surface(ui, tree, colors, None, None, 0);
        },
    )
    .shapes
}

fn assert_paint_contrast(shapes: &[egui::epaint::ClippedShape], label: &str) {
    let mut fills = Vec::new();
    let mut count = 0;
    for clipped in shapes {
        match &clipped.shape {
            egui::Shape::Rect(r) => fills.push((r.rect, r.fill)),
            egui::Shape::Text(t) if !t.galley.job.text.is_empty() => {
                let point = t.pos + t.galley.size() / 2.0;
                if !clipped.clip_rect.contains(point) {
                    continue;
                }
                let bg = fills
                    .iter()
                    .filter(|(r, _)| r.contains(point))
                    .fold(Color32::BLACK, |bg, (_, fill)| bg.blend(*fill));
                let fg = t.override_text_color.unwrap_or_else(|| {
                    let c = t.galley.job.sections[0].format.color;
                    if c == Color32::PLACEHOLDER {
                        t.fallback_color
                    } else {
                        c
                    }
                });
                let ratio = contrast(luminance(bg.blend(fg)), luminance(bg));
                assert!(
                    ratio >= 4.5,
                    "{label}: {:?} foreground={fg:?} fill={bg:?} contrast={ratio}",
                    t.galley.job.text
                );
                count += 1;
            }
            _ => {}
        }
    }
    assert!(count >= 10, "{label}: only {count} text shapes checked");
}

#[test]
fn app_default_text_contrast_preset_sweep() {
    for name in preset_names() {
        let colors = Colors::from_config(&preset_colors(name).unwrap());
        assert_paint_contrast(&painted(&widget_tree(), &colors), name);
    }
}

#[test]
fn app_default_text_contrast_light_overrides() {
    let mut cfg = preset_colors("plexi-night").unwrap();
    cfg.terminal_bg = Some("#e3e3f0".into());
    cfg.bg_active = Some("#b0b0d0".into());
    let colors = Colors::from_config(&cfg);
    assert_paint_contrast(
        &painted(&widget_tree(), &colors),
        "night with light overrides",
    );
}

#[test]
fn app_default_text_contrast_mode_follows_resolved_background() {
    for (preset, background, expected_dark) in [
        ("plexi-night", "#eeeeff", false),
        ("plexi-day", "#101020", true),
    ] {
        let mut cfg = preset_colors(preset).unwrap();
        cfg.terminal_bg = Some(background.into());
        let colors = Colors::from_config(&cfg);
        let ctx = egui::Context::default();
        setup_style(&ctx, &colors);
        assert_eq!(
            ctx.global_style().visuals.dark_mode,
            expected_dark,
            "{preset} with {background}"
        );
    }
}

#[test]
fn app_default_text_contrast_preserves_authored_colors() {
    let tree = crate::host::wasm_python::decode_ui_tree(&serde_json::json!({
        "root": 0, "nodes": [
            {"id":0,"key":"root","data":{"type":"Column","children":[1,2]}},
            {"id":1,"key":"explicit","data":{"type":"Text","text":"Authored text","color":"#eeeeee"}},
            {"id":2,"key":"canvas","data":{"type":"Canvas","width":300,"height":100,"grow":false,"commands":[
                {"type":"text","x":10,"y":20,"text":"Authored canvas","size":14,"color":"#eeeeee"}
            ]}}
        ]
    }).to_string()).unwrap();
    let colors = Colors::from_config(&preset_colors("plexi-day").unwrap());
    let shapes = painted(&tree, &colors);
    for label in ["Authored text", "Authored canvas"] {
        let t = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == label => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {label}"));
        let fg = t
            .override_text_color
            .unwrap_or(t.galley.job.sections[0].format.color);
        assert_eq!(
            fg,
            Color32::from_gray(238),
            "{label} must retain its authored color"
        );
        assert!(contrast(luminance(fg), luminance(colors.terminal_bg)) < 4.5,
            "negative control must stay low contrast; default correction must not change explicit artwork");
    }
}
