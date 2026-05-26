use gpui::{prelude::*, App, FontWeight, *};
use gpui_component::{ActiveTheme, Icon, IconName, h_flex, v_flex};

struct ShortcutGroup {
    title: &'static str,
    entries: &'static [(&'static str, &'static [&'static str])],
}

static GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title: "Panes",
        entries: &[
            ("New terminal", &["⌘", "T"]),
            ("Split horizontal", &["⌘", "-"]),
            ("Split vertical", &["⌘", "\\"]),
            ("Close pane", &["⌘", "W"]),
            ("Zoom pane", &["⌘", "Z"]),
            ("Rename pane", &["⌘", "R"]),
        ],
    },
    ShortcutGroup {
        title: "Navigation",
        entries: &[
            ("Focus left", &["⌘", "H"]),
            ("Focus right", &["⌘", "L"]),
            ("Focus up", &["⌘", "K"]),
            ("Focus down", &["⌘", "J"]),
            ("Focus next", &["⌘", "]"]),
            ("Focus prev", &["⌘", "["]),
        ],
    },
    ShortcutGroup {
        title: "Contexts",
        entries: &[
            ("New context", &["⌘", "N"]),
            ("Rename context", &["⌘", "⇧", "R"]),
            ("Close context", &["⌘", "⇧", "W"]),
            ("Next context", &["⌘", "⇥"]),
            ("Prev context", &["⌘", "⇧", "⇥"]),
        ],
    },
    ShortcutGroup {
        title: "Overlays",
        entries: &[
            ("Command palette", &["⌘", "P"]),
            ("Context inspector", &["⌘", "I"]),
            ("Quick note", &["⌘", "0"]),
            ("Keyboard help", &["⌘", "/"]),
        ],
    },
];

pub fn render_help(cx: &App) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(240. / 360., 0.21, 0.06, 0.75))
        .child(
            v_flex()
                .w(px(560.))
                .rounded(px(8.))
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .overflow_hidden()
                // Header
                .child(
                    h_flex()
                        .px_4()
                        .py_3()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(IconName::Inspector)
                                        .size(px(16.))
                                        .text_color(cx.theme().accent),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(cx.theme().foreground)
                                        .child("Keyboard Shortcuts"),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(3.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .text_color(cx.theme().muted_foreground)
                                .child("⎋ close"),
                        ),
                )
                // Grid — 2 columns
                .child(
                    div()
                        .p_4()
                        .flex()
                        .flex_wrap()
                        .gap_4()
                        .children(GROUPS.iter().map(|group| {
                            v_flex()
                                .w(px(240.))
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(cx.theme().muted_foreground)
                                        .pb_1()
                                        .child(group.title),
                                )
                                .children(group.entries.iter().map(|(label, keys)| {
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .py_0p5()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().foreground)
                                                .child(*label),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_0p5()
                                                .children(keys.iter().map(|k| {
                                                    div()
                                                        .text_xs()
                                                        .px_1p5()
                                                        .py_0p5()
                                                        .rounded(px(3.))
                                                        .border_1()
                                                        .border_color(cx.theme().border)
                                                        .bg(cx.theme().muted)
                                                        .font_family("JetBrains Mono")
                                                        .text_color(cx.theme().foreground)
                                                        .child(*k)
                                                })),
                                        )
                                }))
                        })),
                ),
        )
}
