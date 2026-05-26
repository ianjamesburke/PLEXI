mod overlays;
mod state;

use gpui::prelude::FluentBuilder as _;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, ThemeMode,
    button::{Button, ButtonVariants},
    menu::{ContextMenuExt, PopupMenuItem},
    sidebar::{
        Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
        SidebarToggleButton,
    },
    h_flex, v_flex,
    *,
};
use gpui_component_assets::Assets;

use overlays::{
    command_palette::render_command_palette,
    context_inspector::render_context_inspector,
    help::render_help,
    quick_note::render_quick_note,
};
use state::{
    ActiveOverlay, ContextInfo, NotificationState, PaneKind, PaneState, PaneStatus, TileLayout,
};

actions!(
    plexi,
    [
        Quit,
        ToggleCommandPalette,
        ToggleSidebar,
        ToggleContextInspector,
        ToggleHelp,
        ToggleQuickNote,
        DismissOverlay,
        NewTerminal,
        ClosePane,
        SplitHorizontal,
        SplitVertical,
        FocusNext,
        FocusPrev,
        ZoomPane,
        NewContext,
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
    ]
);

// ─── App State ───────────────────────────────────────────────────────────────

struct PlexiApp {
    panes: Vec<PaneState>,
    focused_pane: usize,
    zoomed_pane: Option<usize>,
    layout: TileLayout,
    contexts: Vec<ContextInfo>,
    active_context: usize,
    sidebar_collapsed: bool,
    overlay: Option<ActiveOverlay>,
    palette_query: String,
    palette_selected: usize,
    quick_note_text: String,
    quick_note_dest: usize,
    notification: Option<NotificationState>,
    _keystroke_sub: Subscription,
}

impl PlexiApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let panes = vec![
            PaneState { id: 0, name: "zsh".into(), kind: PaneKind::Terminal, status: PaneStatus::Running, cwd: "~/Documents/GitHub/PLEXI".into() },
            PaneState { id: 1, name: "claude code".into(), kind: PaneKind::App, status: PaneStatus::Busy, cwd: "~/Documents/GitHub/PLEXI".into() },
            PaneState { id: 2, name: "cargo watch".into(), kind: PaneKind::Terminal, status: PaneStatus::Running, cwd: "~/Documents/GitHub/PLEXI".into() },
            PaneState { id: 3, name: "git log".into(), kind: PaneKind::Terminal, status: PaneStatus::Idle, cwd: "~/Documents/GitHub/PLEXI".into() },
        ];

        let contexts = vec![
            ContextInfo { id: 0, name: "PLEXI".into(), cwd: "~/Documents/GitHub/PLEXI".into(), pane_ids: vec![0, 1, 2, 3], children: vec![] },
            ContextInfo { id: 1, name: "dotfiles".into(), cwd: "~/dotfiles".into(), pane_ids: vec![], children: vec![] },
            ContextInfo { id: 2, name: "labs".into(), cwd: "~/Documents/labs".into(), pane_ids: vec![], children: vec![] },
        ];

        let layout = TileLayout::VSplit {
            ratio: 0.5,
            top: Box::new(TileLayout::HSplit {
                ratio: 0.5,
                left: Box::new(TileLayout::Leaf(0)),
                right: Box::new(TileLayout::Leaf(1)),
            }),
            bottom: Box::new(TileLayout::HSplit {
                ratio: 0.5,
                left: Box::new(TileLayout::Leaf(2)),
                right: Box::new(TileLayout::Leaf(3)),
            }),
        };

        // Keystroke observer — handles text input for command palette and quick note
        let sub = cx.observe_keystrokes(|this, event, _window, cx| {
            // Only handle unbound keystrokes (action == None) as text input
            if event.action.is_some() {
                return;
            }
            let ks = &event.keystroke;
            // Ignore modifier-only and cmd/ctrl combos
            if ks.modifiers.platform || ks.modifiers.control || ks.modifiers.function {
                return;
            }

            match this.overlay {
                Some(ActiveOverlay::CommandPalette) => {
                    if ks.key == "backspace" {
                        this.palette_query.pop();
                        this.palette_selected = 0;
                    } else if ks.key == "up" {
                        this.palette_selected = this.palette_selected.saturating_sub(1);
                    } else if ks.key == "down" {
                        this.palette_selected = (this.palette_selected + 1).min(9);
                    } else if ks.key == "return" {
                        // Execute selected command — dismiss for now
                        this.dismiss_overlay(cx);
                        return;
                    } else if let Some(ch) = &ks.key_char {
                        if !ch.chars().any(|c| c.is_control()) {
                            this.palette_query.push_str(ch);
                            this.palette_selected = 0;
                        }
                    }
                    cx.notify();
                }
                Some(ActiveOverlay::QuickNote) => {
                    if ks.key == "backspace" {
                        this.quick_note_text.pop();
                    } else if ks.key == "tab" {
                        this.quick_note_dest = (this.quick_note_dest + 1) % 3;
                    } else if ks.key == "return" && !ks.modifiers.alt {
                        // Append to destination file
                        let text = this.quick_note_text.clone();
                        let dest = match this.quick_note_dest {
                            1 => "~/Documents/github/daily_log/2026-05-26_claude.md",
                            2 => ".plexi/notes.md",
                            _ => "~/Documents/notes.md",
                        };
                        let dest = dest.replace("~", &std::env::var("HOME").unwrap_or_default());
                        let _ = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&dest)
                            .and_then(|mut f| {
                                use std::io::Write;
                                writeln!(f, "\n{}", text)
                            });
                        this.dismiss_overlay(cx);
                        return;
                    } else if let Some(ch) = &ks.key_char {
                        if !ch.chars().any(|c| c.is_control()) {
                            this.quick_note_text.push_str(ch);
                        }
                    }
                    cx.notify();
                }
                _ => {}
            }
        });

        // Demo: show a notification after 3s, auto-dismiss after 5 more
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
            let _ = this.update(cx, |app: &mut PlexiApp, cx| {
                app.notification = Some(NotificationState {
                    title: "cargo watch".into(),
                    body: "Build succeeded in 4.2s".into(),
                    source_pane: Some(2),
                    dismiss_at: None,
                });
                cx.notify();
            });
            cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
            let _ = this.update(cx, |app: &mut PlexiApp, cx| {
                app.notification = None;
                cx.notify();
            });
        }).detach();

        Self {
            panes,
            focused_pane: 0,
            zoomed_pane: None,
            layout,
            contexts,
            active_context: 0,
            sidebar_collapsed: false,
            overlay: None,
            palette_query: String::new(),
            palette_selected: 0,
            quick_note_text: String::new(),
            quick_note_dest: 0,
            notification: None,
            _keystroke_sub: sub,
        }
    }

    fn toggle_overlay(&mut self, overlay: ActiveOverlay, cx: &mut Context<Self>) {
        if self.overlay == Some(overlay.clone()) {
            self.overlay = None;
        } else {
            self.overlay = Some(overlay);
        }
        cx.notify();
    }

    fn dismiss_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.quick_note_text.clear();
        cx.notify();
    }

    fn add_pane(&mut self, kind: PaneKind, cx: &mut Context<Self>) {
        let id = self.panes.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        let pane = PaneState {
            id,
            name: kind.label().into(),
            kind,
            status: PaneStatus::Running,
            cwd: "~".into(),
        };
        self.panes.push(pane);
        if let Some(ctx) = self.contexts.get_mut(self.active_context) {
            ctx.pane_ids.push(id);
        }
        // Append as right split from current layout leaf
        self.layout = TileLayout::HSplit {
            ratio: 0.5,
            left: Box::new(self.layout.clone()),
            right: Box::new(TileLayout::Leaf(id)),
        };
        self.focused_pane = id;
        cx.notify();
    }

    fn split_horizontal(&mut self, cx: &mut Context<Self>) {
        let id = self.panes.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        let pane = PaneState {
            id,
            name: "zsh".into(),
            kind: PaneKind::Terminal,
            status: PaneStatus::Running,
            cwd: "~".into(),
        };
        self.panes.push(pane);
        if let Some(ctx) = self.contexts.get_mut(self.active_context) {
            ctx.pane_ids.push(id);
        }
        let current = self.layout.clone();
        self.layout = TileLayout::HSplit {
            ratio: 0.5,
            left: Box::new(current),
            right: Box::new(TileLayout::Leaf(id)),
        };
        self.focused_pane = id;
        cx.notify();
    }

    fn split_vertical(&mut self, cx: &mut Context<Self>) {
        let id = self.panes.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        let pane = PaneState {
            id,
            name: "zsh".into(),
            kind: PaneKind::Terminal,
            status: PaneStatus::Running,
            cwd: "~".into(),
        };
        self.panes.push(pane);
        if let Some(ctx) = self.contexts.get_mut(self.active_context) {
            ctx.pane_ids.push(id);
        }
        let current = self.layout.clone();
        self.layout = TileLayout::VSplit {
            ratio: 0.5,
            top: Box::new(current),
            bottom: Box::new(TileLayout::Leaf(id)),
        };
        self.focused_pane = id;
        cx.notify();
    }

    fn focus_next(&mut self, cx: &mut Context<Self>) {
        if let Some(ctx) = self.contexts.get(self.active_context) {
            let ids = &ctx.pane_ids;
            if let Some(pos) = ids.iter().position(|&id| id == self.focused_pane) {
                self.focused_pane = ids[(pos + 1) % ids.len()];
                cx.notify();
            }
        }
    }

    fn new_context(&mut self, cx: &mut Context<Self>) {
        let id = self.contexts.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let ctx = ContextInfo {
            id,
            name: format!("context-{id}"),
            cwd: "~".into(),
            pane_ids: vec![],
            children: vec![],
        };
        self.contexts.push(ctx);
        self.active_context = self.contexts.len() - 1;
        cx.notify();
    }

    fn zoom_pane(&mut self, cx: &mut Context<Self>) {
        if self.zoomed_pane == Some(self.focused_pane) {
            self.zoomed_pane = None;
        } else {
            self.zoomed_pane = Some(self.focused_pane);
        }
        cx.notify();
    }

    fn focus_prev(&mut self, cx: &mut Context<Self>) {
        if let Some(ctx) = self.contexts.get(self.active_context) {
            let ids = &ctx.pane_ids;
            if let Some(pos) = ids.iter().position(|&id| id == self.focused_pane) {
                self.focused_pane = ids[(pos + ids.len() - 1) % ids.len()];
                cx.notify();
            }
        }
    }

    fn close_pane(&mut self, cx: &mut Context<Self>) {
        let target = self.focused_pane;
        if let Some(ctx) = self.contexts.get_mut(self.active_context) {
            ctx.pane_ids.retain(|&id| id != target);
            if let Some(&next) = ctx.pane_ids.first() {
                self.focused_pane = next;
            }
        }
        self.panes.retain(|p| p.id != target);
        self.layout = self.layout.clone().remove_leaf(target);
        cx.notify();
    }

    // ─── Render helpers ───────────────────────────────────────────────────

    fn render_title_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let ctx_name = self.contexts
            .get(self.active_context)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "—".into());
        let sidebar_collapsed = self.sidebar_collapsed;

        TitleBar::new()
            .child(
                h_flex()
                    .flex_1()
                    .items_center()
                    .justify_between()
                    .pr_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                SidebarToggleButton::new()
                                    .collapsed(sidebar_collapsed)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sidebar_collapsed = !this.sidebar_collapsed;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_bold()
                                    .text_color(cx.theme().foreground)
                                    .child("Plexi"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(4.))
                                    .bg(cx.theme().accent)
                                    .text_color(cx.theme().accent_foreground)
                                    .child(ctx_name),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("cmd-palette")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::Search).size(px(14.)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_overlay(ActiveOverlay::CommandPalette, cx);
                                    })),
                            )
                            .child(
                                Button::new("context-inspector")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::PanelRight).size(px(14.)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_overlay(ActiveOverlay::ContextInspector, cx);
                                    })),
                            )
                            .child(
                                Button::new("help")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::Info).size(px(14.)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_overlay(ActiveOverlay::Help, cx);
                                    })),
                            )
                            .child(
                                Button::new("theme-toggle")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::Sun).size(px(14.)))
                                    .on_click(cx.listener(|_, _, window, cx| {
                                        let current = cx.theme().mode;
                                        let next = if current == ThemeMode::Dark {
                                            ThemeMode::Light
                                        } else {
                                            ThemeMode::Dark
                                        };
                                        Theme::change(next, Some(window), cx);
                                    })),
                            )
                            .child(
                                Button::new("settings")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::Settings).size(px(14.)))
                                    .on_click(|_, _, _| {}),
                            ),
                    ),
            )
    }

    fn render_context_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let active_ctx = self.active_context;
        h_flex()
            .h(px(34.))
            .flex_shrink_0()
            .items_center()
            .px_1()
            .gap_0p5()
            .bg(hsla(240. / 360., 0.21, 0.08, 1.0))
            .border_b_1()
            .border_color(cx.theme().border)
            .children(self.contexts.iter().enumerate().map(|(i, ctx)| {
                let is_active = i == active_ctx;
                div()
                    .id(ElementId::Name(format!("tab-{i}").into()))
                    .px_3()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .bg(if is_active { cx.theme().accent } else { cx.theme().transparent })
                    .hover(|s| s.bg(cx.theme().muted))
                    .child(
                        Icon::new(IconName::Folder)
                            .size(px(11.))
                            .text_color(if is_active { cx.theme().accent_foreground } else { cx.theme().muted_foreground }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_active { cx.theme().accent_foreground } else { cx.theme().muted_foreground })
                            .child(ctx.name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_context = i;
                        if let Some(first) = this.contexts.get(i).and_then(|c| c.pane_ids.first()).copied() {
                            this.focused_pane = first;
                        }
                        cx.notify();
                    }))
            }))
            .child(
                div()
                    .id("tab-new")
                    .px_2()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(cx.theme().muted_foreground)
                    .hover(|s| s.bg(cx.theme().muted))
                    .child(Icon::new(IconName::Plus).size(px(12.)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.new_context(cx);
                    }))
            )
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let sidebar_collapsed = self.sidebar_collapsed;
        let active_ctx = self.active_context;

        let mut menu = SidebarMenu::new();
        for (i, ctx) in self.contexts.iter().enumerate() {
            let pane_count = ctx.pane_ids.len();
            let item = SidebarMenuItem::new(ctx.name.as_str())
                .icon(IconName::Folder)
                .active(i == active_ctx)
                .suffix(move |_window, cx: &mut App| {
                    div()
                        .text_xs()
                        .px_1p5()
                        .rounded(px(10.))
                        .bg(cx.theme().muted)
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{pane_count}"))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active_context = i;
                    if let Some(first) = this.contexts.get(i).and_then(|c| c.pane_ids.first()).copied() {
                        this.focused_pane = first;
                    }
                    cx.notify();
                }));
            menu = menu.child(item);
        }

        let mut pane_menu = SidebarMenu::new();
        if let Some(ctx) = self.contexts.get(self.active_context) {
            for id in &ctx.pane_ids {
                if let Some(pane) = self.panes.iter().find(|p| p.id == *id) {
                    let icon = match pane.kind {
                        PaneKind::Terminal => IconName::SquareTerminal,
                        PaneKind::App => IconName::Bot,
                        PaneKind::Agent => IconName::Cpu,
                    };
                    let pane_id = pane.id;
                    let item = SidebarMenuItem::new(pane.name.as_str())
                        .icon(icon)
                        .active(pane.id == self.focused_pane)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.focused_pane = pane_id;
                            cx.notify();
                        }));
                    pane_menu = pane_menu.child(item);
                }
            }
        }

        Sidebar::new("plexi-sidebar")
            .collapsed(sidebar_collapsed)
            .w(px(220.))
            .header(
                SidebarHeader::new().child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_7()
                                .flex_shrink_0()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().sidebar_primary)
                                .text_color(cx.theme().sidebar_primary_foreground)
                                .when(sidebar_collapsed, |el| {
                                    el.size_4()
                                        .bg(cx.theme().transparent)
                                        .text_color(cx.theme().foreground)
                                })
                                .child(Icon::new(IconName::GalleryVerticalEnd)),
                        )
                        .when(!sidebar_collapsed, |el| {
                            el.child(
                                v_flex()
                                    .flex_1()
                                    .overflow_hidden()
                                    .child(div().text_sm().font_bold().child("Contexts"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!("{} active", self.contexts.len())),
                                    ),
                            )
                        }),
                ),
            )
            .child(SidebarGroup::new("Workspaces").child(menu))
            .when(!sidebar_collapsed, |sidebar| {
                sidebar.child(SidebarGroup::new("Panes").child(pane_menu))
            })
            .footer(
                SidebarFooter::new().child(
                    h_flex()
                        .id("sidebar-footer")
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .child(Icon::new(IconName::Plus).size(px(14.)).text_color(cx.theme().muted_foreground))
                        .when(!sidebar_collapsed, |el| {
                            el.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("New context"),
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.new_context(cx);
                        })),
                ),
            )
    }

    fn render_layout_node(&self, layout: &TileLayout, cx: &Context<Self>) -> AnyElement {
        match layout {
            TileLayout::Leaf(id) => {
                self.render_single_pane(*id, cx).into_any_element()
            }
            TileLayout::HSplit { left, right, .. } => {
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap_1()
                    .child(
                        div().flex_1().min_h_0()
                            .child(self.render_layout_node(left, cx)),
                    )
                    .child(
                        div().flex_1().min_h_0()
                            .child(self.render_layout_node(right, cx)),
                    )
                    .into_any_element()
            }
            TileLayout::VSplit { top, bottom, .. } => {
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap_1()
                    .child(
                        div().flex_1().min_w_0()
                            .child(self.render_layout_node(top, cx)),
                    )
                    .child(
                        div().flex_1().min_w_0()
                            .child(self.render_layout_node(bottom, cx)),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_single_pane(&self, id: usize, cx: &Context<Self>) -> impl IntoElement {
        let pane = self.panes.iter().find(|p| p.id == id);
        let focused = id == self.focused_pane;

        let border_color = if focused { cx.theme().accent } else { cx.theme().border };

        let status_color = pane.map(|p| match p.status {
            PaneStatus::Running | PaneStatus::Busy => cx.theme().success,
            PaneStatus::Error => cx.theme().danger,
            _ => cx.theme().muted_foreground,
        }).unwrap_or(cx.theme().muted_foreground);

        let kind_label = pane.map(|p| p.kind.label()).unwrap_or("term");
        let pane_name = pane.map(|p| p.name.clone()).unwrap_or_else(|| "—".into());
        let kind_icon = pane.map(|p| match p.kind {
            PaneKind::Terminal => IconName::SquareTerminal,
            PaneKind::App => IconName::Bot,
            PaneKind::Agent => IconName::Cpu,
        }).unwrap_or(IconName::SquareTerminal);

        div()
            .id(ElementId::Name(format!("pane-{id}").into()))
            .flex_1()
            .min_h(px(60.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(border_color)
            .bg(hsla(240. / 360., 0.21, 0.12, 1.0))
            .overflow_hidden()
            .flex()
            .flex_col()
            // Pane title bar
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .flex_shrink_0()
                    .bg(hsla(240. / 360., 0.21, 0.09, 1.0))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .px_1()
                            .rounded_sm()
                            .bg(cx.theme().muted)
                            .text_color(cx.theme().muted_foreground)
                            .font_family("JetBrains Mono")
                            .child(kind_label),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .gap_1()
                            .items_center()
                            .min_w_0()
                            .child(Icon::new(kind_icon).size(px(12.)).text_color(status_color))
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .text_color(if focused { cx.theme().foreground } else { cx.theme().muted_foreground })
                                    .child(pane_name),
                            ),
                    )
                    .child(div().size(px(6.)).rounded_full().bg(status_color))
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .id(ElementId::Name(format!("zoom-{id}").into()))
                                    .size(px(16.))
                                    .flex().items_center().justify_center()
                                    .rounded_sm().cursor_pointer().text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .hover(|s| s.bg(cx.theme().muted))
                                    .child("⤢")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.zoom_pane(cx);
                                    }))
                            )
                            .child(
                                div()
                                    .id(ElementId::Name(format!("close-{id}").into()))
                                    .size(px(16.))
                                    .flex().items_center().justify_center()
                                    .rounded_sm().cursor_pointer().text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .hover(|s| s.bg(cx.theme().muted))
                                    .child("×")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.focused_pane = id;
                                        this.close_pane(cx);
                                    }))
                            ),
                    ),
            )
            // Terminal content — placeholder until PTY renderer is wired
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(hsla(240. / 360., 0.25, 0.07, 1.0))
                    .p_2()
                    .child(terminal_placeholder(id, &**cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.focused_pane = id;
                cx.notify();
            }))
            .context_menu(move |menu, _window, _cx| {
                menu.item(PopupMenuItem::new("Split Horizontal").action(Box::new(SplitHorizontal)))
                    .item(PopupMenuItem::new("Split Vertical").action(Box::new(SplitVertical)))
                    .separator()
                    .item(PopupMenuItem::new("Zoom").action(Box::new(ZoomPane)))
                    .separator()
                    .item(PopupMenuItem::new("Close Pane").action(Box::new(ClosePane)))
            })
    }

    fn render_status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let ctx = self.contexts.get(self.active_context);
        let pane_count = ctx.map(|c| c.pane_ids.len()).unwrap_or(0);
        let ctx_name = ctx.map(|c| c.name.clone()).unwrap_or_else(|| "—".into());

        h_flex()
            .px_3()
            .h(px(26.))
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .bg(hsla(240. / 360., 0.21, 0.07, 1.0))
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_3()
                    .child(status_item(IconName::LayoutDashboard, "1/1", &**cx))
                    .child(status_item(IconName::SquareTerminal, format!("{pane_count} panes"), &**cx))
                    .child(status_item(IconName::Folder, ctx_name, &**cx)),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(status_item(IconName::Github, "alpha", &**cx))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("v0.0.505")),
            )
    }

    fn render_notification(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let notif = self.notification.as_ref()?;
        let title = notif.title.clone();
        let body = notif.body.clone();
        Some(
            div()
                .id("notification-toast")
                .absolute()
                .bottom(px(36.))
                .right(px(16.))
                .w(px(300.))
                .rounded(px(8.))
                .shadow_xl()
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .items_start()
                        .child(
                            div()
                                .size(px(7.))
                                .mt(px(4.))
                                .rounded_full()
                                .bg(cx.theme().success)
                                .flex_shrink_0(),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .gap_0p5()
                                .child(div().text_sm().font_bold().text_color(cx.theme().foreground).child(title))
                                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(body)),
                        )
                        .child(
                            div()
                                .id("notif-dismiss")
                                .text_xs()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(3.))
                                .bg(cx.theme().muted)
                                .text_color(cx.theme().muted_foreground)
                                .cursor_pointer()
                                .child("✕")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.notification = None;
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }
}

// ─── Terminal placeholder ─────────────────────────────────────────────────────

fn terminal_placeholder(id: usize, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .gap_1()
                .child(div().text_xs().font_family("JetBrains Mono").text_color(cx.theme().success).child("ian@mac"))
                .child(div().text_xs().font_family("JetBrains Mono").text_color(cx.theme().muted_foreground).child(":"))
                .child(div().text_xs().font_family("JetBrains Mono").text_color(cx.theme().accent).child("~/PLEXI"))
                .child(div().text_xs().font_family("JetBrains Mono").text_color(cx.theme().muted_foreground).child(" $")),
        )
        .child(
            div().text_xs().font_family("JetBrains Mono").text_color(cx.theme().foreground)
                .child(match id { 1 => "> What would you like to work on?", 2 => "cargo build --release", 3 => "git log --oneline", _ => "ls -la" }),
        )
        .child(
            div().text_xs().font_family("JetBrains Mono").text_color(cx.theme().muted_foreground)
                .child("  ·· pty renderer wired here ··"),
        )
}

fn pane_btn(label: &'static str, cx: &App) -> impl IntoElement {
    div()
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .cursor_pointer()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .hover(|s| s.bg(cx.theme().muted))
        .child(label)
}

fn status_item(icon: IconName, label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(Icon::new(icon).size(px(11.)).text_color(cx.theme().muted_foreground))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.into()))
}

// ─── Render ───────────────────────────────────────────────────────────────────

impl Render for PlexiApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = self.overlay.clone();
        let layout = self.layout.clone();
        let zoomed = self.zoomed_pane;

        v_flex()
            .size_full()
            .relative()
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(zoomed.is_none(), |el| el.child(self.render_sidebar(cx)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            // Context tab bar
                            .when(zoomed.is_none(), |el| el.child(self.render_context_tabs(cx)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .p_1()
                                    .child(if let Some(zid) = zoomed {
                                        self.render_single_pane(zid, cx).into_any_element()
                                    } else {
                                        self.render_layout_node(&layout, cx)
                                    }),
                            ),
                    ),
            )
            .child(self.render_status_bar(cx))
            // Overlays — painted last so they appear on top
            .when(overlay == Some(ActiveOverlay::CommandPalette), |el| {
                el.child(render_command_palette(&self.palette_query, self.palette_selected, &**cx))
            })
            .when(overlay == Some(ActiveOverlay::ContextInspector), |el| {
                el.child(render_context_inspector(
                    &self.contexts,
                    &self.panes,
                    self.active_context,
                    &**cx,
                ))
            })
            .when(overlay == Some(ActiveOverlay::Help), |el| {
                el.child(render_help(&**cx))
            })
            .when(overlay == Some(ActiveOverlay::QuickNote), |el| {
                el.child(render_quick_note(&self.quick_note_text, self.quick_note_dest, &**cx))
            })
            .when_some(self.render_notification(cx), |el, n| el.child(n))
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    let app = gpui_platform::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-p", ToggleCommandPalette, None),
            KeyBinding::new("cmd-backslash", ToggleSidebar, None),
            KeyBinding::new("cmd-i", ToggleContextInspector, None),
            KeyBinding::new("cmd-/", ToggleHelp, None),
            KeyBinding::new("cmd-0", ToggleQuickNote, None),
            KeyBinding::new("escape", DismissOverlay, None),
            KeyBinding::new("cmd-t", NewTerminal, None),
            KeyBinding::new("cmd-w", ClosePane, None),
            KeyBinding::new("cmd--", SplitHorizontal, None),
            KeyBinding::new("cmd-shift-=", SplitVertical, None),
            KeyBinding::new("cmd-]", FocusNext, None),
            KeyBinding::new("cmd-[", FocusPrev, None),
            KeyBinding::new("cmd-n", NewContext, None),
            KeyBinding::new("cmd-z", ZoomPane, None),
        ]);

        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let window_options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                window.activate_window();
                window.set_window_title("Plexi");

                Theme::change(ThemeMode::Dark, Some(window), cx);

                let view = cx.new(|cx| PlexiApp::new(cx));
                let v = view.clone();

                cx.on_action::<ToggleCommandPalette>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.toggle_overlay(ActiveOverlay::CommandPalette, cx)); }
                });
                cx.on_action::<ToggleSidebar>({
                    let v = v.clone();
                    move |_, cx| {
                        v.update(cx, |a, cx| {
                            a.sidebar_collapsed = !a.sidebar_collapsed;
                            cx.notify();
                        });
                    }
                });
                cx.on_action::<DismissOverlay>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.dismiss_overlay(cx)); }
                });
                cx.on_action::<ToggleContextInspector>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.toggle_overlay(ActiveOverlay::ContextInspector, cx)); }
                });
                cx.on_action::<ToggleHelp>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.toggle_overlay(ActiveOverlay::Help, cx)); }
                });
                cx.on_action::<ToggleQuickNote>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.toggle_overlay(ActiveOverlay::QuickNote, cx)); }
                });
                cx.on_action::<NewTerminal>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.add_pane(PaneKind::Terminal, cx)); }
                });
                cx.on_action::<ClosePane>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.close_pane(cx)); }
                });
                cx.on_action::<SplitHorizontal>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.split_horizontal(cx)); }
                });
                cx.on_action::<SplitVertical>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.split_vertical(cx)); }
                });
                cx.on_action::<FocusNext>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.focus_next(cx)); }
                });
                cx.on_action::<FocusPrev>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.focus_prev(cx)); }
                });
                cx.on_action::<NewContext>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.new_context(cx)); }
                });
                cx.on_action::<ZoomPane>({
                    let v = v.clone();
                    move |_, cx| { v.update(cx, |a, cx| a.zoom_pane(cx)); }
                });

                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
