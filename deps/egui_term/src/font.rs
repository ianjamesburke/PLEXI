use egui::{Context, FontId};

use crate::types::Size;

#[derive(Debug, Clone)]
pub struct FontSettings {
    pub font_type: FontId,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            font_type: FontId::monospace(14.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalFont {
    font_type: FontId,
    bold_font_type: Option<FontId>,
}

impl Default for TerminalFont {
    fn default() -> Self {
        Self {
            font_type: FontSettings::default().font_type,
            bold_font_type: None,
        }
    }
}

impl TerminalFont {
    pub fn new(settings: FontSettings) -> Self {
        Self {
            font_type: settings.font_type,
            bold_font_type: None,
        }
    }

    pub fn font_type(&self) -> FontId {
        self.font_type.clone()
    }

    /// Supply a bold face with the same metrics as the regular terminal face.
    /// Grid sizing always uses the regular face, regardless of cell attributes.
    pub fn with_bold_font(mut self, font_type: FontId) -> Self {
        self.bold_font_type = Some(font_type);
        self
    }

    pub fn font_type_for_bold(&self, bold: bool) -> FontId {
        if bold {
            self.bold_font_type
                .as_ref()
                .unwrap_or(&self.font_type)
                .clone()
        } else {
            self.font_type()
        }
    }

    pub fn font_measure(&self, ctx: &Context) -> Size {
        let (width, height) = ctx.fonts_mut(|f| {
            (
                f.glyph_width(&self.font_type, 'm'),
                f.row_height(&self.font_type),
            )
        });

        Size::new(width, height)
    }
}
