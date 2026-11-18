//! Guard for stint 1561: a dialog is `ui::dialog::ActionModal`, never a
//! `ModalShell` that draws its own buttons. Parsed with `syn` in the style of
//! `broker::custody_guard_tests`.
//!
//! Every production `ModalShell::centered(..)` must be shown in the same
//! function, with its body as an inline closure, and its id must be either a
//! sheet in `SHEETS` or the kit's own shell in `KIT_FILE`. Any shell that is
//! not a sheet must not name `KitButton` or `HintBar`, or call `.button(..)`,
//! inside its body: buttons and key hints belong to `ActionModal`.
//!
//! Limits, stated: this is a syntax check. It does not see macro-generated
//! code, a `ModalShell` imported under another name, or controls drawn by a
//! function the body calls.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::testing::source_tree::{crate_sources, is_cfg_test};
use syn::visit::Visit;

/// The dialog kit's own `ModalShell`; its id is the caller's.
const KIT_FILE: &str = "ui/dialog.rs";

/// `ModalShell` ids that are sheets or pickers, not dialogs: (id, why).
const SHEETS: &[(&str, &str)] = &[
    (
        "command_palette",
        "a search-and-run launcher list, no confirm step",
    ),
    (
        "notes_picker",
        "a note picker list with preview, no confirm step",
    ),
    (
        "quick_note_modal",
        "a free-form capture editor, saved on close",
    ),
    ("shortcuts", "a read-only reference sheet of key bindings"),
    (
        "changelog",
        "a read-only release-notes sheet; its restart control is a status line",
    ),
    ("file_browser_quick_look", "a read-only file preview sheet"),
    (
        "host_ui_gallery",
        "a developer specimen sheet that draws every kit control on purpose",
    ),
    (
        "host_ui_gallery_text_modal",
        "stale gallery demo of the pre-kit text-entry look (field plus HintBar); \
         to be replaced by an ActionModal field dialog, not a pattern to copy",
    ),
];

/// Names a body may not mention: a dialog's own buttons or key hints.
const CONTROL_TYPES: &[&str] = &["KitButton", "HintBar"];
const CONTROL_METHODS: &[&str] = &["button"];

#[derive(Debug)]
struct Shell {
    site: String,
    id: Option<String>,
    /// Controls named in the body; `None` when the body is not a closure.
    controls: Option<BTreeSet<String>>,
}

#[derive(Default)]
struct Findings {
    shells: Vec<Shell>,
    /// Shells built where the guard cannot follow them to a `show`.
    unfollowed: Vec<String>,
}

fn is_centered(call: &syn::ExprCall) -> bool {
    let syn::Expr::Path(p) = &*call.func else {
        return false;
    };
    let names: Vec<_> = p
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    names.ends_with(&["ModalShell".to_string(), "centered".to_string()])
}

fn literal_id(call: &syn::ExprCall) -> Option<String> {
    match call.args.first()? {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) => Some(s.value()),
        _ => None,
    }
}

/// The expression a builder chain starts from: `a.b().c()` -> `a`.
fn chain_root(mut expr: &syn::Expr) -> &syn::Expr {
    loop {
        expr = match expr {
            syn::Expr::MethodCall(m) => &m.receiver,
            syn::Expr::Paren(p) => &p.expr,
            _ => return expr,
        };
    }
}

#[derive(Default)]
struct ControlFinder(BTreeSet<String>);

impl<'ast> Visit<'ast> for ControlFinder {
    fn visit_path(&mut self, node: &'ast syn::Path) {
        for segment in &node.segments {
            let name = segment.ident.to_string();
            if CONTROL_TYPES.contains(&name.as_str()) {
                self.0.insert(name);
            }
        }
        syn::visit::visit_path(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let name = node.method.to_string();
        if CONTROL_METHODS.contains(&name.as_str()) {
            self.0.insert(format!(".{name}()"));
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

struct Visitor<'a, 'ast> {
    file: &'a str,
    test_depth: usize,
    fns: Vec<String>,
    /// `let` bindings to a shell chain, per enclosing function.
    bindings: Vec<HashMap<String, &'ast syn::ExprCall>>,
    shown: HashSet<*const syn::ExprCall>,
    built: Vec<(*const syn::ExprCall, String)>,
    out: &'a mut Findings,
}

impl<'ast> Visitor<'_, 'ast> {
    fn site(&self) -> String {
        format!("{}::{}", self.file, self.fns.join("::"))
    }

    fn in_fn(&mut self, name: String, test: bool, body: impl FnOnce(&mut Self)) {
        self.test_depth += usize::from(test);
        self.fns.push(name);
        self.bindings.push(HashMap::new());
        body(self);
        self.bindings.pop();
        self.fns.pop();
        self.test_depth -= usize::from(test);
    }

    /// The `ModalShell::centered` call a builder chain starts from, directly
    /// or through a `let` binding in this function.
    fn shell_of(&self, expr: &'ast syn::Expr) -> Option<&'ast syn::ExprCall> {
        match chain_root(expr) {
            syn::Expr::Call(call) if is_centered(call) => Some(call),
            syn::Expr::Path(p) => {
                let name = p.path.get_ident()?.to_string();
                self.bindings.last()?.get(&name).copied()
            }
            _ => None,
        }
    }
}

impl<'ast> Visit<'ast> for Visitor<'_, 'ast> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        let test = is_cfg_test(&node.attrs);
        self.test_depth += usize::from(test);
        syn::visit::visit_item_mod(self, node);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let test = is_cfg_test(&node.attrs);
        self.in_fn(node.sig.ident.to_string(), test, |v| {
            syn::visit::visit_item_fn(v, node);
        });
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let test = is_cfg_test(&node.attrs);
        self.in_fn(node.sig.ident.to_string(), test, |v| {
            syn::visit::visit_impl_item_fn(v, node);
        });
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let (syn::Pat::Ident(pat), Some(init)) = (&node.pat, &node.init) {
            if let Some(call) = self.shell_of(&init.expr) {
                if let Some(scope) = self.bindings.last_mut() {
                    scope.insert(pat.ident.to_string(), call);
                }
            }
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method = node.method.to_string();
        let shown_shell =
            if self.test_depth == 0 && matches!(method.as_str(), "show" | "show_scroll_body") {
                self.shell_of(&node.receiver)
            } else {
                None
            };
        if let Some(call) = shown_shell {
            self.shown.insert(std::ptr::from_ref(call));
            let controls = match node.args.last() {
                Some(syn::Expr::Closure(closure)) => {
                    let mut finder = ControlFinder::default();
                    finder.visit_expr(&closure.body);
                    Some(finder.0)
                }
                _ => None,
            };
            self.out.shells.push(Shell {
                site: self.site(),
                id: literal_id(call),
                controls,
            });
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if self.test_depth == 0 && is_centered(node) {
            self.built.push((std::ptr::from_ref(node), self.site()));
        }
        syn::visit::visit_expr_call(self, node);
    }
}

/// Analyse `(relative path, source, whole-file-is-test)` triples.
fn analyse(files: &[(String, String, bool)]) -> Findings {
    let mut out = Findings::default();
    for (rel, src, test) in files {
        let ast = syn::parse_file(src).unwrap_or_else(|e| panic!("parse {rel}: {e}"));
        let mut visitor = Visitor {
            file: rel,
            test_depth: usize::from(*test),
            fns: Vec::new(),
            bindings: Vec::new(),
            shown: HashSet::new(),
            built: Vec::new(),
            out: &mut out,
        };
        visitor.visit_file(&ast);
        let Visitor { shown, built, .. } = visitor;
        out.unfollowed.extend(
            built
                .into_iter()
                .filter(|(call, _)| !shown.contains(call))
                .map(|(_, site)| site),
        );
    }
    out
}

fn id_label(id: &Option<String>) -> String {
    id.as_ref().map_or_else(
        || "with a non-literal id".to_string(),
        |id| format!("{id:?}"),
    )
}

/// Every rule violation, empty when every dialog goes through the kit.
fn violations(findings: &Findings) -> Vec<String> {
    let mut bad = Vec::new();
    for site in &findings.unfollowed {
        bad.push(format!(
            "{site}: ModalShell built but not shown in the same function; \
             chain it into `.show(..)` so this guard can see its body"
        ));
    }
    for shell in &findings.shells {
        let sheet = shell
            .id
            .as_deref()
            .is_some_and(|id| SHEETS.iter().any(|(sheet, _)| *sheet == id));
        if sheet {
            continue;
        }
        let kit = shell.id.is_none() && shell.site.starts_with(&format!("{KIT_FILE}::"));
        if !kit {
            bad.push(format!(
                "{}: ModalShell {} is not a registered sheet; a dialog is \
                 ui::dialog::ActionModal (a new sheet goes in SHEETS with its reason)",
                shell.site,
                id_label(&shell.id)
            ));
        }
        match &shell.controls {
            None => bad.push(format!(
                "{}: ModalShell body is not an inline closure, so the guard cannot check it",
                shell.site
            )),
            Some(controls) if !controls.is_empty() => bad.push(format!(
                "{}: ModalShell {} draws its own buttons or key hints {controls:?}; \
                 use ActionModal actions",
                shell.site,
                id_label(&shell.id)
            )),
            Some(_) => {}
        }
    }
    bad
}

#[test]
fn every_dialog_uses_the_dialog_kit() {
    let findings = analyse(&crate_sources());
    let bad = violations(&findings);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    let kit = findings
        .shells
        .iter()
        .filter(|s| s.site.starts_with(&format!("{KIT_FILE}::")) && s.id.is_none())
        .count();
    assert_eq!(
        kit, 1,
        "ActionModal's own ModalShell not found: {:?}",
        findings.shells
    );
    for (sheet, _) in SHEETS {
        assert!(
            findings
                .shells
                .iter()
                .any(|s| s.id.as_deref() == Some(*sheet)),
            "SHEETS lists {sheet:?}, which no longer exists; remove it"
        );
    }
}

const KIT_FIXTURE: &str = r#"
impl<'a> ActionModal<'a> {
    fn show_field(self, ctx: &egui::Context, colors: &Colors) {
        let mut shell = ModalShell::centered(self.id).title(self.title);
        shell = shell.scrim(false);
        let response = shell.show(ctx, colors, |ui| { ButtonRow { modal: self.id }.show(ui); });
    }
}
"#;

fn fixture(extra: &[(&str, &str)]) -> Vec<String> {
    let mut files = vec![(KIT_FILE.to_string(), KIT_FIXTURE.to_string(), false)];
    for (rel, src) in extra {
        files.push((rel.to_string(), src.to_string(), false));
    }
    violations(&analyse(&files))
}

#[test]
fn guard_accepts_the_kit_and_a_sheet_with_buttons() {
    let bad = fixture(&[(
        "overlays/misc.rs",
        r#"fn draw_shortcuts(&mut self, ctx: &egui::Context) {
            crate::ui::overlay::ModalShell::centered("shortcuts").escape(true).show(ctx, &c, |ui| {
                if KitButton::new("Close", &c).ui(ui).clicked() {}
                HintBar::new(&hints).show(ui, &c);
            });
        }"#,
    )]);
    assert!(bad.is_empty(), "{bad:?}");
}

#[test]
fn guard_fails_for_a_hand_rolled_button_row() {
    let bad = fixture(&[(
        "file_browser/mod.rs",
        r#"impl FileBrowser {
            fn draw_pending_operation_modal(&mut self, ctx: &egui::Context) {
                let response = ModalShell::centered("file_browser_trash").title("Move to Trash?")
                    .show(ctx, colors, |ui| {
                        ui.horizontal(|ui| {
                            if KitButton::new("Move to Trash", colors).ui(ui).clicked() {}
                            if ui.button("Cancel").clicked() {}
                        });
                        HintBar::new(&hints).show(ui, colors);
                    });
            }
        }"#,
    )]);
    assert_eq!(bad.len(), 2, "{bad:?}");
    assert!(bad[0].contains("\"file_browser_trash\"") && bad[0].contains("not a registered sheet"));
    assert!(
        bad[1].contains("{\".button()\", \"HintBar\", \"KitButton\"}"),
        "{bad:?}"
    );
}

#[test]
fn guard_fails_for_a_button_inside_the_kit_shell() {
    let bad = violations(&analyse(&[(
        KIT_FILE.to_string(),
        KIT_FIXTURE.replace(
            "ButtonRow { modal: self.id }.show(ui);",
            "KitButton::new(\"OK\", colors).ui(ui);",
        ),
        false,
    )]));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].contains("draws its own buttons"), "{bad:?}");
}

#[test]
fn guard_fails_for_a_shell_it_cannot_follow() {
    let bad = fixture(&[(
        "overlays/helper.rs",
        r#"fn shell() -> ModalShell<'static> { ModalShell::centered("shortcuts") }
           fn draw(ctx: &egui::Context) { ModalShell::centered("rename").show(ctx, &c, draw_body); }"#,
    )]);
    assert_eq!(bad.len(), 3, "{bad:?}");
    assert!(
        bad[0].contains("helper.rs::shell: ModalShell built but not shown"),
        "{bad:?}"
    );
    assert!(
        bad[1].contains("\"rename\") is not a registered sheet"),
        "{bad:?}"
    );
    assert!(bad[2].contains("not an inline closure"), "{bad:?}");
}

#[test]
fn guard_ignores_test_code() {
    let bad = fixture(&[(
        "ui_extra.rs",
        r#"#[cfg(test)] mod tests {
            fn draw(ctx: &egui::Context) {
                ModalShell::centered("focus_ring_dialog").show(ctx, &c, |ui| { KitButton::new("A", &c).ui(ui); });
            }
        }"#,
    )]);
    assert!(bad.is_empty(), "{bad:?}");
}
