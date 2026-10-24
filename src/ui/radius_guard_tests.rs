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

/// True when `block` contains `Vec2::new(<width>, ... .height())` with width <= 4.0
/// (or `STRIP_W`) paired with `CornerRadius::ZERO` or asymmetric zero right corners.
fn is_adhoc_left_edge_marker(block: &str) -> bool {
    if !block.contains(".height()") {
        return false;
    }
    if !(block.contains("CornerRadius::ZERO") || block.contains("ne: 0")) {
        return false;
    }
    let Some(vec_idx) = block.find("Vec2::new(") else {
        return false;
    };
    let after = &block[vec_idx + "Vec2::new(".len()..];
    let first_arg = after.split(',').next().unwrap_or("").trim();
    if first_arg == "STRIP_W" {
        return true;
    }
    first_arg
        .trim_end_matches("_f32")
        .parse::<f32>()
        .is_ok_and(|w| w > 0.0 && w <= 4.0)
}

#[test]
fn no_adhoc_left_edge_state_markers_outside_helper() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for subdir in ["src/ui", "src/app", "src/spatial"] {
        rs_files(&manifest.join(subdir), &mut files);
    }
    assert!(!files.is_empty(), "no sources found");
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
            if is_adhoc_left_edge_marker(call_block) {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use ui::paint_state_marker instead of ad-hoc left-edge rect_filled markers:\n{}",
        offenders.join("\n")
    );
}
