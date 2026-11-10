//! Native titlebar button area, expressed in AppKit points before UI zoom.

trait ButtonMetrics {
    fn native_button_area(&self) -> Option<egui::Vec2>;
}

impl ButtonMetrics for eframe::Frame {
    fn native_button_area(&self) -> Option<egui::Vec2> {
        native_button_area(self)
    }
}

#[cfg(target_os = "macos")]
fn native_button_area(frame: &eframe::Frame) -> Option<egui::Vec2> {
    use objc2_app_kit::{NSView, NSWindowButton};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let _main = objc2::MainThreadMarker::new()?;
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: eframe owns this NSView for the lifetime of the borrowed window
    // handle, and AppKit access here is restricted to the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let window = view.window()?;
    let content = window.contentView()?;
    let bounds = content.bounds();
    let mut right = 0.0_f64;
    let mut height = 0.0_f64;
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        let button = window.standardWindowButton(kind)?;
        let rect = content.convertRect_fromView(button.bounds(), Some(&button));
        let top = if content.isFlipped() {
            rect.origin.y - bounds.origin.y
        } else {
            bounds.origin.y + bounds.size.height - rect.origin.y - rect.size.height
        };
        right = right.max(rect.origin.x + rect.size.width - bounds.origin.x);
        // Preserve the button's measured top inset below it as well.
        height = height.max(top.max(0.0) * 2.0 + rect.size.height);
    }
    if !right.is_finite() || !height.is_finite() || right <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(egui::vec2(right as f32, height as f32))
}

#[cfg(not(target_os = "macos"))]
fn native_button_area(_frame: &eframe::Frame) -> Option<egui::Vec2> {
    None
}

#[cfg(test)]
struct TestMetrics(egui::Vec2);

#[cfg(test)]
impl ButtonMetrics for TestMetrics {
    fn native_button_area(&self) -> Option<egui::Vec2> {
        Some(self.0)
    }
}

fn metrics_from(source: &impl ButtonMetrics) -> Option<egui::Vec2> {
    source.native_button_area()
}

pub(crate) fn update_metrics(ctx: &egui::Context, frame: &eframe::Frame) {
    #[cfg(test)]
    let injected =
        ctx.data(|d| d.get_temp::<egui::Vec2>(egui::Id::new("native_titlebar_test_metrics")));
    #[cfg(test)]
    let metrics = injected
        .and_then(|size| metrics_from(&TestMetrics(size)))
        .or_else(|| metrics_from(frame));
    #[cfg(not(test))]
    let metrics = metrics_from(frame);
    #[cfg(all(target_os = "macos", not(test)))]
    if metrics.is_none() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let warning_id = egui::Id::new("native_titlebar_metrics_warning");
        if !ctx.data(|d| d.get_temp::<bool>(warning_id).unwrap_or(false)) {
            log::warn!("titlebar: unable to read native window button frames; retrying");
            ctx.data_mut(|d| d.insert_temp(warning_id, true));
        }
    }
    let id = egui::Id::new("native_titlebar_metrics");
    let previous = ctx.data(|d| d.get_temp::<egui::Vec2>(id));
    if previous != metrics {
        if let Some(size) = metrics {
            log::info!(
                "titlebar: native button area {:.1}x{:.1} points",
                size.x,
                size.y
            );
        } else if previous.is_some() {
            log::warn!("titlebar: native button area unavailable");
        }
        ctx.data_mut(|d| {
            if let Some(size) = metrics {
                d.insert_temp(id, size);
            } else {
                d.remove::<egui::Vec2>(id);
            }
        });
    }
}

pub(crate) fn logical_safe_area(ctx: &egui::Context) -> egui::Vec2 {
    ctx.data(|d| d.get_temp::<egui::Vec2>(egui::Id::new("native_titlebar_metrics")))
        .unwrap_or(egui::Vec2::ZERO)
        / ctx.zoom_factor()
}

pub(crate) fn toolbar_height(ctx: &egui::Context) -> f32 {
    crate::ui::style::TOOLBAR_H.max(logical_safe_area(ctx).y)
}
