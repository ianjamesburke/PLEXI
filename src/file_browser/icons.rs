use crate::theme::Colors;
use egui::{Color32, CornerRadius, Stroke, StrokeKind};
use super::helpers::Entry;

pub(crate) enum FileIconKind {
    Image,
    Audio,
    Markdown,
    Text,
    Code,
    Config,
    Pdf,
    Archive,
    Generic,
}

pub(crate) fn file_icon_kind(entry: &Entry) -> FileIconKind {
    if entry.is_image { return FileIconKind::Image; }
    if entry.is_audio { return FileIconKind::Audio; }
    let Some(ext) = entry.path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) else {
        return FileIconKind::Generic;
    };
    match ext.as_str() {
        "md" | "markdown" | "mdx" | "rst" => FileIconKind::Markdown,
        "txt" | "rtf" | "log" => FileIconKind::Text,
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "java" | "swift" | "kt"
        | "c" | "h" | "cpp" | "hpp" | "sh" | "zsh" | "bash" | "fish" | "lua" | "rb" => FileIconKind::Code,
        "toml" | "yaml" | "yml" | "json" | "jsonc" | "json5" | "ini" | "cfg" | "conf"
        | "env" | "plist" => FileIconKind::Config,
        "pdf" => FileIconKind::Pdf,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" => FileIconKind::Archive,
        _ => FileIconKind::Generic,
    }
}

pub(crate) fn paint_entry_icon(painter: &egui::Painter, rect: egui::Rect, entry: &Entry, colors: &Colors) {
    if entry.is_dir {
        let tab = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 2.0),
            egui::vec2(rect.width() * 0.45, rect.height() * 0.3),
        );
        let body = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + rect.height() * 0.25),
            egui::vec2(rect.width() - 2.0, rect.height() * 0.7),
        );
        painter.rect_filled(tab, CornerRadius::same(2), colors.accent.gamma_multiply(0.7));
        painter.rect_filled(body, CornerRadius::same(2), colors.accent.gamma_multiply(0.9));
        return;
    }

    let sheet = rect.shrink(1.0);
    let fold = (sheet.width().min(sheet.height()) * 0.30).clamp(4.0, 18.0);
    let stroke_w = (sheet.width().min(sheet.height()) * 0.10).clamp(1.0, 2.4);

    painter.rect_filled(sheet, CornerRadius::same(2), colors.text_dim.gamma_multiply(0.34));
    painter.rect_stroke(sheet, CornerRadius::same(2), Stroke::new(1.0, colors.border), StrokeKind::Inside);
    let fold_poly = vec![
        egui::pos2(sheet.right() - fold, sheet.top()),
        egui::pos2(sheet.right(), sheet.top()),
        egui::pos2(sheet.right(), sheet.top() + fold),
    ];
    painter.add(egui::Shape::convex_polygon(fold_poly, colors.bg_active.gamma_multiply(0.75), Stroke::new(1.0, colors.border)));

    let x = |t: f32| sheet.left() + sheet.width() * t;
    let y = |t: f32| sheet.top() + sheet.height() * t;
    let kind = file_icon_kind(entry);

    match kind {
        FileIconKind::Image => {
            let sky = Color32::from_rgb(0x89, 0xb4, 0xfa);
            let points = [(0.18, 0.78), (0.36, 0.52), (0.54, 0.72), (0.80, 0.42)];
            for w in points.windows(2) {
                painter.line_segment(
                    [egui::pos2(x(w[0].0), y(w[0].1)), egui::pos2(x(w[1].0), y(w[1].1))],
                    Stroke::new(stroke_w, sky),
                );
            }
            painter.circle_filled(egui::pos2(x(0.76), y(0.26)), (sheet.width().min(sheet.height()) * 0.09).max(1.5), sky.gamma_multiply(0.9));
        }
        FileIconKind::Audio => {
            let c = Color32::from_rgb(0xa6, 0xe3, 0xa1);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x(0.26), y(0.50)), egui::pos2(x(0.36), y(0.40)),
                    egui::pos2(x(0.47), y(0.40)), egui::pos2(x(0.47), y(0.68)),
                    egui::pos2(x(0.36), y(0.68)), egui::pos2(x(0.26), y(0.58)),
                ],
                c,
                Stroke::new(0.0, Color32::TRANSPARENT),
            ));
            painter.line_segment([egui::pos2(x(0.56), y(0.44)), egui::pos2(x(0.66), y(0.54))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.56), y(0.64)), egui::pos2(x(0.66), y(0.54))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.68), y(0.38)), egui::pos2(x(0.80), y(0.54))], Stroke::new(stroke_w, c.gamma_multiply(0.9)));
            painter.line_segment([egui::pos2(x(0.68), y(0.70)), egui::pos2(x(0.80), y(0.54))], Stroke::new(stroke_w, c.gamma_multiply(0.9)));
        }
        FileIconKind::Markdown | FileIconKind::Text => {
            let c = Color32::from_rgb(0xf9, 0xe2, 0xaf);
            painter.line_segment([egui::pos2(x(0.28), y(0.74)), egui::pos2(x(0.72), y(0.30))], Stroke::new(stroke_w * 1.15, c));
            painter.add(egui::Shape::convex_polygon(
                vec![egui::pos2(x(0.70), y(0.26)), egui::pos2(x(0.80), y(0.20)), egui::pos2(x(0.74), y(0.30))],
                c, Stroke::new(0.0, Color32::TRANSPARENT),
            ));
            if matches!(kind, FileIconKind::Markdown) {
                painter.line_segment([egui::pos2(x(0.26), y(0.26)), egui::pos2(x(0.54), y(0.26))], Stroke::new(stroke_w, c.gamma_multiply(0.95)));
            }
        }
        FileIconKind::Code => {
            let c = Color32::from_rgb(0x94, 0xe2, 0xd5);
            painter.line_segment([egui::pos2(x(0.38), y(0.34)), egui::pos2(x(0.24), y(0.52))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.24), y(0.52)), egui::pos2(x(0.38), y(0.70))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.62), y(0.34)), egui::pos2(x(0.76), y(0.52))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.76), y(0.52)), egui::pos2(x(0.62), y(0.70))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.52), y(0.34)), egui::pos2(x(0.46), y(0.70))], Stroke::new(stroke_w * 0.9, c.gamma_multiply(0.85)));
        }
        FileIconKind::Config => {
            let c = Color32::from_rgb(0xb4, 0xbe, 0xfe);
            painter.line_segment([egui::pos2(x(0.22), y(0.38)), egui::pos2(x(0.78), y(0.38))], Stroke::new(stroke_w, c));
            painter.circle_filled(egui::pos2(x(0.42), y(0.38)), (stroke_w * 1.2).max(1.6), c);
            painter.line_segment([egui::pos2(x(0.22), y(0.56)), egui::pos2(x(0.78), y(0.56))], Stroke::new(stroke_w, c));
            painter.circle_filled(egui::pos2(x(0.62), y(0.56)), (stroke_w * 1.2).max(1.6), c);
        }
        FileIconKind::Pdf => {
            let c = Color32::from_rgb(0xf3, 0x8b, 0xa8);
            let band = egui::Rect::from_min_size(
                egui::pos2(x(0.16), y(0.20)),
                egui::vec2(sheet.width() * 0.68, sheet.height() * 0.20),
            );
            painter.rect_filled(band, CornerRadius::same(2), c.gamma_multiply(0.95));
            painter.text(band.center(), egui::Align2::CENTER_CENTER, "PDF", egui::FontId::proportional((sheet.height() * 0.18).max(6.0)), Color32::from_rgb(0x1e, 0x1e, 0x2e));
        }
        FileIconKind::Archive => {
            let c = Color32::from_rgb(0xfa, 0xb3, 0x87);
            let box_rect = egui::Rect::from_min_size(egui::pos2(x(0.26), y(0.30)), egui::vec2(sheet.width() * 0.48, sheet.height() * 0.46));
            painter.rect_stroke(box_rect, CornerRadius::same(2), Stroke::new(stroke_w, c), StrokeKind::Inside);
            painter.line_segment([egui::pos2(box_rect.center().x, box_rect.top()), egui::pos2(box_rect.center().x, box_rect.bottom())], Stroke::new(stroke_w * 0.9, c));
            painter.line_segment([egui::pos2(box_rect.left(), box_rect.center().y), egui::pos2(box_rect.right(), box_rect.center().y)], Stroke::new(stroke_w * 0.9, c));
        }
        FileIconKind::Generic => {
            let c = colors.text_primary.gamma_multiply(0.8);
            painter.line_segment([egui::pos2(x(0.24), y(0.38)), egui::pos2(x(0.70), y(0.38))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.24), y(0.58)), egui::pos2(x(0.60), y(0.58))], Stroke::new(stroke_w, c));
        }
    }
}
