#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod theme;

use egui::{
    Align, Align2, CentralPanel, Color32, FontId, Layout, Rect, RichText, CornerRadius, SidePanel,
    Stroke, StrokeKind, TopBottomPanel, Vec2,
};
use theme::Colors;

const R2: CornerRadius = CornerRadius::same(2);
const R3: CornerRadius = CornerRadius::same(3);
const R6: CornerRadius = CornerRadius::same(6);

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([400.0, 300.0])
            .with_title("Plexi Mockup"),
        ..Default::default()
    };

    eframe::run_native(
        "plexi-mockup",
        native_options,
        Box::new(|cc| {
            theme::setup_fonts(&cc.egui_ctx);
            theme::setup_style(&cc.egui_ctx);
            Ok(Box::new(MockupApp::default()))
        }),
    )
}

struct Context {
    name: &'static str,
    path: &'static str,
    panes: usize,
}

struct MockupApp {
    active_context: usize,
    sidebar_visible: bool,
    show_shortcuts: bool,
    contexts: Vec<Context>,
}

impl Default for MockupApp {
    fn default() -> Self {
        Self {
            active_context: 2,
            sidebar_visible: true,
            show_shortcuts: false,
            contexts: vec![
                Context {
                    name: "sandbox",
                    path: "~/sandbox",
                    panes: 1,
                },
                Context {
                    name: "nrtx",
                    path: "~/projects/nrtx",
                    panes: 3,
                },
                Context {
                    name: "Plexi",
                    path: "~/Documents/GitHub/plexi",
                    panes: 2,
                },
                Context {
                    name: "meta",
                    path: "~/dotfiles",
                    panes: 1,
                },
            ],
        }
    }
}

impl eframe::App for MockupApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Consume keyboard shortcuts
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::COMMAND, egui::Key::B) {
                self.sidebar_visible = !self.sidebar_visible;
            }
            if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Slash) {
                self.show_shortcuts = !self.show_shortcuts;
            }
        });

        // Toolbar
        TopBottomPanel::top("toolbar")
            .exact_height(28.0)
            .frame(egui::Frame::new().fill(Colors::BG_TOOLBAR).inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 4,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let ctx_info = &self.contexts[self.active_context];

                    // Sidebar toggle
                    let toggle_text = if self.sidebar_visible { "◀" } else { "▶" };
                    if ui
                        .add(egui::Button::new(
                            RichText::new(toggle_text).size(11.0).color(Colors::TEXT_DIM),
                        ).frame(false))
                        .on_hover_text("Toggle sidebar (⌘B)")
                        .clicked()
                    {
                        self.sidebar_visible = !self.sidebar_visible;
                    }

                    ui.add_space(8.0);

                    // Context info
                    ui.label(
                        RichText::new(ctx_info.name)
                            .size(12.0)
                            .color(Colors::TEXT_PRIMARY)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(ctx_info.path)
                            .size(11.0)
                            .color(Colors::TEXT_DIM)
                            .family(egui::FontFamily::Monospace),
                    );
                    ui.label(
                        RichText::new(format!("{} pane{}", ctx_info.panes, if ctx_info.panes == 1 { "" } else { "s" }))
                            .size(11.0)
                            .color(Colors::TEXT_SECTION),
                    );

                    // Right side
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(
                                RichText::new("?").size(12.0).color(Colors::TEXT_DIM),
                            ).frame(false))
                            .on_hover_text("Keyboard shortcuts (⇧/)")
                            .clicked()
                        {
                            self.show_shortcuts = !self.show_shortcuts;
                        }
                    });
                });
            });

        // Separator line under toolbar
        TopBottomPanel::top("toolbar_sep")
            .exact_height(1.0)
            .frame(egui::Frame::new().fill(Colors::BORDER))
            .show(ctx, |_ui| {});

        // Sidebar
        if self.sidebar_visible {
            SidePanel::left("sidebar")
                .exact_width(220.0)
                .frame(
                    egui::Frame::new()
                        .fill(Colors::BG_SIDEBAR)
                        .inner_margin(egui::Margin::same(0)),
                )
                .show(ctx, |ui| {
                    self.draw_sidebar(ui);
                });
        }

        // Central panel
        CentralPanel::default()
            .frame(egui::Frame::new().fill(Colors::BG_DARKEST).inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                self.draw_central(ui);
            });

        // Keyboard shortcuts overlay
        if self.show_shortcuts {
            self.draw_shortcuts_overlay(ctx);
        }
    }
}

impl MockupApp {
    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let sidebar_width = ui.available_width();

        // Branding
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("PLEXI")
                    .size(16.0)
                    .color(Colors::TEXT_PRIMARY)
                    .strong(),
            );
        });
        ui.add_space(12.0);

        // Divider
        let rect = ui.cursor();
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + sidebar_width, rect.min.y),
            ],
            Stroke::new(1.0, Colors::BORDER),
        );
        ui.add_space(4.0);

        // Contexts section
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("Contexts")
                    .size(10.0)
                    .color(Colors::TEXT_SECTION),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if ui
                    .add(
                        egui::Button::new(RichText::new("+").size(12.0).color(Colors::TEXT_DIM))
                            .frame(false),
                    )
                    .on_hover_text("New context")
                    .clicked()
                {
                    // placeholder
                }
            });
        });
        ui.add_space(4.0);

        // Context list
        let active = self.active_context;
        for (i, context) in self.contexts.iter().enumerate() {
            let is_active = i == active;

            let bg = if is_active {
                Colors::BG_ACTIVE
            } else {
                Color32::TRANSPARENT
            };

            let response = ui.allocate_ui_with_layout(
                Vec2::new(sidebar_width, 26.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    let rect = ui.max_rect();
                    let hover = ui.rect_contains_pointer(rect);

                    let fill = if is_active {
                        bg
                    } else if hover {
                        Colors::BG_HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);

                    if is_active {
                        ui.painter().rect_filled(
                            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                            CornerRadius::ZERO,
                            Colors::ACCENT,
                        );
                    }

                    ui.add_space(20.0);
                    let text_color = if is_active {
                        Colors::TEXT_PRIMARY
                    } else {
                        Colors::TEXT_DIM
                    };
                    ui.label(RichText::new(context.name).size(12.0).color(text_color));
                },
            );

            if response.response.clicked() {
                self.active_context = i;
            }
        }

        ui.add_space(16.0);

        // Map section divider
        let rect = ui.cursor();
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + sidebar_width, rect.min.y),
            ],
            Stroke::new(1.0, Colors::BORDER),
        );
        ui.add_space(4.0);

        // Map section
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let panes = self.contexts[self.active_context].panes;
            ui.label(
                RichText::new("Map")
                    .size(10.0)
                    .color(Colors::TEXT_SECTION),
            );
            ui.label(
                RichText::new(format!("{} node{}", panes, if panes == 1 { "" } else { "s" }))
                    .size(10.0)
                    .color(Colors::TEXT_SECTION),
            );
        });
        ui.add_space(8.0);

        // Minimap
        self.draw_minimap(ui, sidebar_width);
    }

    fn draw_minimap(&self, ui: &mut egui::Ui, sidebar_width: f32) {
        let panes = self.contexts[self.active_context].panes;
        let map_width = sidebar_width - 32.0;
        let map_height = 60.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(sidebar_width, map_height), egui::Sense::hover());

        let map_rect = Rect::from_min_size(
            egui::pos2(rect.min.x + 16.0, rect.min.y),
            Vec2::new(map_width, map_height),
        );

        // Background
        ui.painter().rect(
            map_rect,
            R3,
            Colors::BG_DARKEST,
            Stroke::new(1.0, Colors::MINIMAP_BORDER),
            StrokeKind::Inside,
        );

        let gap = 3.0;
        let inner = map_rect.shrink(4.0);

        match panes {
            1 => {
                ui.painter().rect_filled(inner, R2, Colors::MINIMAP_PANE);
            }
            2 => {
                let half = (inner.width() - gap) / 2.0;
                let left = Rect::from_min_size(inner.min, Vec2::new(half, inner.height()));
                let right = Rect::from_min_size(
                    egui::pos2(inner.min.x + half + gap, inner.min.y),
                    Vec2::new(half, inner.height()),
                );
                ui.painter().rect_filled(left, R2, Colors::MINIMAP_PANE);
                ui.painter().rect_filled(right, R2, Colors::MINIMAP_PANE);
                // Highlight first pane as focused
                ui.painter().rect_stroke(left, R2, Stroke::new(1.0, Colors::ACCENT_DIM), StrokeKind::Inside);
            }
            3 => {
                let half_w = (inner.width() - gap) / 2.0;
                let half_h = (inner.height() - gap) / 2.0;
                let left = Rect::from_min_size(inner.min, Vec2::new(half_w, inner.height()));
                let top_right = Rect::from_min_size(
                    egui::pos2(inner.min.x + half_w + gap, inner.min.y),
                    Vec2::new(half_w, half_h),
                );
                let bot_right = Rect::from_min_size(
                    egui::pos2(inner.min.x + half_w + gap, inner.min.y + half_h + gap),
                    Vec2::new(half_w, half_h),
                );
                ui.painter().rect_filled(left, R2, Colors::MINIMAP_PANE);
                ui.painter().rect_filled(top_right, R2, Colors::MINIMAP_PANE);
                ui.painter().rect_filled(bot_right, R2, Colors::MINIMAP_PANE);
                ui.painter().rect_stroke(left, R2, Stroke::new(1.0, Colors::ACCENT_DIM), StrokeKind::Inside);
            }
            _ => {
                ui.painter().rect_filled(inner, R2, Colors::MINIMAP_PANE);
            }
        }
    }

    fn draw_central(&self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, Colors::BG_DARKEST);

        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No terminals open\n\n⌘N to create a pane  •  ⌘D to split",
            FontId::proportional(13.0),
            Colors::TEXT_SECTION,
        );
    }

    fn draw_shortcuts_overlay(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("shortcuts_overlay"))
            .anchor(Align2::RIGHT_TOP, Vec2::new(-16.0, 44.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Colors::BG_SIDEBAR)
                    .stroke(Stroke::new(1.0, Colors::BORDER))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(
                            RichText::new("Keyboard Shortcuts")
                                .size(13.0)
                                .color(Colors::TEXT_PRIMARY)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        let shortcuts = [
                            ("⌘N", "New pane"),
                            ("⌘D", "Split right"),
                            ("⌘⇧D", "Split down"),
                            ("⌘W", "Close pane"),
                            ("⌘B", "Toggle sidebar"),
                            ("⌘←/→", "Focus pane"),
                            ("⌘⇧←/→", "Resize pane"),
                            ("⇧/", "This help"),
                        ];

                        for (key, desc) in shortcuts {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(key)
                                        .size(11.0)
                                        .color(Colors::ACCENT)
                                        .family(egui::FontFamily::Monospace),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(desc).size(11.0).color(Colors::TEXT_DIM),
                                );
                            });
                        }
                    });
            });
    }
}
