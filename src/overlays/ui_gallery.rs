use super::*;
use crate::ui::{
    button::{ButtonKind, choice_button, chrome_button, icon_button, toolbar_button},
    code_viewer::ReadOnlyCodeViewer,
    hints::{HintBar, HintGroup},
    labels::chrome_section,
    list::{ListRow, ListRowPips},
    overlay::ModalShell,
    shortcuts::key_chip,
    surface::{
        TrustTone, color_swatch, compact_copyable_command, copyable_command, empty_state_panel,
        status_chip, trust_decision_panel,
    },
    text_field::{TextArea, TextField},
    toast::{ProgressDots, toast_caption},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum UiGalleryDataMode {
    #[default]
    Demo,
    WorstCase,
    Empty,
    SingleItem,
}

impl UiGalleryDataMode {
    pub(crate) const ALL: [Self; 4] = [Self::Demo, Self::WorstCase, Self::Empty, Self::SingleItem];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Demo => "Demo",
            Self::WorstCase => "Worst case",
            Self::Empty => "Empty",
            Self::SingleItem => "1 item",
        }
    }

    pub(crate) fn fixture(self) -> GalleryFixture {
        match self {
            Self::Demo => GalleryFixture {
                names: &["Normal row", "Selected row", "Danger row"],
                detail: "Secondary metadata",
                command: "plexi completions zsh",
                code: "fn main() {\n    println!(\"hello from Plexi\");\n}\n",
                tags: &["app", "ws"],
                count: 3,
                pips: 3,
                status: "running",
                empty: "No matching items",
            },
            Self::WorstCase => GalleryFixture {
                // Curated from emilkowalski/skills/skills/break-ui/CATALOG.md.
                names: &[
                    "Aleksandra Wiśniewska-Kowalczyk",
                    "Đặng Thị Ngọc Hân",
                    "王秀英",
                    "نور الهدى عبد الرحمن",
                    "👩🏽‍💻 Priya",
                    "J",
                    "Line one\nLine two",
                    "Q3 Board Deck — FINAL (revised) v12 [approved by legal].pdf",
                ],
                detail: "bartholomew.fitzgerald@northwind-industries-holdings.example.com",
                command: "https://example.com/workspaces/acme/projects/q3-launch/docs/9f8e7d6c5b4a?tab=comments&filter=unresolved",
                code: "/workspaces/northwind-industries-holdings/quarterly-board-materials/Q3 Board Deck — FINAL (revised) v12 [approved by legal].pdf\nLine one\nLine two",
                tags: &[
                    "design",
                    "frontend",
                    "q3",
                    "urgent",
                    "needs-review",
                    "legal",
                    "approved",
                    "external",
                    "shared",
                    "draft",
                    "finance",
                    "customer-feedback-from-enterprise-onboarding",
                ],
                count: 1284,
                pips: 20,
                status: "Invitation expired 12 days ago",
                empty: "No matching items for Aleksandra Wiśniewska-Kowalczyk",
            },
            Self::Empty => GalleryFixture {
                names: &[],
                detail: "",
                command: "",
                code: "",
                tags: &[],
                count: 0,
                pips: 0,
                status: "empty",
                empty: "No matching items",
            },
            Self::SingleItem => GalleryFixture {
                names: &["J"],
                detail: "One item",
                command: "plexi --help",
                code: "J",
                tags: &["app"],
                count: 1,
                pips: 1,
                status: "running",
                empty: "1 item",
            },
        }
    }
}

pub(crate) struct GalleryFixture {
    pub(crate) names: &'static [&'static str],
    pub(crate) detail: &'static str,
    pub(crate) command: &'static str,
    code: &'static str,
    tags: &'static [&'static str],
    count: usize,
    pips: usize,
    status: &'static str,
    empty: &'static str,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct UiGalleryPreview {
    pub(crate) mode: UiGalleryDataMode,
    pub(crate) narrow: bool,
}

impl UiGalleryPreview {
    pub(crate) fn id() -> egui::Id {
        egui::Id::new("ui_gallery_preview")
    }
}

/// Identical fixture boundary for the interactive gallery and geometry tests.
pub(crate) fn gallery_rows(
    ui: &mut egui::Ui,
    data: &GalleryFixture,
    colors: &crate::ui::theme::Colors,
) {
    for (index, name) in data.names.iter().enumerate() {
        ListRow::new(name)
            .secondary(data.detail)
            .chip("app")
            .metadata_chips(data.tags)
            .pane_pips(ListRowPips {
                count: data.pips,
                focused_idx: Some(0),
                hidden_indices: vec![],
                activities: vec![],
            })
            .trailing_action(if index == 2 { "Delete" } else { "Open" })
            .selected(index == 1)
            .danger_trailing(index == 2)
            .show(ui, colors);
    }
    for name in data.names {
        ListRow::new(name)
            .dense()
            .chip("app")
            .trailing_action("Open")
            .show(ui, colors);
    }
}

impl PlexiApp {
    pub(crate) fn draw_ui_gallery(&mut self, ctx: &egui::Context) {
        if !self.show_ui_gallery {
            return;
        }

        let colors = self.colors;
        let mut preview = ctx.data_mut(|d| {
            d.get_temp::<UiGalleryPreview>(UiGalleryPreview::id())
                .unwrap_or_default()
        });
        // Escape is gated off while the nested text-entry demo is open so the
        // nested modal consumes it first — otherwise one press closes both.
        let response = ModalShell::centered("host_ui_gallery")
            .title("Host UI Gallery")
            .width(style::MODAL_WIDTH_NOTIFY)
            .escape(!self.ui_gallery_show_text_modal)
            .show(ctx, &colors, |ui| {
                ui.label("Chrome primitives");
                let previous = preview.mode;
                ui.horizontal_wrapped(|ui| {
                    for mode in UiGalleryDataMode::ALL {
                        ui.selectable_value(&mut preview.mode, mode, mode.label());
                    }
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut preview.narrow, false, "Full width");
                    ui.selectable_value(&mut preview.narrow, true, "Narrow 320px");
                });
                if previous != preview.mode {
                    log::info!("ui_gallery: data_mode={:?}", preview.mode);
                    self.ui_gallery_normal_buf = preview.mode.fixture().detail.to_owned();
                    self.ui_gallery_focused_buf = preview.mode.fixture().command.to_owned();
                }
                let data = preview.mode.fixture();
                ui.add_space(style::SPACE_MD);

                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 180.0).max(280.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // available_width, not MODAL_WIDTH_NOTIFY — the scroll
                        // area reserves a scrollbar gutter; forcing the full
                        // modal width would push content back under the bar.
                        ui.set_width(if preview.narrow {
                            ui.available_width().min(style::GALLERY_NARROW_W)
                        } else {
                            ui.available_width()
                        });
                        chrome_section(ui, "Modal shell", &colors, |ui| {
                            token_strip(ui, &colors);
                            hint_bar(ui, &colors);
                        });

                        chrome_section(ui, "Code and toast surfaces", &colors, |ui| {
                            ReadOnlyCodeViewer::new(
                                egui::Id::new("host_ui_gallery_code_viewer"),
                                data.code,
                                "rs",
                            )
                            .line_numbers(true)
                            .max_height(96.0)
                            .show(ui, &colors);
                            ui.add_space(style::SPACE_SM);
                            copyable_command(
                                ui,
                                egui::Id::new("host_ui_gallery_copyable_command"),
                                data.command,
                                &colors,
                            );
                            ui.add_space(style::SPACE_SM);
                            ui.vertical(|ui| {
                                toast_caption(ui, "Shell completions aren't set up.", &colors);
                                compact_copyable_command(
                                    ui,
                                    egui::Id::new("host_ui_gallery_compact_command"),
                                    data.command,
                                    &colors,
                                );
                                ProgressDots::new(2, 3).show(ui, &colors);
                            });
                        });

                        chrome_section(ui, "Trust and confirmations", &colors, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                chrome_button(ui, "Install", ButtonKind::Primary, &colors, 88.0);
                                chrome_button(ui, "Cancel", ButtonKind::Secondary, &colors, 88.0);
                                chrome_button(
                                    ui,
                                    "Deny forever",
                                    ButtonKind::Danger,
                                    &colors,
                                    116.0,
                                );
                            });
                            let hints = [
                                HintGroup::new(&["Enter"], "confirm"),
                                HintGroup::new(&["Esc"], "cancel"),
                                HintGroup::new(&["Shift", "Esc"], "deny forever"),
                            ];
                            HintBar::new(&hints).show(ui, &colors);
                            trust_decision_panel(
                                ui,
                                data.names.first().copied().unwrap_or(""),
                                data.detail,
                                TrustTone::Neutral,
                                &colors,
                            );
                            ui.add_space(style::SPACE_SM);
                            trust_decision_panel(
                                ui,
                                data.names.first().copied().unwrap_or(""),
                                data.detail,
                                TrustTone::Warning,
                                &colors,
                            );
                            ui.add_space(style::SPACE_SM);
                            trust_decision_panel(
                                ui,
                                data.names.first().copied().unwrap_or(""),
                                data.detail,
                                TrustTone::Danger,
                                &colors,
                            );
                        });

                        chrome_section(ui, "Rows", &colors, |ui| {
                            gallery_rows(ui, &data, &colors);
                        });
                        chrome_section(ui, "Sidebar rows", &colors, |ui| {
                            for (index, name) in data.names.iter().enumerate() {
                                crate::ui::sidebar_row::SidebarRow {
                                    is_active: index == 1,
                                    is_dragging: false,
                                    any_dragging: false,
                                    action_enabled: true,
                                    ctx_name: (*name).to_owned(),
                                    ctx_index: Some(index),
                                    badge_count: data.count,
                                    subtitle: Some(data.command.to_owned()),
                                    draggable: false,
                                    pane_dots: Some(crate::ui::sidebar_row::PaneDots {
                                        count: data.pips,
                                        focused_idx: Some(0),
                                        hidden_set: Default::default(),
                                        activities: vec![],
                                        windows: vec![],
                                    }),
                                }
                                .show(
                                    ui,
                                    egui::Id::new(("gallery_sidebar", index)),
                                    &colors,
                                );
                            }
                        });

                        chrome_section(ui, "Text fields", &colors, |ui| {
                            TextField::singleline(
                                egui::Id::new("host_ui_gallery_text_normal"),
                                "Normal text field",
                            )
                            .show(
                                ui,
                                &mut self.ui_gallery_normal_buf,
                                &colors,
                            );
                            ui.add_space(style::SPACE_SM);
                            // No `.focused(true)` here — a hardcoded focus
                            // demo fights the user's clicks (two fields
                            // re-stealing focus from each other every frame).
                            // Focus follows clicks; the focused style shows on
                            // whichever field the user activates.
                            TextField::singleline(
                                egui::Id::new("host_ui_gallery_text_focused"),
                                "Click to focus — accent ring + cursor",
                            )
                            .log_name("ui_gallery")
                            .show(
                                ui,
                                &mut self.ui_gallery_focused_buf,
                                &colors,
                            );
                            ui.add_space(style::SPACE_SM);
                            let mut multiline_demo = data.code.to_owned();
                            TextArea::multiline(
                                egui::Id::new("host_ui_gallery_text_area"),
                                "Multiline text area",
                            )
                            .rows(3)
                            .log_name("ui_gallery")
                            .show(ui, &mut multiline_demo, &colors);
                        });

                        chrome_section(ui, "Modal patterns", &colors, |ui| {
                            if chrome_button(
                                ui,
                                "Open text-entry modal",
                                ButtonKind::Primary,
                                &colors,
                                180.0,
                            )
                            .clicked()
                            {
                                self.ui_gallery_show_text_modal = true;
                                log::info!("ui_gallery: opened text-entry modal demo");
                            }
                        });

                        chrome_section(ui, "Buttons and chips", &colors, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                chrome_button(
                                    ui,
                                    data.names.first().copied().unwrap_or(""),
                                    ButtonKind::Primary,
                                    &colors,
                                    80.0,
                                );
                                chrome_button(
                                    ui,
                                    "Secondary",
                                    ButtonKind::Secondary,
                                    &colors,
                                    92.0,
                                );
                                chrome_button(ui, "Danger", ButtonKind::Danger, &colors, 80.0);
                                chrome_button(ui, "Accent", ButtonKind::Accent, &colors, 80.0);
                                ui.add_enabled_ui(false, |ui| {
                                    chrome_button(
                                        ui,
                                        "Disabled",
                                        ButtonKind::Secondary,
                                        &colors,
                                        80.0,
                                    );
                                });
                            });
                            choice_button(ui, data.detail, "⌘Enter", false, &colors);
                            ui.add_space(style::SPACE_SM);
                            ui.horizontal(|ui| {
                                icon_button(ui, "+", "Icon button", &colors);
                                toolbar_button(
                                    ui,
                                    RichText::new("Toolbar")
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.text_dim),
                                    "Toolbar button",
                                );
                            });
                            ui.add_space(style::SPACE_SM);
                            ui.horizontal_wrapped(|ui| {
                                key_chip(
                                    ui,
                                    "app",
                                    &colors,
                                    egui::FontId::monospace(style::TEXT_HINT),
                                );
                                key_chip(
                                    ui,
                                    "term",
                                    &colors,
                                    egui::FontId::monospace(style::TEXT_HINT),
                                );
                                status_chip(ui, data.status, &colors);
                                crate::ui::badge::badge(
                                    ui,
                                    &crate::ui::text::format_count_plural(
                                        data.count, "item", "items",
                                    ),
                                    colors.bg_active,
                                    &colors,
                                );
                                status_chip(ui, "error", &colors);
                                status_chip(ui, "empty", &colors);
                            });
                        });

                        chrome_section(ui, "Empty states", &colors, |ui| {
                            empty_state_panel(ui, data.empty, Some(data.detail), &colors);
                        });
                    });
            });

        ctx.data_mut(|d| d.insert_temp(UiGalleryPreview::id(), preview));

        // Ignore gallery click-away while the nested demo is open — a click
        // inside the nested modal must not tear down the gallery under it.
        if response.dismissed && !self.ui_gallery_show_text_modal {
            self.show_ui_gallery = false;
            log::info!("ui_gallery: closed by escape or click-away");
        }

        if self.ui_gallery_show_text_modal {
            let entry = ModalShell::centered("host_ui_gallery_text_modal")
                .title("Text entry")
                .width(style::MODAL_WIDTH_MD)
                .escape(true)
                .show(ctx, &colors, |ui| {
                    TextField::singleline(
                        egui::Id::new("host_ui_gallery_modal_field"),
                        "Type something...",
                    )
                    .log_name("ui_gallery_modal")
                    .show(ui, &mut self.ui_gallery_modal_buf, &colors);
                    let hints = [
                        HintGroup::new(&["\u{23ce}"], "save"),
                        HintGroup::new(&["esc"], "dismiss"),
                    ];
                    HintBar::new(&hints).show(ui, &colors);
                });
            let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            if entry.dismissed || enter {
                self.ui_gallery_show_text_modal = false;
                log::info!("ui_gallery: text-entry modal demo closed");
            }
        }
    }
}

fn token_strip(ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
    ui.horizontal_wrapped(|ui| {
        color_swatch(ui, "toolbar", colors.bg_toolbar, colors);
        color_swatch(ui, "active", colors.bg_active, colors);
        color_swatch(ui, "accent", colors.accent, colors);
        color_swatch(ui, "danger", colors.danger, colors);
        color_swatch(ui, "border", colors.border, colors);
    });
}

fn hint_bar(ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
    // The real modal-footer treatment: each label attached to its combo,
    // centered, divider above.
    let hints = [
        HintGroup::new(&["\u{2318}", "P"], "palette"),
        HintGroup::new(&["\u{2318}", "/"], "help"),
        HintGroup::new(&["esc"], "dismiss"),
    ];
    HintBar::new(&hints).show(ui, colors);
}
