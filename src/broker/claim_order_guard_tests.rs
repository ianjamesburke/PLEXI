//! Guard for stints 1459 and 1487: in `main`, a host launch claims its
//! profile (`app::host_instance::claim`, reached through `host_launch`) before
//! custody, retention, the rpc sweep and the window, parsed with `syn` in the
//! style of `custody_guard_tests`.
//!
//! The claim must appear exactly once in `fn main`, inside the
//! `HostLaunch::Host` arm, and every later startup step must come after it in
//! source order. `eframe::run_native` builds the window and the app, whose
//! construction binds the notify socket, so "before `run_native`" covers both.
//!
//! Limits, stated: this is a syntax check of source order in `fn main`. It
//! does not see macro-generated code or follow calls into other functions.

use syn::visit::Visit;

/// A startup step the claim must precede: (path's last segment, label).
const LATER_STEPS: &[(&str, &str)] = &[
    ("install_for_host_start", "custody install"),
    ("run_at_startup", "retention"),
    ("sweep_stale", "rpc sweep"),
    ("run_native", "window and socket"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Event {
    /// `claim`, `host_launch`, or a later step's label.
    what: String,
    /// Match-arm patterns enclosing the event, outermost first (last ident).
    arms: Vec<String>,
}

#[derive(Default)]
struct MainVisitor {
    in_main: bool,
    arms: Vec<String>,
    events: Vec<Event>,
}

fn path_names(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

fn pattern_name(pat: &syn::Pat) -> String {
    let path = match pat {
        syn::Pat::Path(p) => Some(&p.path),
        syn::Pat::TupleStruct(p) => Some(&p.path),
        syn::Pat::Struct(p) => Some(&p.path),
        _ => None,
    };
    path.and_then(|p| p.segments.last())
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

impl<'ast> Visit<'ast> for MainVisitor {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let was = self.in_main;
        self.in_main = node.sig.ident == "main";
        syn::visit::visit_item_fn(self, node);
        self.in_main = was;
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        self.arms.push(pattern_name(&node.pat));
        syn::visit::visit_arm(self, node);
        self.arms.pop();
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if self.in_main {
            let names = path_names(&node.path);
            let last = names.last().map(String::as_str).unwrap_or("");
            let host_instance = names.iter().any(|n| n == "host_instance");
            let what = match last {
                "claim" | "host_launch" if host_instance => Some(last.to_string()),
                _ => LATER_STEPS
                    .iter()
                    .find(|(name, _)| *name == last)
                    .map(|(_, label)| label.to_string()),
            };
            if let Some(what) = what {
                self.events.push(Event {
                    what,
                    arms: self.arms.clone(),
                });
            }
        }
        syn::visit::visit_expr_path(self, node);
    }
}

fn main_events(src: &str) -> Vec<Event> {
    let ast = syn::parse_file(src).unwrap_or_else(|e| panic!("parse main.rs: {e}"));
    let mut visitor = MainVisitor::default();
    visitor.visit_file(&ast);
    visitor.events
}

/// Every rule violation, empty when the claim comes first on the host path.
fn violations(events: &[Event]) -> Vec<String> {
    let mut bad = Vec::new();
    let claims: Vec<_> = events.iter().filter(|e| e.what == "claim").collect();
    let launches: Vec<_> = events.iter().filter(|e| e.what == "host_launch").collect();
    if claims.len() != 1 || launches.len() != 1 {
        bad.push(format!(
            "fn main must name host_instance::host_launch and host_instance::claim exactly once: {events:?}"
        ));
        return bad;
    }
    if claims[0].arms.last().map(String::as_str) != Some("Host") {
        bad.push(format!(
            "the claim must sit in the HostLaunch::Host arm: {:?}",
            claims[0]
        ));
    }
    let claim_at = events.iter().position(|e| e.what == "claim").unwrap_or(0);
    let launch_at = events
        .iter()
        .position(|e| e.what == "host_launch")
        .unwrap_or(0);
    if launch_at > claim_at {
        bad.push("host_launch must classify the launch before the claim".to_string());
    }
    for (_, label) in LATER_STEPS {
        match events.iter().position(|e| e.what == *label) {
            None => bad.push(format!("fn main no longer names the {label} step")),
            Some(at) if at < claim_at => {
                bad.push(format!("the {label} step runs before the claim"))
            }
            Some(_) => {}
        }
    }
    bad
}

#[test]
fn main_claims_the_profile_before_every_later_startup_step() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
    )
    .unwrap();
    let bad = violations(&main_events(&src));
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

const MAIN_FIXTURE: &str = r#"
fn main() {
    let _instance = match app::host_instance::host_launch(cli_mode, None) {
        app::host_instance::HostLaunch::Cli => None,
        app::host_instance::HostLaunch::Host => match app::host_instance::claim(&a, &b) {
            Ok(instance) => Some(instance),
            Err(_) => std::process::exit(3),
        },
    };
    if !cli_mode { crate::broker::custody::install_for_host_start(); }
    crate::cloud::retention::run_at_startup();
    rpc::sweep_stale(&dir);
    eframe::run_native();
}
"#;

#[test]
fn guard_accepts_the_claim_first() {
    assert_eq!(violations(&main_events(MAIN_FIXTURE)), Vec::<String>::new());
}

#[test]
fn guard_fails_when_the_claim_is_deleted() {
    let src = MAIN_FIXTURE.replace("match app::host_instance::claim(&a, &b)", "match ok()");
    assert!(!violations(&main_events(&src)).is_empty());
}

#[test]
fn guard_fails_when_custody_moves_above_the_claim() {
    let src = MAIN_FIXTURE
        .replace(
            "    if !cli_mode { crate::broker::custody::install_for_host_start(); }\n",
            "",
        )
        .replace(
            "fn main() {\n",
            "fn main() {\n    if !cli_mode { crate::broker::custody::install_for_host_start(); }\n",
        );
    let bad = violations(&main_events(&src));
    assert!(bad.iter().any(|b| b.contains("custody install")), "{bad:?}");
}

#[test]
fn guard_fails_when_the_claim_leaves_the_host_arm() {
    let src = MAIN_FIXTURE.replace(
        "app::host_instance::HostLaunch::Host =>",
        "app::host_instance::HostLaunch::InsidePlexi =>",
    );
    assert!(!violations(&main_events(&src)).is_empty());
}
