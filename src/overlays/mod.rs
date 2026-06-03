use crate::app_trait::AppCommand;
use crate::style;
use crate::theme::Colors;
use crate::tiling::PaneId;
use egui::{Align, Align2, Color32, CornerRadius, Layout, RichText, Stroke, Vec2};

pub(crate) mod toolbar;
pub(crate) mod misc;
pub(crate) mod quick_note;
pub(crate) mod confirmations;
pub(crate) mod notification_modal;
pub(crate) mod setup;

/// Consume the first digit key (0–9) pressed this frame; return its value.
pub(crate) fn consume_digit_key(ctx: &egui::Context) -> Option<u8> {
    ctx.input_mut(|i| {
        let keys = [
            (egui::Key::Num0, 0u8),
            (egui::Key::Num1, 1),
            (egui::Key::Num2, 2),
            (egui::Key::Num3, 3),
            (egui::Key::Num4, 4),
            (egui::Key::Num5, 5),
            (egui::Key::Num6, 6),
            (egui::Key::Num7, 7),
            (egui::Key::Num8, 8),
            (egui::Key::Num9, 9),
        ];
        for (key, n) in keys {
            if i.consume_key(egui::Modifiers::NONE, key) {
                return Some(n);
            }
        }
        None
    })
}

use crate::app::PlexiApp;

pub(crate) const MODAL_WIDTH: f32 = 400.0;
pub(crate) const R6: CornerRadius = CornerRadius::same(6);

/// Show a native macOS folder picker using rfd's async API.
/// Blocks the calling (background) thread; must NOT be called on the main thread.
pub(crate) fn pick_folder() -> Option<std::path::PathBuf> {
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use std::pin::pin;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        struct Signal(Arc<(Mutex<bool>, Condvar)>);
        impl Wake for Signal {
            fn wake(self: Arc<Self>) { self.signal(); }
            fn wake_by_ref(self: &Arc<Self>) { self.signal(); }
        }
        impl Signal {
            fn signal(&self) {
                let (lock, cvar) = &*self.0;
                *lock.lock().unwrap() = true;
                cvar.notify_one();
            }
        }
        let waker: Waker = Arc::new(Signal(Arc::clone(&signal))).into();
        let mut cx = Context::from_waker(&waker);
        let mut f = pin!(f);
        loop {
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(val) => return val,
                Poll::Pending => {
                    let (lock, cvar) = &*signal;
                    let mut ready = lock.lock().unwrap();
                    while !*ready { ready = cvar.wait(ready).unwrap(); }
                    *ready = false;
                }
            }
        }
    }

    let dialog = rfd::AsyncFileDialog::new();
    let handle = block_on(dialog.pick_folder())?;
    Some(handle.path().to_path_buf())
}

pub(crate) fn draw_contact_footer(ui: &mut egui::Ui, colors: &Colors) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(
                "If you have any ideas, want to help, or just want to say what's up...",
            )
            .size(style::TEXT_CAPTION)
            .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM / 2.0);
        {
            let email = "ADHDISNTREAL@GMAIL.COM";
            let mailto = "mailto:ADHDisntreal@gmail.com";
            let font_id = egui::FontId::proportional(style::TEXT_CAPTION);
            let email_w = ui.fonts(|f| {
                f.layout_no_wrap(email.to_string(), font_id, colors.text_dim)
                    .size()
                    .x
            });
            let btn_w = 24.0;
            let gap = ui.spacing().item_spacing.x;
            let pad = ((ui.available_width() - email_w - gap - btn_w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.hyperlink_to(
                    RichText::new(email)
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                    mailto,
                );
                crate::widgets::copy_button(
                    ui,
                    egui::Id::new("shortcuts_email_copy"),
                    "ADHDisntreal@gmail.com",
                );
            });
        }
        ui.add_space(style::SPACE_SM / 2.0);
        ui.hyperlink_to(
            RichText::new("❤️  Support the project")
                .size(style::TEXT_CAPTION)
                .color(colors.text_dim),
            "https://buymeacoffee.com/ianjamesbu8",
        );
    });
}
