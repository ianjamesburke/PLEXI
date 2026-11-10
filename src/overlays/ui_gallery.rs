use super::*;
use crate::ui::{
    button::{choice_button, icon_button, toolbar_button, ButtonKind, KitButton},
    code_viewer::ReadOnlyCodeViewer,
    hints::{HintBar, HintGroup},
    labels::chrome_section,
    list::{ListRow, ListRowPips},
    overlay::ModalShell,
    shortcuts::key_chip,
    surface::{
        color_swatch, compact_copyable_command, copyable_command, empty_state_panel, status_chip,
        trust_decision_panel, TrustTone,
    },
    text_field::{TextArea, TextField},
    toast::{toast_caption, ProgressDots},
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
        gallery_row(ui, data, colors, name, index, false, index == 1);
    }
    for name in data.names {
        gallery_row(ui, data, colors, name, 0, true, false);
    }
}

fn gallery_row(
    ui: &mut egui::Ui,
    data: &GalleryFixture,
    colors: &Colors,
    name: &str,
    index: usize,
    dense: bool,
    selected: bool,
) -> egui::Response {
    let row = ListRow::new(name)
        .chip("app")
        .trailing_action(if index == 2 { "Delete" } else { "Open" })
        .selected(selected);
    let row = if dense {
        row.dense()
    } else {
        row.secondary(data.detail)
            .metadata_chips(data.tags)
            .pane_pips(ListRowPips {
                count: data.pips,
                focused_idx: Some(0),
                hidden_indices: vec![],
                activities: vec![],
                static_health_pips: vec![],
            })
            .danger_trailing(index == 2)
    };
    row.show(ui, colors).response()
}

fn gallery_sidebar_row(
    ui: &mut egui::Ui,
    data: &GalleryFixture,
    colors: &Colors,
    name: &str,
    index: usize,
    selected: bool,
) -> egui::Response {
    crate::ui::sidebar_row::SidebarRow {
        ancestry: Vec::new(),
        is_active: selected,
        is_dragging: false,
        any_dragging: false,
        action_enabled: true,
        ctx_name: name.to_owned(),
        ctx_index: Some(index),
        badge_count: data.count,
        subtitle: Some(data.command.to_owned()),
        draggable: false,
        pane_dots: Some(crate::ui::sidebar_row::PaneDots {
            route_idx: None,
            count: data.pips,
            focused_idx: Some(0),
            hidden_set: Default::default(),
            activities: vec![],
            static_health_pips: Default::default(),
            windows: vec![],
        }),
    }
    .show(ui, egui::Id::new(("gallery_sidebar", index)), colors)
    .1
}

fn gallery_text_field(
    ui: &mut egui::Ui,
    colors: &Colors,
    id: egui::Id,
    hint: &str,
    buffer: &mut String,
    log_name: Option<&'static str>,
) -> egui::Response {
    let field = TextField::singleline(id, hint);
    let field = if let Some(name) = log_name {
        field.log_name(name)
    } else {
        field
    };
    field.show(ui, buffer, colors)
}

fn gallery_text_area(ui: &mut egui::Ui, colors: &Colors, buffer: &mut String) -> egui::Response {
    TextArea::multiline(
        egui::Id::new("host_ui_gallery_text_area"),
        "Multiline text area",
    )
    .rows(3)
    .log_name("ui_gallery")
    .show(ui, buffer, colors)
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum KitSpecimen {
    Button {
        kind: ButtonKind,
        size: crate::ui::button::ButtonSize,
        label: &'static str,
    },
    GalleryButton {
        kind: ButtonKind,
        label: &'static str,
        min_width: f32,
    },
    Choice,
    Icon,
    Toolbar,
    CodeViewer,
    CopyCommand,
    CompactCommand,
    Progress,
    TrustNeutral,
    TrustWarning,
    TrustDanger,
    KeyChip,
    StatusChip,
    Badge,
    EmptyState,
    SwatchToolbar,
    SwatchActive,
    SwatchAccent,
    SwatchDanger,
    SwatchBorder,
    Hints,
    TrustHints,
    ToastCaption,
    Row,
    DenseRow,
    SidebarRow,
    TextField,
    TextArea,
}

impl KitSpecimen {
    pub(crate) fn show(
        self,
        ui: &mut egui::Ui,
        colors: &Colors,
        selected: bool,
        choice_label: &str,
        detail: &str,
    ) -> egui::Response {
        match self {
            Self::Button { kind, size, label } => ui.add(
                KitButton::new(label, colors)
                    .kind(kind)
                    .size(size)
                    .selected(selected),
            ),
            Self::GalleryButton {
                kind,
                label,
                min_width,
            } => ui.add(
                KitButton::new(label, colors)
                    .kind(kind)
                    .min_width(min_width)
                    .selected(selected),
            ),
            Self::Choice => choice_button(ui, choice_label, "⌘Enter", selected, colors),
            Self::Icon => icon_button(ui, "+", "Icon button", colors),
            Self::Toolbar => toolbar_button(
                ui,
                KitButton::new("Toolbar", colors).text_color(colors.text_dim),
                "Toolbar button",
            ),
            Self::CodeViewer => ReadOnlyCodeViewer::new(
                egui::Id::new("host_ui_gallery_code_viewer"),
                choice_label,
                "rs",
            )
            .line_numbers(true)
            .max_height(96.0)
            .show(ui, colors),
            Self::CopyCommand => copyable_command(
                ui,
                egui::Id::new("host_ui_gallery_copyable_command"),
                choice_label,
                colors,
            ),
            Self::CompactCommand => compact_copyable_command(
                ui,
                egui::Id::new("host_ui_gallery_compact_command"),
                choice_label,
                colors,
            ),
            Self::Progress => ProgressDots::new(2, 3).show(ui, colors),
            Self::TrustNeutral => {
                trust_decision_panel(ui, choice_label, detail, TrustTone::Neutral, colors)
            }
            Self::TrustWarning => {
                trust_decision_panel(ui, choice_label, detail, TrustTone::Warning, colors)
            }
            Self::TrustDanger => {
                trust_decision_panel(ui, choice_label, detail, TrustTone::Danger, colors)
            }
            Self::KeyChip => key_chip(
                ui,
                choice_label,
                colors,
                egui::FontId::monospace(style::TEXT_HINT),
            ),
            Self::StatusChip => status_chip(ui, choice_label, colors),
            Self::Badge => crate::ui::badge::badge(ui, choice_label, colors.bg_active, colors),
            Self::EmptyState => empty_state_panel(ui, choice_label, Some(detail), colors),
            Self::Row => gallery_row(
                ui,
                &UiGalleryDataMode::Demo.fixture(),
                colors,
                "Normal row",
                0,
                false,
                selected,
            ),
            Self::DenseRow => gallery_row(
                ui,
                &UiGalleryDataMode::Demo.fixture(),
                colors,
                "Normal row",
                0,
                true,
                selected,
            ),
            Self::SidebarRow => gallery_sidebar_row(
                ui,
                &UiGalleryDataMode::Demo.fixture(),
                colors,
                "Normal row",
                0,
                selected,
            ),
            Self::TextField => gallery_text_field(
                ui,
                colors,
                egui::Id::new("host_ui_gallery_text_normal"),
                "Normal text field",
                &mut choice_label.to_owned(),
                None,
            ),
            Self::TextArea => gallery_text_area(ui, colors, &mut choice_label.to_owned()),
            Self::SwatchToolbar => color_swatch(ui, "toolbar", colors.bg_toolbar, colors),
            Self::SwatchActive => color_swatch(ui, "active", colors.bg_active, colors),
            Self::SwatchAccent => color_swatch(ui, "accent", colors.accent, colors),
            Self::SwatchDanger => color_swatch(ui, "danger", colors.danger, colors),
            Self::SwatchBorder => color_swatch(ui, "border", colors.border, colors),
            Self::Hints => ui.scope(|ui| hint_bar(ui, colors)).response,
            Self::TrustHints => {
                ui.scope(|ui| {
                    let hints = [
                        HintGroup::new(&["Enter"], "confirm"),
                        HintGroup::new(&["Esc"], "cancel"),
                        HintGroup::new(&["Shift", "Esc"], "deny forever"),
                    ];
                    HintBar::new(&hints).show(ui, colors);
                })
                .response
            }
            Self::ToastCaption => {
                ui.scope(|ui| toast_caption(ui, choice_label, colors))
                    .response
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn interactive(self) -> bool {
        matches!(
            self,
            Self::Button { .. }
                | Self::GalleryButton { .. }
                | Self::Choice
                | Self::Icon
                | Self::Toolbar
                | Self::Row
                | Self::DenseRow
                | Self::SidebarRow
                | Self::TextField
                | Self::TextArea
        )
    }
}

pub(crate) fn kit_specimens() -> impl Iterator<Item = KitSpecimen> {
    use crate::ui::button::ButtonSize;
    [ButtonSize::Sm, ButtonSize::Md, ButtonSize::Lg]
        .into_iter()
        .flat_map(|size| {
            [
                (ButtonKind::Primary, "Primary"),
                (ButtonKind::Secondary, "Secondary"),
                (ButtonKind::Accent, "Accent"),
                (ButtonKind::Danger, "Danger"),
                (ButtonKind::Ghost, "Ghost"),
            ]
            .into_iter()
            .map(move |(kind, label)| KitSpecimen::Button { kind, size, label })
        })
        .chain([
            KitSpecimen::GalleryButton {
                kind: ButtonKind::Ghost,
                label: "Full width",
                min_width: 0.0,
            },
            KitSpecimen::GalleryButton {
                kind: ButtonKind::Primary,
                label: "Install",
                min_width: 88.0,
            },
            KitSpecimen::GalleryButton {
                kind: ButtonKind::Secondary,
                label: "Cancel",
                min_width: 88.0,
            },
            KitSpecimen::GalleryButton {
                kind: ButtonKind::Danger,
                label: "Deny forever",
                min_width: 116.0,
            },
            KitSpecimen::GalleryButton {
                kind: ButtonKind::Primary,
                label: "Open text-entry modal",
                min_width: 180.0,
            },
            KitSpecimen::Choice,
            KitSpecimen::Icon,
            KitSpecimen::Toolbar,
            KitSpecimen::CodeViewer,
            KitSpecimen::CopyCommand,
            KitSpecimen::CompactCommand,
            KitSpecimen::Progress,
            KitSpecimen::TrustNeutral,
            KitSpecimen::TrustWarning,
            KitSpecimen::TrustDanger,
            KitSpecimen::KeyChip,
            KitSpecimen::StatusChip,
            KitSpecimen::Badge,
            KitSpecimen::EmptyState,
            KitSpecimen::SwatchToolbar,
            KitSpecimen::SwatchActive,
            KitSpecimen::SwatchAccent,
            KitSpecimen::SwatchDanger,
            KitSpecimen::SwatchBorder,
            KitSpecimen::Hints,
            KitSpecimen::TrustHints,
            KitSpecimen::ToastCaption,
            KitSpecimen::Row,
            KitSpecimen::DenseRow,
            KitSpecimen::SidebarRow,
            KitSpecimen::TextField,
            KitSpecimen::TextArea,
        ])
}

/// Every kind at every size class, plus the disabled state. Rendered by the
/// gallery and by the `one-kit-gallery` scene fixture.
pub(crate) fn draw_button_sizes(ui: &mut egui::Ui, colors: &Colors) {
    use crate::ui::button::ButtonSize;
    for (size, caption) in [
        (ButtonSize::Sm, "Sm · 28"),
        (ButtonSize::Md, "Md · 32 (default)"),
        (ButtonSize::Lg, "Lg · 40"),
    ] {
        crate::ui::typography::caption(ui, caption, colors);
        ui.horizontal_wrapped(|ui| {
            for specimen in kit_specimens().filter(|specimen| matches!(specimen, KitSpecimen::Button { size: specimen_size, .. } if *specimen_size == size)) {
                specimen.show(ui, colors, false, "", "");
            }
            ui.add_enabled_ui(false, |ui| {
                KitSpecimen::Button { kind: ButtonKind::Primary, size, label: "Disabled" }.show(ui, colors, false, "", "");
            });
        });
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
                        if (KitSpecimen::GalleryButton {
                            kind: ButtonKind::Ghost,
                            label: mode.label(),
                            min_width: 0.0,
                        })
                        .show(ui, &colors, preview.mode == mode, "", "")
                        .clicked()
                        {
                            preview.mode = mode;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    for (narrow, label) in [(false, "Full width"), (true, "Narrow 320px")] {
                        if (KitSpecimen::GalleryButton {
                            kind: ButtonKind::Ghost,
                            label,
                            min_width: 0.0,
                        })
                        .show(ui, &colors, preview.narrow == narrow, "", "")
                        .clicked()
                        {
                            preview.narrow = narrow;
                        }
                    }
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
                            KitSpecimen::Hints.show(ui, &colors, false, "", "");
                        });

                        chrome_section(ui, "Code and toast surfaces", &colors, |ui| {
                            KitSpecimen::CodeViewer.show(ui, &colors, false, data.code, "");
                            ui.add_space(style::SPACE_SM);
                            KitSpecimen::CopyCommand.show(ui, &colors, false, data.command, "");
                            ui.add_space(style::SPACE_SM);
                            ui.vertical(|ui| {
                                KitSpecimen::ToastCaption.show(
                                    ui,
                                    &colors,
                                    false,
                                    "Shell completions aren't set up.",
                                    "",
                                );
                                KitSpecimen::CompactCommand.show(
                                    ui,
                                    &colors,
                                    false,
                                    data.command,
                                    "",
                                );
                                KitSpecimen::Progress.show(ui, &colors, false, "", "");
                            });
                        });

                        chrome_section(ui, "Trust and confirmations", &colors, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (label, kind, min_width) in [
                                    ("Install", ButtonKind::Primary, 88.0),
                                    ("Cancel", ButtonKind::Secondary, 88.0),
                                    ("Deny forever", ButtonKind::Danger, 116.0),
                                ] {
                                    KitSpecimen::GalleryButton {
                                        kind,
                                        label,
                                        min_width,
                                    }
                                    .show(ui, &colors, false, "", "");
                                }
                            });
                            KitSpecimen::TrustHints.show(ui, &colors, false, "", "");
                            for (index, specimen) in [
                                KitSpecimen::TrustNeutral,
                                KitSpecimen::TrustWarning,
                                KitSpecimen::TrustDanger,
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                if index > 0 {
                                    ui.add_space(style::SPACE_SM);
                                }
                                specimen.show(
                                    ui,
                                    &colors,
                                    false,
                                    data.names.first().copied().unwrap_or(""),
                                    data.detail,
                                );
                            }
                        });

                        chrome_section(ui, "Rows", &colors, |ui| {
                            gallery_rows(ui, &data, &colors);
                        });
                        chrome_section(ui, "Sidebar rows", &colors, |ui| {
                            for (index, name) in data.names.iter().enumerate() {
                                gallery_sidebar_row(ui, &data, &colors, name, index, index == 1);
                            }
                        });

                        chrome_section(ui, "Text fields", &colors, |ui| {
                            gallery_text_field(
                                ui,
                                &colors,
                                egui::Id::new("host_ui_gallery_text_normal"),
                                "Normal text field",
                                &mut self.ui_gallery_normal_buf,
                                None,
                            );
                            ui.add_space(style::SPACE_SM);
                            // Focus follows clicks; a hardcoded focus demo would fight the user.
                            gallery_text_field(
                                ui,
                                &colors,
                                egui::Id::new("host_ui_gallery_text_focused"),
                                "Click to focus — accent ring + cursor",
                                &mut self.ui_gallery_focused_buf,
                                Some("ui_gallery"),
                            );
                            ui.add_space(style::SPACE_SM);
                            let mut multiline_demo = data.code.to_owned();
                            gallery_text_area(ui, &colors, &mut multiline_demo);
                        });

                        chrome_section(ui, "Modal patterns", &colors, |ui| {
                            if (KitSpecimen::GalleryButton {
                                kind: ButtonKind::Primary,
                                label: "Open text-entry modal",
                                min_width: 180.0,
                            })
                            .show(ui, &colors, false, "", "")
                            .clicked()
                            {
                                self.ui_gallery_show_text_modal = true;
                                log::info!("ui_gallery: opened text-entry modal demo");
                            }
                        });

                        chrome_section(ui, "Buttons and chips", &colors, |ui| {
                            draw_button_sizes(ui, &colors);
                            ui.horizontal_wrapped(|ui| {
                                for (label, kind, min_width) in [
                                    (
                                        data.names.first().copied().unwrap_or(""),
                                        ButtonKind::Primary,
                                        80.0,
                                    ),
                                    ("Secondary", ButtonKind::Secondary, 92.0),
                                    ("Danger", ButtonKind::Danger, 80.0),
                                    ("Accent", ButtonKind::Accent, 80.0),
                                ] {
                                    KitSpecimen::GalleryButton {
                                        kind,
                                        label,
                                        min_width,
                                    }
                                    .show(ui, &colors, false, "", "");
                                }
                                ui.add_enabled_ui(false, |ui| {
                                    KitSpecimen::GalleryButton {
                                        kind: ButtonKind::Secondary,
                                        label: "Disabled",
                                        min_width: 80.0,
                                    }
                                    .show(ui, &colors, false, "", "");
                                });
                            });
                            KitSpecimen::Choice.show(ui, &colors, false, data.detail, "");
                            ui.add_space(style::SPACE_SM);
                            ui.horizontal(|ui| {
                                KitSpecimen::Icon.show(ui, &colors, false, "", "");
                                KitSpecimen::Toolbar.show(ui, &colors, false, "", "");
                            });
                            ui.add_space(style::SPACE_SM);
                            ui.horizontal_wrapped(|ui| {
                                KitSpecimen::KeyChip.show(ui, &colors, false, "app", "");
                                KitSpecimen::KeyChip.show(ui, &colors, false, "term", "");
                                KitSpecimen::StatusChip.show(ui, &colors, false, data.status, "");
                                KitSpecimen::Badge.show(
                                    ui,
                                    &colors,
                                    false,
                                    &crate::ui::text::format_count_plural(
                                        data.count, "item", "items",
                                    ),
                                    "",
                                );
                                KitSpecimen::StatusChip.show(ui, &colors, false, "error", "");
                                KitSpecimen::StatusChip.show(ui, &colors, false, "empty", "");
                            });
                        });

                        chrome_section(ui, "Empty states", &colors, |ui| {
                            KitSpecimen::EmptyState.show(
                                ui,
                                &colors,
                                false,
                                data.empty,
                                data.detail,
                            );
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
                    gallery_text_field(
                        ui,
                        &colors,
                        egui::Id::new("host_ui_gallery_modal_field"),
                        "Type something...",
                        &mut self.ui_gallery_modal_buf,
                        Some("ui_gallery_modal"),
                    );
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
        for specimen in [
            KitSpecimen::SwatchToolbar,
            KitSpecimen::SwatchActive,
            KitSpecimen::SwatchAccent,
            KitSpecimen::SwatchDanger,
            KitSpecimen::SwatchBorder,
        ] {
            specimen.show(ui, colors, false, "", "");
        }
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
