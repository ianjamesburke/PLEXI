//! Guard for stint 1291: who can mint a `HostProcess`, parsed with `syn`.
//!
//! Production must construct `HostProcess` exactly once, inside
//! `custody::install_for_host_start`, and name that installer exactly once, in
//! `main` under `if !cli_mode`. Tests may construct it only in
//! `HostProcess::for_test`. `custody.rs` may not implement `Default` or use
//! `unsafe`. Any reference to the installer counts (a call, a wrapper, a
//! function pointer), including inside `custody.rs`.
//!
//! Limits, stated: this is a syntax check. It does not see macro-generated
//! code, and `unsafe` code elsewhere in the crate could still forge a
//! zero-sized value.

use crate::testing::source_tree::{crate_sources, is_cfg_test};
use syn::visit::Visit;

const TYPE: &str = "HostProcess";
const INSTALLER: &str = "install_for_host_start";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Site {
    file: String,
    /// Enclosing function names, outermost first.
    fns: Vec<String>,
    test: bool,
    /// Inside `if !cli_mode { .. }`.
    host_branch: bool,
}

#[derive(Default)]
struct Findings {
    literals: Vec<Site>,
    installer_refs: Vec<Site>,
    custody_default: Vec<String>,
    custody_unsafe: Vec<String>,
}

fn last_ident(path: &syn::Path) -> Option<String> {
    path.segments.last().map(|s| s.ident.to_string())
}

fn type_is_host_process(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(p) if last_ident(&p.path).as_deref() == Some(TYPE))
}

struct Visitor<'a> {
    file: &'a str,
    custody_file: bool,
    test_depth: usize,
    host_branch_depth: usize,
    fns: Vec<String>,
    /// Whether each enclosing impl is `impl HostProcess` / `impl X for HostProcess`.
    impls: Vec<bool>,
    out: &'a mut Findings,
}

impl Visitor<'_> {
    fn site(&self) -> Site {
        Site {
            file: self.file.to_string(),
            fns: self.fns.clone(),
            test: self.test_depth > 0,
            host_branch: self.host_branch_depth > 0,
        }
    }

    fn with_test<T>(&mut self, test: bool, body: impl FnOnce(&mut Self) -> T) -> T {
        if test {
            self.test_depth += 1;
        }
        let value = body(self);
        if test {
            self.test_depth -= 1;
        }
        value
    }
}

impl<'ast> Visit<'ast> for Visitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.with_test(is_cfg_test(&node.attrs), |v| {
            syn::visit::visit_item_mod(v, node)
        });
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if self.custody_file && node.sig.unsafety.is_some() {
            self.out
                .custody_unsafe
                .push(format!("unsafe fn {}", node.sig.ident));
        }
        self.fns.push(node.sig.ident.to_string());
        self.with_test(is_cfg_test(&node.attrs), |v| {
            syn::visit::visit_item_fn(v, node)
        });
        self.fns.pop();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if self.custody_file && node.sig.unsafety.is_some() {
            self.out
                .custody_unsafe
                .push(format!("unsafe fn {}", node.sig.ident));
        }
        self.fns.push(node.sig.ident.to_string());
        self.with_test(is_cfg_test(&node.attrs), |v| {
            syn::visit::visit_impl_item_fn(v, node)
        });
        self.fns.pop();
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if self.custody_file {
            if node.unsafety.is_some() {
                self.out.custody_unsafe.push("unsafe impl".to_string());
            }
            if let Some((_, trait_path, _)) = &node.trait_ {
                if last_ident(trait_path).as_deref() == Some("Default") {
                    self.out.custody_default.push("impl Default".to_string());
                }
            }
        }
        self.impls.push(type_is_host_process(&node.self_ty));
        self.with_test(is_cfg_test(&node.attrs), |v| {
            syn::visit::visit_item_impl(v, node)
        });
        self.impls.pop();
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        if self.custody_file {
            for attr in node.attrs.iter().filter(|a| a.path().is_ident("derive")) {
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("Default") {
                        self.out
                            .custody_default
                            .push(format!("derive(Default) on {}", node.ident));
                    }
                    Ok(())
                });
            }
        }
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_expr_unsafe(&mut self, node: &'ast syn::ExprUnsafe) {
        if self.custody_file {
            self.out.custody_unsafe.push("unsafe block".to_string());
        }
        syn::visit::visit_expr_unsafe(self, node);
    }

    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        let name = last_ident(&node.path);
        let in_host_impl = self.impls.last().copied().unwrap_or(false);
        if name.as_deref() == Some(TYPE) || (name.as_deref() == Some("Self") && in_host_impl) {
            let site = self.site();
            self.out.literals.push(site);
        }
        syn::visit::visit_expr_struct(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if last_ident(&node.path).as_deref() == Some(INSTALLER) {
            let site = self.site();
            self.out.installer_refs.push(site);
        }
        syn::visit::visit_expr_path(self, node);
    }

    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        let host_branch = matches!(
            &*node.cond,
            syn::Expr::Unary(syn::ExprUnary { op: syn::UnOp::Not(_), expr, .. })
                if matches!(&**expr, syn::Expr::Path(p) if p.path.is_ident("cli_mode"))
        );
        self.visit_expr(&node.cond);
        if host_branch {
            self.host_branch_depth += 1;
        }
        self.visit_block(&node.then_branch);
        if host_branch {
            self.host_branch_depth -= 1;
        }
        if let Some((_, else_branch)) = &node.else_branch {
            self.visit_expr(else_branch);
        }
    }
}

/// Analyse `(relative path, source, whole-file-is-test)` triples.
fn analyse(files: &[(String, String, bool)]) -> Findings {
    let mut out = Findings::default();
    for (rel, src, test) in files {
        let ast = syn::parse_file(src).unwrap_or_else(|e| panic!("parse {rel}: {e}"));
        let mut visitor = Visitor {
            file: rel,
            custody_file: rel == "broker/custody.rs",
            test_depth: usize::from(*test),
            host_branch_depth: 0,
            fns: Vec::new(),
            impls: Vec::new(),
            out: &mut out,
        };
        visitor.visit_file(&ast);
    }
    out
}

/// Every rule violation, empty when custody is minted only at host start.
fn violations(findings: &Findings) -> Vec<String> {
    let mut bad = Vec::new();
    let prod_literals: Vec<_> = findings.literals.iter().filter(|s| !s.test).collect();
    let test_literals: Vec<_> = findings.literals.iter().filter(|s| s.test).collect();
    let mint_ok = |s: &Site| s.file == "broker/custody.rs" && s.fns == [INSTALLER.to_string()];
    if prod_literals.len() != 1 || !prod_literals.iter().all(|s| mint_ok(s)) {
        bad.push(format!(
            "production must build {TYPE} exactly once, in {INSTALLER}: {prod_literals:?}"
        ));
    }
    let test_ok = |s: &Site| s.file == "broker/custody.rs" && s.fns == ["for_test".to_string()];
    if test_literals.len() != 1 || !test_literals.iter().all(|s| test_ok(s)) {
        bad.push(format!(
            "tests must build {TYPE} only in HostProcess::for_test: {test_literals:?}"
        ));
    }
    let prod_refs: Vec<_> = findings.installer_refs.iter().filter(|s| !s.test).collect();
    let host_start =
        |s: &Site| s.file == "main.rs" && s.fns == ["main".to_string()] && s.host_branch;
    if prod_refs.len() != 1 || !prod_refs.iter().all(|s| host_start(s)) {
        bad.push(format!(
            "{INSTALLER} must be named exactly once, in main under `if !cli_mode`: {prod_refs:?}"
        ));
    }
    let stray_test_refs: Vec<_> = findings
        .installer_refs
        .iter()
        .filter(|s| s.test && s.file != "broker/custody.rs")
        .collect();
    if !stray_test_refs.is_empty() {
        bad.push(format!(
            "only custody's own tests may name {INSTALLER}: {stray_test_refs:?}"
        ));
    }
    if !findings.custody_default.is_empty() {
        bad.push(format!(
            "custody.rs implements Default: {:?}",
            findings.custody_default
        ));
    }
    if !findings.custody_unsafe.is_empty() {
        bad.push(format!(
            "custody.rs uses unsafe: {:?}",
            findings.custody_unsafe
        ));
    }
    bad
}

#[test]
fn only_host_start_installs_custody() {
    let findings = analyse(&crate_sources());
    let bad = violations(&findings);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

const CUSTODY_FIXTURE: &str = r#"
pub struct HostProcess { _private: () }
pub(crate) fn install_for_host_start() { let _ = HostProcess { _private: () }; }
#[cfg(test)]
impl HostProcess { pub(crate) const fn for_test() -> Self { HostProcess { _private: () } } }
"#;
const MAIN_FIXTURE: &str = r#"
fn main() { let cli_mode = false; if !cli_mode { crate::broker::custody::install_for_host_start(); } }
"#;

fn fixture(extra: &[(&str, &str)]) -> Vec<String> {
    let mut files = vec![
        (
            "broker/custody.rs".to_string(),
            CUSTODY_FIXTURE.to_string(),
            false,
        ),
        ("main.rs".to_string(), MAIN_FIXTURE.to_string(), false),
    ];
    for (rel, src) in extra {
        files.push((rel.to_string(), src.to_string(), false));
    }
    violations(&analyse(&files))
}

#[test]
fn guard_accepts_the_one_mint_and_the_one_host_start_call() {
    assert!(fixture(&[]).is_empty(), "{:?}", fixture(&[]));
}

#[test]
fn guard_fails_for_a_second_literal() {
    let bad = fixture(&[(
        "broker/custody_extra.rs",
        "impl HostProcess { pub fn forge() -> Self { Self { _private: () } } }",
    )]);
    assert!(
        bad.iter()
            .any(|v| v.contains("exactly once, in install_for_host_start")),
        "{bad:?}"
    );
}

#[test]
fn guard_fails_for_a_wrapper_fn() {
    let bad = fixture(&[(
        "cli/plugin.rs",
        "pub fn seal_from_cli() { crate::broker::custody::install_for_host_start(); }",
    )]);
    assert!(
        bad.iter().any(|v| v.contains("named exactly once")),
        "{bad:?}"
    );
    let pointer = fixture(&[(
        "cli/open.rs",
        "pub fn f() { let mint = install_for_host_start; mint(); }",
    )]);
    assert!(
        pointer.iter().any(|v| v.contains("named exactly once")),
        "{pointer:?}"
    );
}

#[test]
fn guard_fails_for_default_or_unsafe_in_custody() {
    let with_default = fixture(&[(
        "broker/custody.rs",
        "impl Default for HostProcess { fn default() -> Self { todo!() } }",
    )]);
    assert!(
        with_default.iter().any(|v| v.contains("Default")),
        "{with_default:?}"
    );
    let with_unsafe = fixture(&[(
        "broker/custody.rs",
        "fn f() { unsafe { std::mem::zeroed::<u8>() }; }",
    )]);
    assert!(
        with_unsafe.iter().any(|v| v.contains("unsafe")),
        "{with_unsafe:?}"
    );
}

#[test]
fn workspace_secret_commands_never_open_a_store() {
    struct StoreGuard;
    impl<'ast> Visit<'ast> for StoreGuard {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            assert!(!matches!(
                last_ident(path).as_deref(),
                Some("system_store" | "folder_store")
            ));
            syn::visit::visit_path(self, path);
        }
    }
    let (_, source, _) = crate_sources()
        .into_iter()
        .find(|(p, _, _)| p == "cli/workspace.rs")
        .unwrap();
    let file = syn::parse_file(&source).unwrap();
    for name in [
        "workspace_secret_get",
        "workspace_secret_set",
        "workspace_secret_delete",
        "folder_secret_exec",
        "folder_secret_read",
        "folder_secret_set",
        "folder_secret_rm",
    ] {
        let function = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(f) if f.sig.ident == name => Some(f),
                _ => None,
            })
            .unwrap();
        StoreGuard.visit_block(&function.block);
    }
}
