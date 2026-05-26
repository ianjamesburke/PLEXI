use gpui::{prelude::*, App, *};
use gpui_component::{ActiveTheme, Icon, IconName, h_flex, v_flex};

use crate::state::{all_commands, CommandEntry};

pub fn filtered_commands(query: &str) -> Vec<CommandEntry> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return all_commands();
    }
    all_commands()
        .into_iter()
        .filter(|c| {
            c.label.to_lowercase().contains(&q)
                || c.description.to_lowercase().contains(&q)
                || c.category.to_lowercase().contains(&q)
        })
        .collect()
}

pub fn render_command_palette(query: &str, selected: usize, cx: &App) -> impl IntoElement {
    let commands = filtered_commands(query);
    let is_empty_query = query.is_empty();

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_start()
        .justify_center()
        .pt(px(80.))
        .bg(hsla(240. / 360., 0.21, 0.06, 0.75))
        .child(
            v_flex()
                .w(px(560.))
                .rounded(px(8.))
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .overflow_hidden()
                // Search bar
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .items_center()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            Icon::new(IconName::Search)
                                .size(px(16.))
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(if is_empty_query {
                                    cx.theme().muted_foreground
                                } else {
                                    cx.theme().foreground
                                })
                                .child(if is_empty_query {
                                    "Search commands...".to_string()
                                } else {
                                    format!("{}_", query)
                                }),
                        ),
                )
                // Results
                .child(
                    div()
                        .max_h(px(360.))
                        .overflow_hidden()
                        .py_1()
                        .children(commands.iter().take(10).enumerate().map(|(i, cmd)| {
                            let is_sel = i == selected;
                            h_flex()
                                .px_3()
                                .py_1p5()
                                .gap_3()
                                .items_center()
                                .rounded(px(4.))
                                .mx_1()
                                .bg(if is_sel { cx.theme().accent } else { cx.theme().transparent })
                                .cursor_pointer()
                                .child(
                                    div()
                                        .text_xs()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(px(3.))
                                        .bg(cx.theme().muted)
                                        .text_color(cx.theme().muted_foreground)
                                        .child(cmd.category),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(if is_sel { cx.theme().accent_foreground } else { cx.theme().foreground })
                                                .child(cmd.label),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(cmd.description),
                                        ),
                                )
                                .when(!cmd.keys.is_empty(), |el| {
                                    el.child(
                                        h_flex()
                                            .gap_0p5()
                                            .children(cmd.keys.iter().map(|k| {
                                                div()
                                                    .text_xs()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded(px(3.))
                                                    .border_1()
                                                    .border_color(cx.theme().border)
                                                    .bg(cx.theme().muted)
                                                    .font_family("JetBrains Mono")
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(*k)
                                            })),
                                    )
                                })
                        }))
                        .when(commands.is_empty(), |el| {
                            el.child(
                                div()
                                    .p_4()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("No commands found"),
                            )
                        }),
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
                        .child(kbd_hint("↑↓", "navigate", cx))
                        .child(kbd_hint("⏎", "run", cx))
                        .child(kbd_hint("⎋", "close", cx)),
                ),
        )
}

fn kbd_hint(key: &'static str, label: &'static str, cx: &App) -> impl IntoElement {
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
                .child(key),
        )
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
}
