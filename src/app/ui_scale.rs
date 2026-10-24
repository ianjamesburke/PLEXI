//! App-wide UI scale (browser-style zoom) on `Cmd+=` / `Cmd+-` / `Cmd+Shift+0`.
//!
//! The scale is applied through `egui::Context::set_zoom_factor`, which
//! multiplies `pixels_per_point`, so every egui surface and the terminal cells
//! (their font is in points) scale together. It is persisted to the global
//! `config.toml` as `ui_scale`, debounced so held keys write once.

use super::PlexiApp;
use std::time::{Duration, Instant};

pub const UI_SCALE_MIN: f32 = 0.6;
pub const UI_SCALE_MAX: f32 = 2.0;
pub const UI_SCALE_STEP: f32 = 0.1;
pub const UI_SCALE_DEFAULT: f32 = 1.0;

const PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);

/// Clamp to the supported range and round to 2 decimals (no float drift).
/// Non-finite input falls back to the default.
pub fn clamp_scale(scale: f32) -> f32 {
    if !scale.is_finite() {
        return UI_SCALE_DEFAULT;
    }
    let clamped = scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
    (clamped * 100.0).round() / 100.0
}

/// Next 0.1 multiple above `scale`, capped at the maximum.
pub fn step_up(scale: f32) -> f32 {
    let s = clamp_scale(scale);
    let steps = (s / UI_SCALE_STEP + 1e-3).floor() + 1.0;
    clamp_scale(steps * UI_SCALE_STEP)
}

/// Next 0.1 multiple below `scale`, floored at the minimum.
pub fn step_down(scale: f32) -> f32 {
    let s = clamp_scale(scale);
    let steps = (s / UI_SCALE_STEP - 1e-3).ceil() - 1.0;
    clamp_scale(steps * UI_SCALE_STEP)
}

pub fn reset() -> f32 {
    UI_SCALE_DEFAULT
}

impl PlexiApp {
    /// Push `self.ui_scale` into egui when it differs. Cheap enough per frame.
    pub(crate) fn apply_ui_scale(&self, ctx: &egui::Context) {
        if (ctx.zoom_factor() - self.ui_scale).abs() > f32::EPSILON {
            ctx.set_zoom_factor(self.ui_scale);
            log::info!("ui_scale: applied {:.2}", self.ui_scale);
        }
    }

    /// Change the scale from a key action and schedule persistence.
    pub(crate) fn set_ui_scale(&mut self, new: f32) {
        let new = clamp_scale(new);
        if (new - self.ui_scale).abs() < 1e-4 {
            return;
        }
        log::info!("ui_scale: {:.2} -> {:.2}", self.ui_scale, new);
        self.ui_scale = new;
        self.ui_scale_persist_at = Some(Instant::now() + PERSIST_DEBOUNCE);
        self.apply_ui_scale(&self.ctx.clone());
        self.ctx.request_repaint_after(PERSIST_DEBOUNCE);
    }

    /// Adopt `ui_scale` from a freshly loaded config. A pending local change
    /// wins so a reload cannot undo keys pressed before the debounced write.
    pub(crate) fn adopt_config_ui_scale(&mut self, configured: Option<f32>) {
        if self.ui_scale_persist_at.is_some() {
            return;
        }
        let scale = configured.map_or(UI_SCALE_DEFAULT, clamp_scale);
        if (scale - self.ui_scale).abs() > 1e-4 {
            log::info!("ui_scale: config reload {:.2} -> {:.2}", self.ui_scale, scale);
            self.ui_scale = scale;
        }
    }

    /// Write the scale to the global config once the debounce has elapsed.
    /// Best-effort: failures are logged, never fatal.
    pub(crate) fn flush_ui_scale_persist(&mut self) {
        let Some(due) = self.ui_scale_persist_at else {
            return;
        };
        if Instant::now() < due {
            return;
        }
        self.ui_scale_persist_at = None;
        let scale = self.ui_scale;
        let path = crate::config::config_path();
        let result = crate::config::edit::edit_config_file(&path, |doc| {
            if (scale - UI_SCALE_DEFAULT).abs() < 1e-4 {
                crate::config::edit::remove_value(doc, &["ui_scale"]);
                Ok(())
            } else {
                crate::config::edit::set_value(
                    doc,
                    &["ui_scale"],
                    toml_edit::Value::from(f64::from(scale)),
                )
            }
        });
        match result {
            Ok(()) => log::info!("ui_scale: persisted {scale:.2} to {}", path.display()),
            Err(e) => log::warn!("ui_scale: could not persist: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_bounds_and_nan() {
        assert_eq!(clamp_scale(0.1), 0.6);
        assert_eq!(clamp_scale(9.0), 2.0);
        assert_eq!(clamp_scale(1.234), 1.23);
        assert_eq!(clamp_scale(f32::NAN), 1.0);
        assert_eq!(clamp_scale(f32::INFINITY), 1.0);
        assert_eq!(clamp_scale(f32::NEG_INFINITY), 1.0);
    }

    #[test]
    fn steps_land_on_tenths_without_drift() {
        let mut s = 1.0;
        let up = [1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 2.0, 2.0];
        for want in up {
            s = step_up(s);
            assert_eq!(s, want);
        }
        let down = [1.9, 1.8, 1.7, 1.6, 1.5, 1.4, 1.3, 1.2, 1.1, 1.0, 0.9, 0.8, 0.7, 0.6, 0.6];
        for want in down {
            s = step_down(s);
            assert_eq!(s, want);
        }
    }

    #[test]
    fn off_grid_values_snap_to_next_tenth() {
        assert_eq!(step_up(1.05), 1.1);
        assert_eq!(step_down(1.05), 1.0);
        assert_eq!(step_up(f32::NAN), 1.1);
    }

    #[test]
    fn reset_is_default() {
        assert_eq!(reset(), 1.0);
    }
}
