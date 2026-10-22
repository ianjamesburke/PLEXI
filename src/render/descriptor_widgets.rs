//! Shared UI upcycle widgets for CLI and MCP descriptors.
//! Callers own navigation, submission, validation and transport. These widgets
//! only paint command choices and edit named values; they never execute tools.

use crate::app::plexi_descriptor::{ArgSpec, ArgType, Command, ROOT_COMMAND_NAME};
use crate::ui::{style, theme::Colors};
use egui::RichText;
use std::collections::HashMap;

/// Paint the current command level. The owner handles arrows/Enter and supplies
/// selection; a click returns the selected index. Scope the UI per pane.
pub fn command_list<'a>(
    ui: &mut egui::Ui,
    colors: &Colors,
    commands: impl IntoIterator<Item = &'a Command>,
    root_label: &str,
    selected: usize,
    selection_changed: bool,
) -> Option<usize> {
    let mut clicked = None;
    for (i, command) in commands.into_iter().enumerate() {
        let label = if command.name == ROOT_COMMAND_NAME {
            root_label
        } else {
            &command.name
        };
        let mut row = crate::ui::list::ListRow::new(label).selected(i == selected);
        if let Some(description) = &command.description {
            row = row.secondary(description);
        }
        if !command.commands.is_empty() {
            row = row.chip("›");
        }
        let response = row.show(ui, colors);
        if i == selected {
            response.scroll_into_view(ui, selection_changed);
        }
        if response.row_clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// Edit descriptor fields in caller-provided order. Values use ArgSpec names
/// as keys, bools use "true"/empty, and all other values stay literal strings.
/// Defaults are seeded by the caller. `focus_first` claims the first text field
/// through the host focus registry; IDs include `pane_key` for pane isolation.
pub fn form_fields<'a>(
    ui: &mut egui::Ui,
    colors: &Colors,
    fields: impl IntoIterator<Item = &'a ArgSpec>,
    values: &mut HashMap<String, String>,
    focus_first: bool,
    pane_key: u64,
) {
    let mut want_focus = focus_first;
    for field in fields {
        if render_field(ui, colors, field, values, want_focus, pane_key) {
            want_focus = false;
        }
    }
}

/// Renders one arg/flag field. When `want_focus` is true and this field is
/// a text input, it requests keyboard focus and returns true so the caller
/// stops offering focus to later fields.
fn render_field(
    ui: &mut egui::Ui,
    colors: &crate::ui::theme::Colors,
    arg: &ArgSpec,
    values: &mut HashMap<String, String>,
    want_focus: bool,
    pane_key: u64,
) -> bool {
    let mut took_focus = false;
    ui.horizontal(|ui| {
        ui.add_space(style::SPACE_MD);
        ui.vertical(|ui| {
            // Label
            let required = arg.required.unwrap_or(false);
            let label_text = if required {
                format!("{}*", arg.name)
            } else {
                arg.name.clone()
            };
            ui.label(
                RichText::new(&label_text)
                    .size(style::TEXT_CAPTION)
                    .color(colors.text_primary),
            );

            if let Some(desc) = &arg.description {
                if !desc.is_empty() {
                    ui.label(
                        RichText::new(desc)
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }
            }

            // Input
            let val = values.entry(arg.name.clone()).or_default();

            match arg.ty {
                ArgType::Bool => {
                    let mut checked = val == "true";
                    if ui.checkbox(&mut checked, "").changed() {
                        *val = if checked {
                            "true".into()
                        } else {
                            String::new()
                        };
                    }
                }
                ArgType::Enum => {
                    if let Some(options) = &arg.enum_values {
                        egui::ComboBox::from_id_salt(("descriptor_enum", pane_key, &arg.name))
                            .selected_text(if val.is_empty() {
                                "(select)"
                            } else {
                                val.as_str()
                            })
                            .show_ui(ui, |ui| {
                                for opt in options {
                                    ui.selectable_value(val, opt.clone(), opt);
                                }
                            });
                    }
                }
                _ => {
                    let placeholder = arg.placeholder.as_deref().unwrap_or("");
                    ui.set_max_width(ui.available_width() - style::SPACE_MD);
                    let resp = crate::ui::text_field::styled_text_input(
                        ui,
                        val,
                        placeholder,
                        egui::Id::new(("descriptor_field", pane_key, arg.name.as_str())),
                        colors,
                    );
                    crate::ui::focus::register_text_surface(
                        ui.ctx(),
                        crate::ui::focus::SurfaceKey::Pane(pane_key),
                        resp.id,
                    );
                    if want_focus {
                        crate::ui::focus::claim_text_surface(
                            ui.ctx(),
                            crate::ui::focus::SurfaceKey::Pane(pane_key),
                            resp.id,
                        );
                        took_focus = true;
                        log::info!("CliRendererApp: auto-focused field '{}'", arg.name);
                    }
                }
            }

            ui.add_space(style::SPACE_SM);
        });
    });
    took_focus
}
