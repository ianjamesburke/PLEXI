use super::*;

#[test]
fn screenshot_changelog_modal_uses_host_chrome() {
    let mut h = PlexiUiHarness::new_sized(1024.0, 640.0);
    add_focused_pane(&mut h);
    h.step();
    h.with_app_mut(|app| {
        app.toggle_changelog();
    });
    h.run_steps(2);

    h.harness().get_by_label("Changelog");
    assert!(
        h.harness().get_all_by_label("Changes").count() > 0,
        "changelog body should render release section labels"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.show_changelog),
        "changelog should remain open for screenshot"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

fn set_update_ready(app: &mut crate::app::PlexiApp, version: &str) {
    app.update_status.begin_check();
    app.update_status
        .apply(crate::host::update_status::UpdateEvent::Ready {
            version: version.into(),
        });
}

#[test]
fn screenshot_command_palette_update_ready_row() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    add_focused_pane(&mut h);
    h.step();
    h.with_app_mut(|app| {
        set_update_ready(app, "0.5.1");
        app.show_command_palette = true;
        app.palette_selected = 0;
        app.palette_selected_key = None;
        app.sync_command_palette_focus();
    });
    h.run_steps(3);
    let row = h.harness().get_by_label("Update ready: v0.5.1").rect();
    let first_context = h.harness().get_by_label("CONTEXTS").rect();
    assert!(
        row.top() < first_context.top(),
        "the update row sits above every other result"
    );
    assert_eq!(
        h.with_app(|app| app.palette_selected_key.clone()),
        Some("update-ready:0.5.1".to_string()),
        "Enter on a fresh palette lands on the update row"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!());

    h.with_app_mut(|app| {
        app.update_status = crate::host::update_status::UpdateStatus::default();
        app.show_command_palette = true;
        app.palette_selected_key = None;
    });
    h.run_steps(2);
    assert!(
        h.harness().query_by_label("Update ready: v0.5.1").is_none(),
        "no ready build, no update row"
    );
}

#[test]
fn screenshot_changelog_update_status_line() {
    let mut h = PlexiUiHarness::new_sized(1024.0, 640.0);
    add_focused_pane(&mut h);
    h.step();
    h.with_app_mut(|app| {
        set_update_ready(app, "0.5.1");
        app.toggle_changelog();
    });
    h.run_steps(2);
    h.harness().get_by_label("v0.5.1 is ready");
    h.harness().get_by_label("Restart to update");
    h.save_screenshot(&evidence_png!("_ready")).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!("_ready"));

    h.with_app_mut(|app| app.offer_update_restart("test"));
    h.run_steps(2);
    h.harness().get_by_label(
        "Plexi will restart into v0.5.1. Running panes will close; your workspace will be saved.",
    );
    h.harness().get_by_label("Restart to v0.5.1");
    assert!(
        h.with_app(|app| !app.update_quit_pending),
        "showing the confirm never restarts"
    );
    h.save_screenshot(&evidence_png!("_confirm")).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!("_confirm"));
    h.with_app_mut(|app| app.update_confirm_prompt = false);

    h.with_app_mut(|app| {
        let mut status = crate::host::update_status::UpdateStatus::default();
        status.begin_check();
        status.apply(crate::host::update_status::UpdateEvent::Failed {
            reason: "GitHub is slow, try again".into(),
        });
        app.update_status = status;
    });
    h.run_steps(2);
    h.harness()
        .get_by_label("Update check failed: GitHub is slow, try again");
    assert!(h.harness().query_by_label("Restart to update").is_none());
    h.save_screenshot(&evidence_png!("_failed")).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!("_failed"));

    h.with_app_mut(|app| {
        let mut status = crate::host::update_status::UpdateStatus::default();
        status.begin_check();
        status.apply(crate::host::update_status::UpdateEvent::UpToDate {
            checked_at: crate::platform::clock::now_secs(),
        });
        app.update_status = status;
    });
    h.run_steps(2);
    h.harness().get_by_label("Up to date · checked just now");
    h.save_screenshot(&evidence_png!("_up_to_date")).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!("_up_to_date"));
}

/// Stint 1649: the changelog's update status panel in every state at a
/// normal, narrow, scaled and narrow-at-2x window. Every required action
/// keeps a full painted label, a hit rect inside the panel and no overlap
/// with the panel's text; the confirm step keeps the caption in place.
#[test]
fn screenshot_update_ui_matrix() {
    use crate::host::update_status::{UpdateEvent, UpdateStatus};
    let sizes: [(&str, f32, f32, f32); 4] = [
        ("normal", 1024.0, 640.0, 1.0),
        ("narrow", 420.0, 640.0, 1.0),
        ("scaled", 1024.0, 640.0, 1.5),
        ("narrow_scaled", 560.0, 640.0, 2.0),
    ];
    type Seed = Box<dyn Fn(&mut crate::app::PlexiApp)>;
    let after = |event: Option<UpdateEvent>| -> Seed {
        Box::new(move |app| {
            app.update_confirm_prompt = false;
            let mut s = UpdateStatus::default();
            s.begin_check();
            if let Some(event) = event.clone() {
                s.apply(event);
            }
            app.update_status = s;
        })
    };
    let states: Vec<(&str, Seed, &[&str])> = vec![
        ("checking", after(None), &[]),
        (
            "downloading",
            after(Some(UpdateEvent::Downloading { version: "0.5.1".into() })),
            &[],
        ),
        ("ready", after(Some(UpdateEvent::Ready { version: "0.5.1".into() })), &["offer"]),
        ("confirm", Box::new(|app| app.update_confirm_prompt = true), &["restart", "cancel"]),
        (
            "failed_long",
            after(Some(UpdateEvent::Failed {
                reason: "could not fetch releases from https://api.github.com/repos/plexi/plexi/releases/latest: HTTP 503 Service Unavailable".into(),
            })),
            &[],
        ),
        (
            "unavailable",
            after(Some(UpdateEvent::Unavailable {
                reason: "Updates are managed by your package manager; run brew upgrade plexi to update.".into(),
            })),
            &[],
        ),
    ];
    for (size, w, hh, scale) in sizes {
        let mut h = PlexiUiHarness::new_sized_scaled(w, hh, scale);
        add_focused_pane(&mut h);
        h.step();
        h.with_app_mut(|app| app.toggle_changelog());
        let mut ready_caption = None;
        for (state, seed, expected) in &states {
            h.with_app_mut(|app| seed(app));
            h.run_steps(3);
            let report = h.with_app(|app| {
                crate::ui::layout_report::snapshot(&app.ctx)
                    .into_iter()
                    .find(|r| r.surface == "status_panel")
                    .unwrap_or_else(|| panic!("{size}/{state}: no status_panel report"))
            });
            let at = format!("{size}/{state}");
            assert!(report.violations().is_empty(), "{at}: {:?}", report.violations());
            let ids: Vec<&str> = report.actions.iter().map(|a| a.id.as_str()).collect();
            assert_eq!(ids.as_slice(), *expected, "{at}: actions");
            for action in &report.actions {
                assert!(!action.painted_label.trim().is_empty(), "{at}: {} label", action.id);
                assert!(report.parent.contains_rect(action.hit), "{at}: {} leaves panel", action.id);
                for text in report.texts.iter().filter(|t| {
                    !report.actions.iter().any(|a| a.hit.contains_rect(t.paint))
                }) {
                    assert!(
                        !action.hit.intersects(text.paint),
                        "{at}: {} overlaps {:?}",
                        action.id,
                        text.text
                    );
                }
            }
            let caption = report
                .texts
                .iter()
                .find(|t| t.id == "caption")
                .unwrap_or_else(|| panic!("{at}: caption not painted"))
                .paint;
            match *state {
                "ready" => ready_caption = Some(caption),
                "confirm" => assert_eq!(Some(caption), ready_caption, "{at}: caption moved"),
                _ => {}
            }
            h.save_screenshot(&evidence_png!(&format!("_{size}_{state}")))
                .expect("render failed");
        }
    }
}
