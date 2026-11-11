//! Minimum text contrast for terminal glyphs.
//!
//! Modeled on Ghostty's `minimum-contrast`: `contrast_ratio` and
//! `contrasted_color` in `src/renderer/shaders/glsl/common.glsl`
//! (ghostty-org/ghostty), applied per glyph in `cell_text.v.glsl`. Same WCAG
//! 2.x ratio on linearized sRGB, same choice of black or white as the target
//! (whichever contrasts more with the background), same "1.0 means off".
//!
//! One deliberate difference: Ghostty snaps a failing color straight to pure
//! white or black, which erases hue (a pale-blue inline-code color becomes
//! black). Here the color is blended toward the same extreme by the smallest
//! amount that reaches the minimum, so hue and most of the lightness survive.
//! Because luminance is linear in linear RGB, that amount has a closed form.
//! When even the pure extreme cannot reach the minimum, the result is the pure
//! extreme, which is exactly Ghostty's output.

use egui::Color32;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Lowest accepted minimum: 1.0 is the WCAG floor and means "off".
pub const MIN_CONTRAST_OFF: f32 = 1.0;
/// Highest accepted minimum: the WCAG ratio of black on white.
pub const MIN_CONTRAST_MAX: f32 = 21.0;

fn linear_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0.0; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *slot = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        lut
    })
}

fn linear_to_srgb_u8(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

fn linear_rgb(c: Color32) -> [f32; 3] {
    let lut = linear_lut();
    [
        lut[c.r() as usize],
        lut[c.g() as usize],
        lut[c.b() as usize],
    ]
}

fn luminance_of(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn ratio(l1: f32, l2: f32) -> f32 {
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

/// WCAG contrast ratio between two opaque colors, in `1.0..=21.0`.
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    ratio(luminance_of(linear_rgb(a)), luminance_of(linear_rgb(b)))
}

// Bounded per-render-thread cache, shared by all panes. Key the final painted
// colors (including alpha) and exact minimum: theme/selection/SGR/cursor changes
// cannot reuse an adjustment for a different input. The last-pair shortcut
// handles runs of text without hashing, locking, or cloning a per-view cache.
const CACHE_CAPACITY: usize = 4096;
type ContrastKey = (Color32, Color32, u32);

#[derive(Default)]
struct ContrastCache {
    colors: HashMap<ContrastKey, Color32>,
    last: Option<(ContrastKey, Color32)>,
    #[cfg(test)]
    computations: usize,
}

impl ContrastCache {
    fn get(&mut self, fg: Color32, bg: Color32, minimum: f32) -> Color32 {
        let key = (fg, bg, minimum.to_bits());
        if let Some((previous, color)) = self.last {
            if previous == key {
                return color;
            }
        }
        let color = if let Some(&color) = self.colors.get(&key) {
            color
        } else {
            let color = compute_min_contrast(fg, bg, minimum);
            #[cfg(test)]
            {
                self.computations += 1;
            }
            if self.colors.len() == CACHE_CAPACITY {
                self.colors.clear();
            }
            self.colors.insert(key, color);
            color
        };
        self.last = Some((key, color));
        color
    }
}

thread_local! {
    static CACHE: RefCell<ContrastCache> = RefCell::new(ContrastCache::default());
}

/// Return `fg` unchanged when its contrast against `bg` already meets
/// `minimum`; otherwise the closest color toward black or white that does.
/// `minimum <= 1.0` disables the adjustment. Alpha is carried through.
pub fn ensure_min_contrast(fg: Color32, bg: Color32, minimum: f32) -> Color32 {
    if minimum <= MIN_CONTRAST_OFF {
        return fg;
    }
    CACHE.with(|cache| cache.borrow_mut().get(fg, bg, minimum))
}

fn compute_min_contrast(fg: Color32, bg: Color32, minimum: f32) -> Color32 {
    if minimum <= MIN_CONTRAST_OFF {
        return fg;
    }
    let fg_lin = linear_rgb(fg);
    let fg_lum = luminance_of(fg_lin);
    let bg_lum = luminance_of(linear_rgb(bg));
    if ratio(fg_lum, bg_lum) >= minimum {
        return fg;
    }

    // Ghostty's target rule: the extreme with the higher ratio against bg.
    let to_white = ratio(1.0, bg_lum) > ratio(0.0, bg_lum);
    let (target_lum, target_lin) = if to_white {
        (1.0, [1.0; 3])
    } else {
        (0.0, [0.0; 3])
    };
    // Luminance needed on the target side of the background.
    let needed = if to_white {
        minimum * (bg_lum + 0.05) - 0.05
    } else {
        (bg_lum + 0.05) / minimum - 0.05
    };
    let mut t = ((needed - fg_lum) / (target_lum - fg_lum)).clamp(0.0, 1.0);

    // 8-bit rounding can land a hair under the minimum; step toward the target
    // until the quantized result passes, ending at the pure extreme at worst.
    loop {
        let mixed = [
            fg_lin[0] + (target_lin[0] - fg_lin[0]) * t,
            fg_lin[1] + (target_lin[1] - fg_lin[1]) * t,
            fg_lin[2] + (target_lin[2] - fg_lin[2]) * t,
        ];
        let out = Color32::from_rgba_premultiplied(
            linear_to_srgb_u8(mixed[0]),
            linear_to_srgb_u8(mixed[1]),
            linear_to_srgb_u8(mixed[2]),
            fg.a(),
        );
        if t >= 1.0 || contrast_ratio(out, bg) >= minimum {
            return out;
        }
        t = (t + 0.005).min(1.0);
    }
}

/// Glyphs drawn in color or as images are exempt (Ghostty: `NO_MIN_CONTRAST`).
/// Pictographic and emoji blocks only; dingbats such as check marks and
/// prompts stay subject to the minimum.
pub fn is_exempt_glyph(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAWN_BG: Color32 = Color32::from_rgb(0xf4, 0xec, 0xdf);
    const DARK_GREY: Color32 = Color32::from_rgb(153, 153, 153);
    const PALE_BLUE: Color32 = Color32::from_rgb(177, 185, 249);

    #[test]
    fn unchanged_cell_inputs_are_computed_once_across_resize_frames() {
        CACHE.with(|cache| *cache.borrow_mut() = ContrastCache::default());
        for _frame in 0..120 {
            for _cell in 0..100 {
                for fg in [DARK_GREY, PALE_BLUE] {
                    assert_eq!(
                        ensure_min_contrast(fg, DAWN_BG, 3.0),
                        compute_min_contrast(fg, DAWN_BG, 3.0)
                    );
                }
            }
        }
        CACHE.with(|cache| assert_eq!(cache.borrow().computations, 2));
    }

    #[test]
    fn cache_keys_every_color_input_and_stays_bounded() {
        let mut cache = ContrastCache::default();
        for fg in [
            DARK_GREY,
            PALE_BLUE,
            Color32::from_rgba_premultiplied(100, 100, 100, 128),
        ] {
            for bg in [DAWN_BG, Color32::BLACK, Color32::WHITE] {
                for minimum in [1.0, 3.0, 4.5, 21.0] {
                    assert_eq!(
                        cache.get(fg, bg, minimum),
                        compute_min_contrast(fg, bg, minimum)
                    );
                }
            }
        }
        assert_eq!(cache.computations, 36);
        for value in 0..(CACHE_CAPACITY * 2) {
            let fg = Color32::from_rgb(value as u8, (value >> 8) as u8, 100);
            assert_eq!(
                cache.get(fg, DAWN_BG, 3.0),
                compute_min_contrast(fg, DAWN_BG, 3.0)
            );
            assert!(cache.colors.len() <= CACHE_CAPACITY);
        }
        assert_eq!(
            cache.get(DARK_GREY, DAWN_BG, 3.0),
            compute_min_contrast(DARK_GREY, DAWN_BG, 3.0)
        );
    }

    /// CPU-only probe for the per-cell path, independent of debug host/layout
    /// instrumentation. Report separately from full ui() and installed timings.
    #[test]
    #[ignore = "perf-gate: run resize contrast probe on a quiet machine"]
    fn perf_gate_resize_contrast_cost() {
        use std::hint::black_box;
        use std::time::Instant;
        for enabled in [false, true] {
            let mut samples = Vec::new();
            for frame in 0..240 {
                let start = Instant::now();
                for _pane in 0..4 {
                    for cell in 0..((100 - frame % 30) * 40) {
                        let fg = black_box(if cell % 2 == 0 {
                            DARK_GREY
                        } else {
                            PALE_BLUE
                        });
                        black_box(if enabled {
                            ensure_min_contrast(
                                fg,
                                black_box(DAWN_BG),
                                black_box(3.0),
                            )
                        } else {
                            fg
                        });
                    }
                }
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            println!("CONTRAST_PROBE enabled={enabled} frames=240 p50_ms={:.4} p95_ms={:.4}", samples[120], samples[227]);
        }
    }

    #[test]
    fn low_contrast_pairs_on_light_bg_are_lifted_to_the_minimum() {
        for fg in [DARK_GREY, PALE_BLUE] {
            assert!(contrast_ratio(fg, DAWN_BG) < 3.0);
            let out = ensure_min_contrast(fg, DAWN_BG, 3.0);
            assert!(
                contrast_ratio(out, DAWN_BG) >= 3.0,
                "{fg:?} -> {out:?} ratio {}",
                contrast_ratio(out, DAWN_BG)
            );
        }
    }

    #[test]
    fn low_contrast_pairs_on_dark_bg_are_lifted_toward_white() {
        let bg = Color32::from_rgb(0x1a, 0x1b, 0x26);
        let fg = Color32::from_rgb(0x30, 0x32, 0x45);
        let out = ensure_min_contrast(fg, bg, 3.0);
        assert!(contrast_ratio(out, bg) >= 3.0);
        assert!(out.r() > fg.r());
    }

    #[test]
    fn already_readable_pairs_are_unchanged() {
        let fg = Color32::from_rgb(0x2b, 0x2b, 0x33);
        assert_eq!(ensure_min_contrast(fg, DAWN_BG, 3.0), fg);
        let dark_bg = Color32::from_rgb(0x1a, 0x1b, 0x26);
        let light = Color32::from_rgb(0xe8, 0xe6, 0xed);
        assert_eq!(ensure_min_contrast(light, dark_bg, 3.0), light);
    }

    #[test]
    fn exact_black_and_white_stay_put() {
        for bg in [DAWN_BG, Color32::from_rgb(0x1a, 0x1b, 0x26)] {
            let best = if contrast_ratio(Color32::BLACK, bg)
                > contrast_ratio(Color32::WHITE, bg)
            {
                Color32::BLACK
            } else {
                Color32::WHITE
            };
            assert_eq!(ensure_min_contrast(best, bg, 3.0), best);
        }
        let moved = ensure_min_contrast(Color32::BLACK, Color32::BLACK, 3.0);
        assert!(contrast_ratio(moved, Color32::BLACK) >= 3.0);
    }

    #[test]
    fn minimum_of_one_turns_it_off() {
        assert_eq!(ensure_min_contrast(DARK_GREY, DAWN_BG, 1.0), DARK_GREY);
        assert_eq!(ensure_min_contrast(PALE_BLUE, DAWN_BG, 0.5), PALE_BLUE);
    }

    #[test]
    fn hue_is_kept_when_a_small_move_is_enough() {
        // Mildly low contrast: the blue channel must stay the largest.
        let fg = Color32::from_rgb(110, 130, 230);
        let bg = Color32::from_rgb(0xf4, 0xec, 0xdf);
        let out = ensure_min_contrast(fg, bg, 3.0);
        assert!(contrast_ratio(out, bg) >= 3.0);
        assert!(
            out.b() > out.r() && out.b() > out.g(),
            "{out:?} lost its hue"
        );
    }

    #[test]
    fn unreachable_minimum_falls_back_to_the_pure_extreme() {
        let bg = Color32::from_rgb(120, 120, 120);
        let out =
            ensure_min_contrast(Color32::from_rgb(110, 110, 110), bg, 21.0);
        assert!(out == Color32::BLACK || out == Color32::WHITE);
    }

    #[test]
    fn sweep_never_ends_below_the_minimum_when_reachable() {
        for bg in [DAWN_BG, Color32::from_rgb(0x1a, 0x1b, 0x26), Color32::WHITE]
        {
            for r in (0..=255).step_by(15) {
                for g in (0..=255).step_by(15) {
                    for b in (0..=255).step_by(15) {
                        let fg = Color32::from_rgb(r, g, b);
                        let out = ensure_min_contrast(fg, bg, 3.0);
                        assert!(
                            contrast_ratio(out, bg) >= 3.0,
                            "{fg:?} on {bg:?} -> {out:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn emoji_blocks_are_exempt_but_dingbats_are_not() {
        assert!(is_exempt_glyph('\u{1F600}'));
        assert!(!is_exempt_glyph('\u{2713}'));
        assert!(!is_exempt_glyph('a'));
    }
}
