//! Contract between the Python SDK runtime and `LivePythonPane`: the two sides
//! must agree on every message type string. Each set is derived from its own
//! source (Python via `ast`, Rust via `syn`), never from a hand-copied list.
//! A hand-kept list exists only for host kinds the SDK cannot emit
//! (`HOST_ONLY_GUEST_KINDS`); the test fails when it goes stale in either
//! direction.
//!
//! Born from the `save_app_state` / `save_plugin_state` rename regression: the
//! SDK kept the old name, the host dropped the message with a warning, and
//! plugin state silently stopped persisting.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

/// Guest-to-host kinds the host accepts but the SDK runtime has no effect to
/// emit (raw-bridge and WIT-parity commands). Adding a host kind not listed
/// here and not emitted by the SDK fails the test; so does listing a kind the
/// SDK now emits.
const HOST_ONLY_GUEST_KINDS: &[&str] = &[
    "command_preview",
    "focus_pane",
    "insert_path_token",
    "query_context_state",
    "run_update",
    "show_notification",
    "spawn_pane",
    "spawn_view",
];

/// Host-to-guest kinds the SDK runtime dispatches on that the host has no
/// sender for today. `capability_granted` / `capability_denied` are superseded
/// by `capability_decision`; `focus_changed` has no host sender at all (the
/// stats plugin listens for it). Reported as findings of the wire audit, not
/// fixed here. Stale entries fail the test.
const SDK_ONLY_HANDLED_KINDS: &[&str] =
    &["capability_denied", "capability_granted", "focus_changed"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn parse(path: &Path) -> syn::File {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    syn::parse_file(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
}

fn pat_strings(pat: &syn::Pat, out: &mut BTreeSet<String>) {
    match pat {
        syn::Pat::Lit(lit) => {
            if let syn::Lit::Str(s) = &lit.lit {
                out.insert(s.value());
            }
        }
        syn::Pat::Or(or) => or.cases.iter().for_each(|p| pat_strings(p, out)),
        syn::Pat::TupleStruct(ts) => ts.elems.iter().for_each(|p| pat_strings(p, out)),
        syn::Pat::Paren(p) => pat_strings(&p.pat, out),
        _ => {}
    }
}

/// String patterns of every `match message_type { .. }` in non-test code.
#[derive(Default)]
struct MessageTypeMatches {
    kinds: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for MessageTypeMatches {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !is_cfg_test(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }
    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        if matches!(&*node.expr, syn::Expr::Path(p) if p.path.is_ident("message_type")) {
            for arm in &node.arms {
                pat_strings(&arm.pat, &mut self.kinds);
            }
        }
        syn::visit::visit_expr_match(self, node);
    }
}

/// `"type": "literal"` pairs inside macro bodies (`json!`) and string
/// arguments of `wire.push(..)` (the pane-presence kinds), in non-test code.
#[derive(Default)]
struct TypeLiterals {
    kinds: BTreeSet<String>,
}

fn string_literal(tt: &TokenTree) -> Option<String> {
    let TokenTree::Literal(lit) = tt else {
        return None;
    };
    match syn::parse_str::<syn::Lit>(&lit.to_string()).ok()? {
        syn::Lit::Str(s) => Some(s.value()),
        _ => None,
    }
}

fn scan_tokens(stream: TokenStream, out: &mut BTreeSet<String>) {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    for window in trees.windows(3) {
        if string_literal(&window[0]).as_deref() == Some("type")
            && matches!(&window[1], TokenTree::Punct(p) if p.as_char() == ':')
        {
            if let Some(kind) = string_literal(&window[2]) {
                out.insert(kind);
            }
        }
    }
    for tt in &trees {
        if let TokenTree::Group(group) = tt {
            scan_tokens(group.stream(), out);
        }
    }
}

impl<'ast> Visit<'ast> for TypeLiterals {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !is_cfg_test(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }
    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        scan_tokens(node.mac.tokens.clone(), &mut self.kinds);
    }
    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        scan_tokens(node.mac.tokens.clone(), &mut self.kinds);
    }
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "push"
            && matches!(&*node.receiver, syn::Expr::Path(p) if p.path.is_ident("wire"))
        {
            if let Some(syn::Expr::Lit(lit)) = node.args.first() {
                if let syn::Lit::Str(s) = &lit.lit {
                    self.kinds.insert(s.value());
                }
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

fn snake_case(ident: &str) -> String {
    let mut out = String::new();
    for (i, ch) in ident.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Variant names (snake_case) of every `#[serde(tag = "type", rename_all =
/// "snake_case")]` enum in the file.
fn tagged_enum_kinds(path: &Path) -> BTreeSet<String> {
    let mut kinds = BTreeSet::new();
    for item in parse(path).items {
        let syn::Item::Enum(item) = item else {
            continue;
        };
        let (mut tag, mut snake) = (false, false);
        for attr in item.attrs.iter().filter(|a| a.path().is_ident("serde")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("tag") {
                    tag = meta.value()?.parse::<syn::LitStr>()?.value() == "type";
                } else if meta.path.is_ident("rename_all") {
                    snake = meta.value()?.parse::<syn::LitStr>()?.value() == "snake_case";
                } else if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<syn::Expr>()?;
                }
                Ok(())
            })
            .unwrap_or_else(|e| panic!("serde attribute on {} in {path:?}: {e}", item.ident));
        }
        if tag && snake {
            kinds.extend(
                item.variants
                    .iter()
                    .map(|v| snake_case(&v.ident.to_string())),
            );
        }
    }
    kinds
}

fn sdk_wire_kinds() -> (BTreeSet<String>, BTreeSet<String>) {
    let output = std::process::Command::new("python3")
        .arg("-I")
        .arg(repo_root().join("scripts/sdk-wire-kinds.py"))
        .output()
        .expect("run scripts/sdk-wire-kinds.py");
    assert!(
        output.status.success(),
        "sdk-wire-kinds.py failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("sdk-wire-kinds.py prints JSON");
    let set = |key: &str| -> BTreeSet<String> {
        parsed[key]
            .as_array()
            .unwrap_or_else(|| panic!("sdk-wire-kinds.py output has no '{key}' array"))
            .iter()
            .map(|v| v.as_str().expect("string kind").to_string())
            .collect()
    };
    (set("emits"), set("handles"))
}

#[test]
fn sdk_and_host_agree_on_wire_message_kinds() {
    let (sdk_emits, sdk_handles) = sdk_wire_kinds();
    let wasm_python = parse(&repo_root().join("src/host/wasm_python.rs"));

    // Guest to host: everything `LivePythonPane` / the decoder thread /
    // `plugin_command_from_python_message` match on.
    let mut matches = MessageTypeMatches::default();
    matches.visit_file(&wasm_python);
    let host_accepts = matches.kinds;

    let host_only: BTreeSet<String> = HOST_ONLY_GUEST_KINDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let dropped_by_host: Vec<_> = sdk_emits.difference(&host_accepts).collect();
    assert!(
        dropped_by_host.is_empty(),
        "SDK emits message kinds the host does not accept (silently dropped with an \
         'unhandled' warning): {dropped_by_host:?}"
    );
    let unemitted: BTreeSet<_> = host_accepts.difference(&sdk_emits).cloned().collect();
    let unexpected: Vec<_> = unemitted.difference(&host_only).collect();
    assert!(
        unexpected.is_empty(),
        "host accepts kinds the SDK never emits and that are not in HOST_ONLY_GUEST_KINDS \
         (renamed on one side only?): {unexpected:?}"
    );
    let stale: Vec<_> = host_only.difference(&unemitted).collect();
    assert!(
        stale.is_empty(),
        "HOST_ONLY_GUEST_KINDS lists kinds that are emitted by the SDK or no longer accepted \
         by the host; drop them: {stale:?}"
    );

    // Host to guest: every kind the SDK dispatches on must be something the
    // host can send. The host's sendable set is the `"type"` literals of
    // its wire builders plus the tagged protocol enums; it is a superset of
    // what python panes receive, so the reverse direction is not asserted.
    let mut sent = TypeLiterals::default();
    sent.visit_file(&wasm_python);
    sent.visit_file(&parse(&repo_root().join("src/host/cli_ops/service.rs")));
    let mut host_sends = sent.kinds;
    for file in ["events.rs", "commands.rs"] {
        host_sends.extend(tagged_enum_kinds(
            &repo_root().join("src/protocol").join(file),
        ));
    }
    let sdk_only: BTreeSet<String> = SDK_ONLY_HANDLED_KINDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let unsendable: BTreeSet<_> = sdk_handles.difference(&host_sends).cloned().collect();
    let unexpected: Vec<_> = unsendable.difference(&sdk_only).collect();
    assert!(
        unexpected.is_empty(),
        "SDK runtime dispatches on kinds the host never sends and that are not in \
         SDK_ONLY_HANDLED_KINDS (renamed on one side only?): {unexpected:?}"
    );
    let stale: Vec<_> = sdk_only.difference(&unsendable).collect();
    assert!(
        stale.is_empty(),
        "SDK_ONLY_HANDLED_KINDS lists kinds the host now sends or the SDK no longer handles; \
         drop them: {stale:?}"
    );
}
