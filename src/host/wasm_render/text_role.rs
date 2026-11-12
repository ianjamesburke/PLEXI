//! Python semantic text roles retain their intent until the host paints them.
//! Like Card/Table metadata, this travels in the host-owned indexed-node key,
//! keeping the shared WIT component ABI unchanged. Literal RGBA stays in Text.
use crate::ui::theme::Colors;
use egui::Color32;

const PREFIX: &str = "__plexi_text_role__:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextRole {
    Primary,
    Muted,
    Section,
    Accent,
    Success,
    Warning,
    Danger,
}

impl TextRole {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "fg" => Self::Primary,
            "neutral" | "muted" => Self::Muted,
            "text_section" => Self::Section,
            "accent" => Self::Accent,
            "success" | "green" => Self::Success,
            "warning" | "yellow" => Self::Warning,
            "danger" | "red" => Self::Danger,
            _ => return None,
        })
    }

    pub(crate) fn key(self, identity: &str) -> String {
        let name = match self {
            Self::Primary => "fg",
            Self::Muted => "muted",
            Self::Section => "text_section",
            Self::Accent => "accent",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Danger => "danger",
        };
        format!("{PREFIX}{name}:{identity}")
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        let (name, _) = key.strip_prefix(PREFIX)?.split_once(':')?;
        Self::parse(name)
    }

    pub(crate) fn preferred(self, colors: &Colors) -> Color32 {
        match self {
            Self::Primary => colors.text_primary,
            Self::Muted => colors.text_dim,
            Self::Section => colors.text_section,
            Self::Accent => colors.accent,
            Self::Success => colors.success,
            Self::Warning => colors.warning,
            Self::Danger => colors.danger,
        }
    }
}
