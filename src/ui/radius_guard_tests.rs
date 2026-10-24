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
