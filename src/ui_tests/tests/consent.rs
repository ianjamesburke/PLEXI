use super::*;

/// Stint 1498: dialog action rows take no Tab focus, so egui's keyboard
/// click on a focused row cannot press a consent action. Even with the
/// review armed, Tab then Space allows nothing and writes no grant.
#[test]
fn raw_wasm_review_tab_then_space_allows_nothing() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/wasm-fixtures/sysmon.wasm");
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    h.with_app_mut(|app| {
        app.launch_view_by_path_with_layout(
            &fixture.to_string_lossy(),
            Some("split_v".to_string()),
            None,
            &[],
        )
    })
    .expect("unreviewed wasm launch queues a review");
    h.run_steps(3);
    let (plugin, root, caps) = h.with_app(|app| {
        let review = app
            .pending_raw_wasm_launches
            .front()
            .expect("review queued");
        (
            review.plugin_id.clone(),
            review.workspace_root.clone(),
            review.missing_capabilities.clone(),
        )
    });
    assert!(!caps.is_empty());
    h.with_app_mut(|app| {
        app.raw_wasm_review_guard
            .backdate(crate::broker::consent::ARM_DELAY)
    });

    h.harness().key_press(egui::Key::Tab);
    h.run_steps(2);
    h.harness().key_press(egui::Key::Space);
    h.run_steps(3);

    assert_eq!(
        h.with_app(|app| app.pending_raw_wasm_launches.len()),
        1,
        "Tab then Space allowed the raw WASM review"
    );
    let store =
        crate::app::permissions::PermissionStore::load_or_default(&crate::config::config_dir())
            .expect("profile permission store");
    for cap in &caps {
        assert_eq!(
            store.get_wasm(&plugin, &root, cap),
            None,
            "a grant was written for {cap}"
        );
    }
}

/// Stint 1489: the bundled Kraken plugin on a fresh profile asks for
/// `net.http` in its pane (naming the hosts), and after "Deny once" (Esc)
/// shows a refusal that says how to be asked again, not an empty ticker.
#[test]
fn screenshot_python_consent_prompt_then_refusal() {
    let kraken = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/kraken");
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    apply_fixture_theme(&mut h, "plexi-dusk");
    let pane = h.open_view_at(&kraken, &[]).expect("open kraken");
    fn python_pane(
        app: &mut crate::app::PlexiApp,
        pane: PaneId,
    ) -> Option<&mut crate::host::wasm_python::LivePythonPane> {
        let window = app.active_window;
        match &mut app.windows[window]
            .panes
            .get_mut(&pane)?
            .as_view_mut()?
            .runtime
        {
            ViewRuntime::Python(p) => Some(p),
            _ => None,
        }
    }
    let start = Instant::now();
    while !h.with_app_mut(|app| {
        python_pane(app, pane).is_some_and(|p| p.has_pending_capability_prompt())
    }) {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "no net.http prompt"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(25));
    }
    h.with_app_mut(|app| {
        if let Some(p) = python_pane(app, pane) {
            p.backdate_capability_guard(crate::broker::consent::ARM_DELAY);
        }
    });
    // Arming fades the approving buttons in; let the fade finish before
    // measuring contrast. Under load the fade can outlast a fixed step
    // count, so step until the report is clean or the deadline passes.
    let start = Instant::now();
    loop {
        h.run_steps(5);
        let errors: Vec<String> = h.with_app(|app| {
            crate::ui::layout_report::snapshot(&app.ctx)
                .iter()
                .flat_map(|r| r.violations())
                .collect()
        });
        if errors.is_empty() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{errors:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
    h.harness().get_by_label("Grant once");
    h.harness()
        .get_by_label("Hosts: api.kraken.com and their subdomains");
    h.harness()
        .get_by_label("net.http: Make outbound HTTP requests");
    h.save_screenshot(&evidence_png!("_prompt")).unwrap();

    h.harness().key_press(egui::Key::Escape);
    // Python plugin text is painted, not in the accessibility tree, so
    // read the plugin's semantic state.
    let refusal = "net.http was denied for this request. kraken asks again next time.";
    let start = Instant::now();
    loop {
        let labels: Vec<String> = h.with_app_mut(|app| {
            python_pane(app, pane)
                .map(|p| {
                    p.semantic_state()
                        .nodes
                        .into_iter()
                        .filter_map(|n| n.label)
                        .collect()
                })
                .unwrap_or_default()
        });
        if labels.iter().any(|l| l == "Network access refused")
            && labels.iter().any(|l| l == refusal)
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "no refusal shown: {labels:?}"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(25));
    }
    h.run_steps(2);
    assert!(h.with_app_mut(|app| {
        python_pane(app, pane).is_some_and(|p| !p.has_pending_capability_prompt())
    }));
    h.save_screenshot(&evidence_png!("_refused")).unwrap();
}

/// Stint 1498: a consent prompt that has just appeared draws its approving
/// action disabled and its refusal enabled; once armed, both are enabled.
#[test]
fn screenshot_raw_wasm_review_arming_then_armed() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/wasm-fixtures/sysmon.wasm");
    use egui_kittest::kittest::NodeT;
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    apply_fixture_theme(&mut h, "plexi-dusk");
    h.with_app_mut(|app| {
        app.launch_view_by_path_with_layout(
            &fixture.to_string_lossy(),
            Some("split_v".to_string()),
            None,
            &[],
        )
    })
    .expect("unreviewed wasm launch queues a review");
    h.run_steps(3);
    assert!(h.harness().get_by_label("Allow and remember").accesskit_node().is_disabled());
    assert!(!h.harness().get_by_label("Deny").accesskit_node().is_disabled());
    h.save_screenshot(&evidence_png!("_arming")).unwrap();

    h.with_app_mut(|app| {
        app.raw_wasm_review_guard
            .backdate(crate::broker::consent::ARM_DELAY)
    });
    h.run_steps(2);
    assert!(!h.harness().get_by_label("Allow and remember").accesskit_node().is_disabled());
    h.with_app(|app| {
        let reports = crate::ui::layout_report::snapshot(&app.ctx);
        let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
        assert!(errors.is_empty(), "{errors:?}");
    });
    h.save_screenshot(&evidence_png!("_armed")).unwrap();
}

/// Stint 1558: the raw WASM review renders as ActionModal's one button
/// row (Allow and remember rightmost, Deny on its left, one top edge) and
/// keeps its consent guard: Allow is disabled until armed, a real click
/// on it inside the arm delay allows nothing, Tab never focuses its
/// buttons (so egui's Space/Enter click cannot bypass the guard), and once
/// armed the same click allows.
#[test]
fn raw_wasm_review_button_row_keeps_its_consent_guard() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    // A copy outside the checkout, so the published evidence shows no
    // machine path.
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("sysmon.wasm");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon.wasm"),
        &fixture,
    )
    .unwrap();
    use egui_kittest::kittest::NodeT;
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    h.with_app_mut(|app| {
        app.launch_view_by_path_with_layout(
            &fixture.to_string_lossy(),
            Some("split_v".to_string()),
            None,
            &[],
        )
    })
    .expect("unreviewed wasm launch queues a review");
    h.run_steps(3);
    let pending =
        |h: &mut PlexiUiHarness| h.with_app(|app| app.pending_raw_wasm_launches.len());
    let allow = h.harness().get_by_label("Allow and remember").rect();
    let deny = h.harness().get_by_label("Deny").rect();
    assert_eq!(allow.top(), deny.top(), "one row");
    assert!(
        allow.left() > deny.right(),
        "Allow is rightmost, Deny on its left"
    );
    let disabled = |h: &mut PlexiUiHarness, label| {
        h.harness()
            .get_by_label(label)
            .accesskit_node()
            .is_disabled()
    };
    assert!(disabled(&mut h, "Allow and remember"));
    assert!(!disabled(&mut h, "Deny"));

    click_at(&mut h, allow.center());
    assert_eq!(pending(&mut h), 1, "a click inside the arm delay allowed");

    h.with_app_mut(|app| {
        app.raw_wasm_review_guard
            .backdate(crate::broker::consent::ARM_DELAY)
    });
    h.run_steps(2);
    h.harness().key_press(egui::Key::Tab);
    h.run_steps(2);
    let focused = focused_label(&mut h, &["Allow and remember", "Deny"]);
    assert!(focused.is_none(), "Tab focused the review's {focused:?}");
    assert!(!disabled(&mut h, "Allow and remember"));
    h.save_screenshot(&evidence_png!()).unwrap();
    click_at(&mut h, allow.center());
    assert_eq!(pending(&mut h), 0, "a real click once armed must allow");
}

/// A real pointer press and release at `pos`, each in its own frame.
fn click_at(h: &mut PlexiUiHarness, pos: egui::Pos2) {
    for pressed in [true, false] {
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        h.step();
    }
    h.run_steps(2);
}

/// Stint 0788: a pane's `plexi secret get` shows on the desktop banner
/// with the secret and its workspace, so the person knows what Allow once
/// releases.
#[test]
fn screenshot_secret_get_banner_names_the_secret() {
    let profile = tempfile::tempdir().expect("profile");
    let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
    let root = tempfile::tempdir().expect("workspace");
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .unwrap();
    let scope = crate::workspace::secrets::SecretScope::Workspace {
        id: "ws-banner".to_string(),
        root: root.path().to_path_buf(),
    };
    let actor = crate::workspace::secrets::SecretActor {
        kind: crate::broker::ActorType::Agent,
        id: "pane:4".to_string(),
    };
    let store = crate::workspace::secrets::InMemoryKeychain::new();
    let read = crate::workspace::secrets::read_pane_secret(
        &monitor,
        &store,
        &actor,
        "OPENAI_API_KEY",
        &scope,
        profile.path(),
    )
    .expect("read admits");
    assert!(matches!(
        read,
        crate::workspace::secrets::ReadResult::PermissionRequired { .. }
    ));

    let mut h = PlexiUiHarness::new_sized(1100.0, 300.0);
    h.run_steps(2);
    // The old approval banner is gone: the ask is an attention-queue item
    // that names the secret and its workspace.
    h.with_app_mut(|app| app.sync_attention_queue());
    let (summary, command_text) = h.with_app(|app| {
        let ask = app
            .attention_queue
            .items
            .iter()
            .find(|b| b.category == crate::app::attention_queue::QueueCategory::Ask)
            .expect("secret.get ask is queued");
        (ask.summary.clone(), ask.command_text.clone())
    });
    assert!(
        summary.contains("wants to use secret.get"),
        "summary: {summary}"
    );
    assert!(
        command_text.contains("OPENAI_API_KEY"),
        "command: {command_text}"
    );
    assert!(
        command_text.contains(&root.path().display().to_string()),
        "command: {command_text}"
    );
    // Arrival alone never captures keys or grants consent.
    h.harness()
        .key_down_modifiers(egui::Modifiers::NONE, egui::Key::A);
    h.step();
    h.harness()
        .key_up_modifiers(egui::Modifiers::NONE, egui::Key::A);
    h.step();
    assert_eq!(monitor.list_pending().len(), 1);
    h.save_screenshot(&evidence_png!()).expect("render banner");
}

/// An unreadable host seal key is recoverable from the desktop. The action
/// is not a tool grant: it is the existing keychain retry/rotation path.
#[test]
fn screenshot_keychain_recovery_banner_is_actionable() {
    let profile = tempfile::tempdir().expect("profile");
    let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .unwrap();
    monitor
        .file_needs_you(crate::broker::gate::NeedsYouFile {
            kind: crate::broker::gate::NeedsYouKind::Keychain,
            actor: "host".to_string(),
            resource: "permission-mac".to_string(),
            summary: crate::broker::gate::KEYCHAIN_ACCESS_SUMMARY.to_string(),
            expires_at: None,
            run_tag: Some("keychain:permission-mac".to_string()),
        })
        .expect("file keychain recovery");

    let mut h = PlexiUiHarness::new_sized(1100.0, 360.0);
    h.run_steps(2);
    h.harness().get_by_label("Keychain access needed");
    h.harness()
        .get_by_label("Saved permissions are unavailable.");
    h.harness().get_by_label("Continue");
    h.harness().get_by_label("Not now");
    h.save_screenshot(&evidence_png!())
        .expect("render keychain recovery");
}

/// Raw `.wasm` paths queue a host pre-launch review before spawning. The
/// underlying launch path still fails closed until the modal persists the
/// required link-time import grants.
#[test]
fn wasm_path_launch_queues_review_before_spawning() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/wasm-fixtures/sysmon.wasm");
    let config_dir = tempfile::tempdir().expect("temp config dir");
    let _profile_guard = crate::config::set_test_profile_dir(config_dir.path().to_path_buf());
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);

    let result = h.with_app_mut(|app| {
        app.launch_view_by_path_with_layout(
            &fixture.to_string_lossy(),
            Some("split_v".to_string()),
            None,
            &[],
        )
    });

    result.expect("unreviewed wasm launch should queue review");
    h.step();
    let queued = h.with_app(|app| app.pending_raw_wasm_launches.len());
    assert_eq!(queued, 1, "raw .wasm path launch should queue one review");
    let has_review_focus =
        h.with_app(|app| matches!(app.focus_stack.last(), Some(FocusKind::RawWasmReview)));
    assert!(has_review_focus, "raw WASM review should own focus");
    let has_wasm = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .values()
            .any(|p| matches!(p, Pane::View(a) if matches!(a.runtime, ViewRuntime::Wasm(_))))
    });
    assert!(
        !has_wasm,
        "unreviewed .wasm path launch must not spawn a WASM pane"
    );

    // A deliberate approval comes after the consent arm delay (stint 1498).
    h.with_app_mut(|app| {
        app.raw_wasm_review_guard
            .backdate(crate::broker::consent::ARM_DELAY)
    });
    h.harness().key_press(egui::Key::Enter);
    h.step();
    let queued = h.with_app(|app| app.pending_raw_wasm_launches.len());
    assert_eq!(queued, 0, "approval should drain the raw WASM review queue");
    let has_wasm = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .values()
            .any(|p| matches!(p, Pane::View(a) if matches!(a.runtime, ViewRuntime::Wasm(_))))
    });
    assert!(
        has_wasm,
        "approving raw WASM review should spawn an ViewRuntime::Wasm pane"
    );
}

/// Once raw `.wasm` imports are reviewed and persisted for the path scope,
/// the same path-launch entry point must still spawn an `ViewRuntime::Wasm`
/// pane rather than attempting a manifest/process launch.
#[test]
fn wasm_path_launch_opens_wasm_pane_after_review() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/wasm-fixtures/sysmon.wasm");
    let config_dir = tempfile::tempdir().expect("temp config dir");
    let _profile_guard = crate::config::set_test_profile_dir(config_dir.path().to_path_buf());
    let plugin_id = fixture
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wasm");
    let workspace_root = fixture.parent().expect("fixture parent");
    let grants = crate::host::wasm_view::WasmView::inspect_required_grants(&fixture)
        .expect("inspect fixture grants");
    let mut store =
        crate::app::permissions::PermissionStore::load_or_default(config_dir.path()).unwrap();
    for capability_id in grants.capability_ids() {
        store.set_wasm(
            plugin_id,
            workspace_root,
            &capability_id,
            crate::app::permissions::PermissionState::Green,
        );
    }
    store.save();

    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    h.with_app_mut(|app| {
        app.launch_view_by_path_with_layout(
            &fixture.to_string_lossy(),
            Some("split_v".to_string()),
            None,
            &[],
        )
    })
    .expect("reviewed wasm launch should succeed");
    h.step();
    let has_wasm = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .values()
            .any(|p| matches!(p, Pane::View(a) if matches!(a.runtime, ViewRuntime::Wasm(_))))
    });
    assert!(
        has_wasm,
        "a reviewed .wasm path launch should produce an ViewRuntime::Wasm pane"
    );
}

/// Stint 1558 review: a consent dialog's buttons never take Tab focus,
/// whether laid out as a row or a stack and across the reflow between them.
#[test]
fn raw_wasm_review_buttons_take_no_focus_across_reflow() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("sysmon.wasm");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon.wasm"),
        &fixture,
    )
    .unwrap();
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    h.with_app_mut(|app| {
        app.launch_view_by_path_with_layout(
            &fixture.to_string_lossy(),
            Some("split_v".to_string()),
            None,
            &[],
        )
    })
    .expect("unreviewed wasm launch queues a review");
    h.run_steps(3);
    h.with_app_mut(|app| {
        app.raw_wasm_review_guard
            .backdate(crate::broker::consent::ARM_DELAY)
    });
    h.run_steps(2);
    let labels = ["Allow and remember", "Deny"];
    for (width, stacks) in [
        (1100.0, false),
        (240.0, true),
        (1100.0, false),
        (240.0, true),
    ] {
        assert_eq!(resize_settle(&mut h, width), stacks, "width {width}");
        for _ in 0..3 {
            tab(&mut h, false);
            let focused = focused_label(&mut h, &labels);
            assert!(
                focused.is_none(),
                "Tab focused {focused:?} at width {width}"
            );
        }
    }
    assert_eq!(
        h.with_app(|app| app.pending_raw_wasm_launches.len()),
        1,
        "nothing decided the review"
    );
}
