//! Guard: corner radii come from `ui::style` tokens, never numeric literals.

use std::path::{Path, PathBuf};

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn is_exempt(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let p = path.to_string_lossy();
    name == "style.rs" || name == "theme.rs" || name.contains("test") || p.contains("/tests/")
}

/// True when `needle(` in `line` is followed directly by a numeric literal.
fn has_literal_call(line: &str, needle: &str) -> bool {
    let mut rest = line;
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len()..];
        if after
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return true;
        }
        rest = after;
    }
    false
}

#[test]
fn no_literal_corner_radii_outside_style() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    assert!(!files.is_empty(), "no sources found under {root:?}");
    let mut offenders = Vec::new();
    for path in files.iter().filter(|p| !is_exempt(p)) {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if ["CornerRadius::same(", "Rounding::same(", ".corner_radius("]
                .iter()
                .any(|needle| has_literal_call(code, needle))
            {
                offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use ui::style radius tokens, not literals:\n{}",
        offenders.join("\n")
    );
}

/// True when `block` contains a `rect_filled(` of a rect built with
/// `Rect::from_min_size(<something>.min, Vec2::new(<width <= 6.0 or STRIP/MARKER const>, <...>.height()))`.
fn is_left_edge_bar_marker(block: &str) -> bool {
    if !(block.contains("from_min_size(") && block.contains(".min") && block.contains(".height()"))
    {
        return false;
    }
    let after = if let Some(idx) = block.find("Vec2::new(") {
        &block[idx + "Vec2::new(".len()..]
    } else if let Some(idx) = block.find("vec2(") {
        &block[idx + "vec2(".len()..]
    } else {
        return false;
    };
    let first_arg = after.split(',').next().unwrap_or("").trim();
    if first_arg.contains("STRIP") || first_arg.contains("MARKER") {
        return true;
    }
    first_arg
        .trim_end_matches("_f32")
        .parse::<f32>()
        .is_ok_and(|w| w > 0.0 && w <= 6.0)
}

#[test]
fn no_left_edge_bar_markers_on_cards() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for subdir in [
        "src/ui",
        "src/app",
        "src/spatial",
        "src/overlays",
        "src/file_browser",
    ] {
        rs_files(&manifest.join(subdir), &mut files);
    }
    assert!(!files.is_empty(), "no sources found");
    let banned_idents = [
        "paint_state_marker",
        "state_marker_rect",
        "MARKER_W",
        "MARKER_STROKE_INSET",
        "STRIP_W",
    ];
    let mut offenders = Vec::new();
    for path in files.iter().filter(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let lossy = p.to_string_lossy();
        !name.contains("test") && !lossy.contains("/tests/")
    }) {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if banned_idents.iter().any(|ident| code.contains(ident)) {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                continue;
            }
            if !code.contains("rect_filled(") {
                continue;
            }
            let end = (i + 12).min(lines.len());
            let window: String = lines[i..end]
                .iter()
                .map(|l| l.split("//").next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" ");
            let call_slice = window
                .find("rect_filled(")
                .map(|idx| &window[idx..])
                .unwrap_or(&window);
            let call_block = call_slice.split(';').next().unwrap_or(call_slice);
            if is_left_edge_bar_marker(call_block) {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "left-edge bar markers are banned on cards and rows:\n{}",
        offenders.join("\n")
    );
}
