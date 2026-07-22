//! Editor presentation/input mode (stint 0318). Pure — no egui.
//!
//! The mode changes how the widget *translates input* and *presents* the
//! document (Markdown structure commands; code gutter, current-line
//! highlight, syntax spans). It is deliberately not stored on
//! [`super::Document`]: the document model, selection, history, IME, and
//! persistence are identical in every mode.

/// How the shared editor presents a document and routes structural keys.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EditorMode {
    /// No structural key handling; no code chrome.
    #[default]
    PlainText,
    /// Markdown structure commands on Tab/Shift-Tab/Enter/Backspace
    /// (list continuation, line-wise indent, marker-aware backspace).
    /// `live_preview` selects Obsidian-style presentation: inactive blocks
    /// render styled while the active block reveals raw source. `false` is
    /// plain source mode. The presentation flag never changes document,
    /// selection, history, or IME behavior.
    Markdown {
        live_preview: bool,
    },
    /// Code presentation: line-number gutter, current-line highlight, and
    /// syntax-highlight spans for `language` (a language identifier such as a
    /// file extension token — detection from file metadata is the caller's
    /// job, never the core's). Unknown languages fall back to plain spans.
    Code {
        language: String,
    },
}

impl EditorMode {
    /// Whether Tab/Shift-Tab/Enter/Backspace route through the
    /// Markdown-aware commands.
    #[must_use]
    pub fn is_markdown(&self) -> bool {
        matches!(self, EditorMode::Markdown { .. })
    }

    /// Whether Markdown Live Preview presentation is on (styled inactive
    /// blocks, raw source around the caret).
    #[must_use]
    pub fn is_live_preview(&self) -> bool {
        matches!(self, EditorMode::Markdown { live_preview: true })
    }

    /// Whether code chrome (gutter, current-line highlight, syntax spans)
    /// is shown.
    #[must_use]
    pub fn is_code(&self) -> bool {
        matches!(self, EditorMode::Code { .. })
    }

    /// The language identifier for code mode, if any.
    #[must_use]
    pub fn language(&self) -> Option<&str> {
        match self {
            EditorMode::Code { language } => Some(language),
            _ => None,
        }
    }

    /// Short name for logging.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            EditorMode::PlainText => "plain".to_string(),
            EditorMode::Markdown { live_preview: true } => "markdown:live-preview".to_string(),
            EditorMode::Markdown { live_preview: false } => "markdown:source".to_string(),
            EditorMode::Code { language } => format!("code:{language}"),
        }
    }
}
