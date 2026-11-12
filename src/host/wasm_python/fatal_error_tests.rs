//! Stint 1328: a plugin exception reaches `plugin check`, `view render` and the
//! live pane with the plugin's own file, line, exception type and message
//! first. Each headless test is one of the misuse probes the plugin gauntlet
//! ran; before the fix every one of them read
//! `plugin exited before responding`, then a CPython startup line, then the
//! real exception last, with `/app/` guest paths.

use std::path::Path;

use super::*;

const PRELUDE: &str = "from plexi_sdk import state\n\
from plexi_sdk.effects import SetTitle\n\
from plexi_sdk.ui import Badge, Column, Text\n\
def update(event):\n    return []\n";
const GOOD_INIT: &str = "def init(size, args):\n    return []\n";

fn launch_config(plugin_dir: &Path) -> PythonLaunchConfig {
    PythonLaunchConfig {
        plugin_id: "test.fatal-error".to_string(),
        plugin_dir: plugin_dir.to_path_buf(),
        entry: plugin_dir.join("main.py"),
        module_name: "main".to_string(),
        launch_args: Vec::new(),
        workspace_root: plugin_dir.to_path_buf(),
        capabilities: Vec::new(),
        allowed_hosts: Vec::new(),
        theme: crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default())
            .to_theme_map(),
        state_scopes: crate::host::state_scope::default_scopes(),
        state_format: crate::host::state_scope::StateFormat::Json,
        context_root: plugin_dir.to_path_buf(),
    }
}

/// The text `plugin check` and `view render` print after their own label: the
/// `Display` of the error one headless frame returns.
fn headless_failure(main_py: &str, extra: &[(&str, &str)]) -> String {
    let app = tempfile::tempdir().expect("plugin dir");
    std::fs::write(app.path().join("main.py"), main_py).expect("write entry");
    for (relative, body) in extra {
        let path = app.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        std::fs::write(path, body).expect("write helper");
    }
    run_headless_frame(&launch_config(app.path()), (320.0, 240.0), None)
        .expect_err("the probe plugin must fail to render")
        .to_string()
}

fn line_of(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in fixture source"))
        + 1
}

fn assert_leads_with(report: &str, headline: &str) {
    assert_eq!(
        report.lines().next(),
        Some(headline),
        "the first line must be the plugin's own exception:\n{report}"
    );
    assert!(
        !report.contains("/app/"),
        "guest mount paths leaked:\n{report}"
    );
    assert!(
        !report.contains("platform dependent libraries"),
        "CPython startup noise leaked:\n{report}"
    );
    assert!(
        !report.contains("plugin exited before responding"),
        "a reported exception is not an unexplained death:\n{report}"
    );
}

#[test]
fn state_set_inside_view_leads_with_the_plugin_line() {
    let source =
        format!("{PRELUDE}{GOOD_INIT}def view():\n    state.set(\"last\", \"x\")\n    return Text(\"ok\")\n");
    let report = headless_failure(&source, &[]);
    assert_leads_with(
        &report,
        &format!(
            "plugin error in main.py:{} (view): RuntimeError: state.set() called inside view() \
             - return SetState from update() instead",
            line_of(&source, "state.set(")
        ),
    );
    assert!(
        report.contains("File \"main.py\", line"),
        "the traceback follows the headline with plugin-relative paths:\n{report}"
    );
}

#[test]
fn bad_badge_color_leads_with_the_helper_module_line() {
    let source = format!(
        "{PRELUDE}{GOOD_INIT}from plugin import ui as plugin_ui\n\
         def view():\n    return plugin_ui.build_view()\n"
    );
    let helper = "from plexi_sdk.ui import Badge, Column\n\
                  def build_view():\n    return Column(children=[\n        Badge(\"hi\", color=\"blue\"),\n    ])\n";
    let report = headless_failure(
        &source,
        &[("plugin/__init__.py", ""), ("plugin/ui.py", helper)],
    );
    let first = report.lines().next().unwrap_or_default();
    let expected_prefix = format!(
        "plugin error in plugin/ui.py:{} (view): ValueError: Badge color must be one of",
        line_of(helper, "Badge(\"hi\"")
    );
    assert!(first.starts_with(&expected_prefix), "{report}");
    assert!(first.contains("got 'blue'"), "{report}");
    assert_leads_with(&report, first);
}

#[test]
fn str_returned_from_view_names_the_type_and_the_fix() {
    let source = format!("{PRELUDE}{GOOD_INIT}def view():\n    return \"hello\"\n");
    let report = headless_failure(&source, &[]);
    assert_leads_with(
        &report,
        &format!(
            "plugin error in main.py:{} (view): TypeError: view() returned str, which is not a \
             component. Return exactly one plexi_sdk.ui component, for example \
             Column(children=[Text(...)]).",
            line_of(&source, "def view():")
        ),
    );
}

#[test]
fn init_without_parameters_names_the_expected_signature() {
    let source =
        format!("{PRELUDE}def init():\n    return []\ndef view():\n    return Text(\"ok\")\n");
    let report = headless_failure(&source, &[]);
    assert_leads_with(
        &report,
        &format!(
            "plugin error in main.py:{} (init): TypeError: init() is called as \
             `def init(size, args)`, but the plugin defines `def init()`. Change the definition \
             to `def init(size, args):`.",
            line_of(&source, "def init():")
        ),
    );
}

#[test]
fn effect_returned_from_view_leads_with_the_view() {
    let source = format!("{PRELUDE}{GOOD_INIT}def view():\n    return SetTitle(\"nope\")\n");
    let report = headless_failure(&source, &[]);
    assert_leads_with(
        &report,
        &format!(
            "plugin error in main.py:{} (view): TypeError: view() returned the effect SetTitle. \
             Effects are returned from update() or init(); view() returns exactly one component.",
            line_of(&source, "def view():")
        ),
    );
}

/// A guest that dies without reporting (here: the process is ended from under
/// the runtime) still takes the death path, with whatever it wrote to stderr
/// and without CPython's prefix warning.
#[test]
fn a_hard_crash_still_reports_an_unexplained_exit_with_stderr() {
    let report = headless_failure(
        "import os, sys\nsys.stderr.write('last words\\n')\nsys.stderr.flush()\nos._exit(3)\n",
        &[],
    );
    assert!(
        report.contains("plugin exited before responding"),
        "{report}"
    );
    assert!(report.contains("last words"), "{report}");
    assert!(
        !report.contains("platform dependent libraries"),
        "the guest's prefix is configured, so CPython has nothing to warn about:\n{report}"
    );
}

#[test]
fn live_pane_failure_leads_with_the_plugin_exception() {
    let app = tempfile::tempdir().expect("plugin dir");
    let source = format!(
        "{PRELUDE}def init(size, args):\n    raise ValueError(\"broken on purpose\")\n\
         def view():\n    return Text(\"ok\")\n"
    );
    std::fs::write(app.path().join("main.py"), &source).expect("write entry");
    let mut pane = LivePythonPane::launch(launch_config(app.path())).expect("launch CPython WASM");
    let deadline = std::time::Instant::now()
        + crate::testing::load_aware_timeout(std::time::Duration::from_secs(60));
    while pane.error().is_none() && std::time::Instant::now() < deadline {
        pane.background_tick();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let expected = format!(
        "plugin error in main.py:{} (init): ValueError: broken on purpose",
        line_of(&source, "raise ValueError")
    );
    assert_eq!(pane.error(), Some(expected.as_str()));
    assert_eq!(pane.lifecycle(), ("failed", Some(expected.as_str())));
}

/// The report is final: a guest that keeps talking after `fatal_error` gets
/// nothing serviced, and cannot rewrite the failure the pane shows.
#[test]
fn live_pane_ignores_everything_after_the_first_fatal_error() {
    let app = tempfile::tempdir().expect("plugin dir");
    std::fs::write(
        app.path().join("main.py"),
        format!("{PRELUDE}{GOOD_INIT}def view():\n    return Text(\"ok\")\n"),
    )
    .expect("write entry");
    let mut pane = LivePythonPane::launch(launch_config(app.path())).expect("launch CPython WASM");
    let fatal = |message: &str| {
        json!({
            "type": "fatal_error",
            "phase": "view",
            "file": "main.py",
            "line": 3,
            "exception": "RuntimeError",
            "message": message,
            "traceback": "Traceback (most recent call last):\nRuntimeError\n",
        })
    };
    pane.handle_message(fatal("first"));
    let expected = "plugin error in main.py:3 (view): RuntimeError: first";
    assert_eq!(pane.error(), Some(expected));
    assert_eq!(pane.dropped_after_fatal, Some(0));

    pane.handle_message(fatal("second"));
    pane.handle_message(json!({"type": "log", "level": "info", "message": "still here"}));
    pane.handle_message(json!({"type": "ready", "sdk": "x", "features_used": []}));
    pane.handle_message(json!({"type": "set_title", "title": "forged"}));

    assert_eq!(
        pane.error(),
        Some(expected),
        "a later report must not replace the first"
    );
    assert!(
        !pane.ready,
        "a message after fatal_error must not be handled"
    );
    assert_eq!(pane.display_name(), "test.fatal-error");
    assert_eq!(pane.dropped_after_fatal, Some(4));
}
