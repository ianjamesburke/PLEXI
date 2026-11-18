//! The crate's own source tree, parsed for `syn` structural guards
//! (`broker::custody_guard_tests`, `ui::modal_kit_guard_tests`).

use std::path::{Path, PathBuf};

/// True for `#[cfg(test)]`.
pub(crate) fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
}

/// `#[cfg(test)] mod name;` declarations in `file`, resolved to paths.
fn test_children(file: &Path, ast: &syn::File) -> Vec<PathBuf> {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = file.parent().unwrap_or(Path::new("."));
    let child_dir = if matches!(stem, "mod" | "main" | "lib") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    let mut out = Vec::new();
    for item in &ast.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if module.content.is_some() || !is_cfg_test(&module.attrs) {
            continue;
        }
        let explicit = module.attrs.iter().find_map(|a| {
            let syn::Meta::NameValue(nv) = &a.meta else {
                return None;
            };
            if !nv.path.is_ident("path") {
                return None;
            }
            match &nv.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) => Some(dir.join(s.value())),
                _ => None,
            }
        });
        let name = module.ident.to_string();
        out.push(explicit.unwrap_or_else(|| {
            let flat = child_dir.join(format!("{name}.rs"));
            if flat.exists() {
                flat
            } else {
                child_dir.join(&name).join("mod.rs")
            }
        }));
    }
    out
}

/// Every `.rs` file under `src/` as `(path relative to src, source,
/// whole-file-is-test)`. A file is test-only when a `#[cfg(test)] mod`
/// declares it, directly or through a test-only parent.
pub(crate) fn crate_sources() -> Vec<(String, String, bool)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                paths.push(path);
            }
        }
    }
    let mut parsed = Vec::new();
    let mut test_files = std::collections::BTreeSet::new();
    for path in &paths {
        let src = std::fs::read_to_string(path).unwrap();
        let ast = syn::parse_file(&src).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
        test_files.extend(test_children(path, &ast));
        parsed.push((path.clone(), src));
    }
    // A child of a test-only module is test-only too.
    loop {
        let mut grew = false;
        for (path, src) in &parsed {
            if test_files.contains(path) {
                let ast = syn::parse_file(src).unwrap();
                for child in test_children(path, &ast) {
                    grew |= test_files.insert(child);
                }
            }
        }
        if !grew {
            break;
        }
    }
    parsed
        .into_iter()
        .map(|(path, src)| {
            let rel = path.strip_prefix(&root).unwrap().display().to_string();
            let test = test_files.contains(&path) || rel == "lib.rs";
            (rel, src, test)
        })
        .collect()
}
