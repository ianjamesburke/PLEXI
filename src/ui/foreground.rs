//! Semantic foregrounds resolved after surface composition, never by theme
//! name. Callers supply the opaque backdrop and the actual painted fill.
use super::theme;
use egui::Color32;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Role {
    SmallText,
    ControlMark,
}

impl Role {
    pub fn minimum(self) -> f32 {
        match self {
            Self::SmallText => 4.5,
            Self::ControlMark => 3.0,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Surface {
    pub fill: Color32,
}

impl Surface {
    pub fn over(backdrop: Color32, fill: Color32) -> Self {
        Self {
            fill: backdrop.blend(fill),
        }
    }

    pub fn foreground(self, preferred: Color32, role: Role) -> Color32 {
        let color = self.fill.blend(preferred);
        let ratio = |fg| theme::contrast(theme::luminance(fg), theme::luminance(self.fill));
        if ratio(color) >= role.minimum() {
            return color;
        }
        static CORRECTION: std::sync::Once = std::sync::Once::new();
        CORRECTION
            .call_once(|| log::info!("ui foreground: correcting composited surface contrast"));
        let target = if ratio(Color32::WHITE) >= ratio(Color32::BLACK) {
            Color32::WHITE
        } else {
            Color32::BLACK
        };
        (1..=64)
            .map(|step| color.lerp_to_gamma(target, step as f32 / 64.0))
            .find(|candidate| ratio(*candidate) >= role.minimum())
            .unwrap_or(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_roles_cover_alpha_and_custom_theme_boundaries() {
        for base in [0, 32, 96, 117, 128, 160, 224, 255] {
            for alpha in [0, 16, 64, 128, 192, 255] {
                for rgb in [
                    [0, 0, 0],
                    [255, 255, 255],
                    [255, 0, 128],
                    [32, 200, 64],
                    [117, 117, 117],
                ] {
                    let surface = Surface::over(
                        Color32::from_gray(base),
                        Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha),
                    );
                    for role in [Role::SmallText, Role::ControlMark] {
                        let fg = surface.foreground(
                            Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha),
                            role,
                        );
                        assert_eq!(fg.a(), 255);
                        assert!(
                            theme::contrast(theme::luminance(fg), theme::luminance(surface.fill))
                                >= role.minimum()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn low_contrast_negative_control_is_rejected() {
        let fill = Color32::from_gray(117);
        let wrong = fill;
        assert!(
            theme::contrast(theme::luminance(wrong), theme::luminance(fill))
                < Role::SmallText.minimum()
        );
        let fixed = Surface::over(Color32::BLACK, fill).foreground(wrong, Role::SmallText);
        assert!(
            theme::contrast(theme::luminance(fixed), theme::luminance(fill))
                >= Role::SmallText.minimum()
        );
    }
}

/// Undo egui's `multiply_opacity` on an emitted colour: the same colour at the
/// alpha it had before the ui's `opacity` was applied. Post-paint passes that
/// resolve text against emitted shapes must resolve undimmed colours and dim
/// the result, or an unfocused (dimmed) pane picks a different text colour
/// than a focused one (stint 1404).
pub(crate) fn undim(color: Color32, opacity: f32) -> Color32 {
    if opacity <= 0.0 || opacity >= 1.0 {
        return color;
    }
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let alpha = (f32::from(a) / opacity).round().min(255.0) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}
