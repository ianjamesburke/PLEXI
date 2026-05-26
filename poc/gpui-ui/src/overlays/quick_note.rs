use gpui::{prelude::*, App, *};
use gpui_component::{ActiveTheme, Icon, IconName, h_flex, v_flex};

pub const DESTINATIONS: &[&str] = &[
    "~/Documents/notes.md",
    "~/Documents/github/daily_log/2026-05-26_claude.md",
    ".plexi/notes.md",
];

pub fn render_quick_note(text: &str, dest_index: usize, cx: &App) -> impl IntoElement {
    let dest = DESTINATIONS.get(dest_index).copied().unwrap_or("~/Documents/notes.md");
    let display_text = if text.is_empty() {
        "Type a note...".to_string()
    } else {
        format!("{}_", text)
    };
    let is_empty = text.is_empty();

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_end()
        .justify_center()
        .pb(px(60.))
        .bg(hsla(240. / 360., 0.21, 0.06, 0.5))
        .child(
            v_flex()
                .w(px(500.))
                .rounded(px(8.))
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .overflow_hidden()
                // Destination bar
                .child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .bg(cx.theme().muted)
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(Icon::new(IconName::File).size(px(12.)).text_color(cx.theme().muted_foreground))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Destination:"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("JetBrains Mono")
                                .text_color(cx.theme().foreground)
                                .child(dest),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("⇥ change"),
                        ),
                )
                // Input area
                .child(
                    div()
                        .p_3()
                        .min_h(px(80.))
                        .child(
                            div()
                                .text_sm()
                                .font_family("JetBrains Mono")
                                .text_color(if is_empty { cx.theme().muted_foreground } else { cx.theme().foreground })
                                .child(display_text),
                        ),
                )
                // Footer
                .child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_3()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().muted)
                        .child(qn_hint("⏎", "append", cx))
                        .child(qn_hint("⇥", "change dest", cx))
                        .child(qn_hint("⎋", "cancel", cx)),
                ),
        )
}

fn qn_hint(key: &'static str, label: &'static str, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(
            div()
                .px_1()
                .rounded(px(2.))
                .border_1()
                .border_color(cx.theme().border)
                .text_xs()
                .font_family("JetBrains Mono")
                .text_color(cx.theme().muted_foreground)
                .child(key),
        )
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
}
