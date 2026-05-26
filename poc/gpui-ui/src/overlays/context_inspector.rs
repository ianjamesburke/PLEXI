use gpui::{prelude::*, App, FontWeight, *};
use gpui_component::{ActiveTheme, Icon, IconName, h_flex, v_flex};

use crate::state::{ContextInfo, PaneKind, PaneState, PaneStatus};

pub fn render_context_inspector(
    contexts: &[ContextInfo],
    panes: &[PaneState],
    active_context: usize,
    cx: &App,
) -> impl IntoElement {
    let ctx = contexts.get(active_context);
    let ctx_name = ctx.map(|c| c.name.clone()).unwrap_or_else(|| "—".into());

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_start()
        .justify_end()
        .bg(hsla(240. / 360., 0.21, 0.06, 0.5))
        .child(
            v_flex()
                .w(px(300.))
                .h_full()
                .bg(cx.theme().popover)
                .border_l_1()
                .border_color(cx.theme().border)
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
                                    Icon::new(IconName::LayoutDashboard)
                                        .size(px(14.))
                                        .text_color(cx.theme().accent),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(cx.theme().foreground)
                                        .child(ctx_name),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("⌘I"),
                        ),
                )
                // Context list
                .child(
                    v_flex()
                        .px_2()
                        .py_2()
                        .gap_0p5()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("CONTEXTS"),
                        )
                        .children(contexts.iter().enumerate().map(|(i, ctx)| {
                            let is_active = i == active_context;
                            h_flex()
                                .px_2()
                                .py_1p5()
                                .gap_2()
                                .items_center()
                                .rounded(px(4.))
                                .bg(if is_active { cx.theme().accent } else { cx.theme().transparent })
                                .child(
                                    Icon::new(IconName::Folder)
                                        .size(px(13.))
                                        .text_color(if is_active { cx.theme().accent_foreground } else { cx.theme().muted_foreground }),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .text_sm()
                                                .truncate()
                                                .text_color(if is_active { cx.theme().accent_foreground } else { cx.theme().foreground })
                                                .child(ctx.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .truncate()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(ctx.cwd.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(px(10.))
                                        .bg(cx.theme().muted)
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{}", ctx.pane_ids.len())),
                                )
                        })),
                )
                // Panes in active context
                .child(
                    v_flex()
                        .flex_1()
                        .px_2()
                        .py_2()
                        .gap_0p5()
                        .overflow_hidden()
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("PANES"),
                        )
                        .children({
                            let pane_rows: Vec<_> = ctx
                                .map(|c| {
                                    c.pane_ids.iter()
                                        .filter_map(|id| panes.iter().find(|p| p.id == *id))
                                        .map(|pane| render_pane_row(pane, cx))
                                        .collect()
                                })
                                .unwrap_or_default();
                            pane_rows
                        }),
                )
                // Footer
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_3()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(Icon::new(IconName::Plus).size(px(11.)))
                                .child("New pane"),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(Icon::new(IconName::Settings).size(px(11.)))
                                .child("Settings"),
                        ),
                ),
        )
}

fn render_pane_row(pane: &PaneState, cx: &App) -> impl IntoElement {
    let status_color = match pane.status {
        PaneStatus::Running | PaneStatus::Busy => cx.theme().success,
        PaneStatus::Error | PaneStatus::Exited => cx.theme().danger,
        PaneStatus::Idle => cx.theme().muted_foreground,
    };

    let kind_icon = match pane.kind {
        PaneKind::Terminal => IconName::SquareTerminal,
        PaneKind::App => IconName::Bot,
        PaneKind::Agent => IconName::Cpu,
    };

    h_flex()
        .px_2()
        .py_1p5()
        .gap_2()
        .items_center()
        .rounded(px(4.))
        .child(div().size(px(6.)).rounded_full().bg(status_color).flex_shrink_0())
        .child(Icon::new(kind_icon).size(px(13.)).text_color(cx.theme().muted_foreground))
        .child(
            v_flex()
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .text_color(cx.theme().foreground)
                        .child(pane.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(pane.cwd.clone()),
                ),
        )
        .child(
            div()
                .text_xs()
                .px_1()
                .rounded(px(3.))
                .bg(cx.theme().muted)
                .text_color(cx.theme().muted_foreground)
                .child(pane.kind.label()),
        )
}
