//! File-open handler specs for the `[file_handlers]` config table and the
//! File Explorer open path (#2283).
//!
//! A handler spec is a `kind:value` string chosen by the user per file
//! extension. The grammar is prefix-namespaced by *handler kind* (not by
//! `plexi:`) so the value space stays open-ended — new kinds (`mcp:`, `url:`,
//! …) slot in without colliding with the app-id namespace:
//!
//!   `app:<id>`      — open in a Plexi app by its registry id
//!   `os`            — hand to the OS default opener (`open` / `xdg-open`)
//!   `cmd:<command>` — reserved; parsed now, executed in a follow-up
//!
//! `os` is the lone bare keyword (it is a singleton with no value), kept bare
//! for ergonomics rather than forcing `os:default`.

/// A parsed file-open handler. See the module docs for the grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileHandler {
    /// Open in a Plexi app by registry id.
    App(String),
    /// Hand to the OS default opener.
    Os,
    /// Run an external command with the path appended. Reserved — not yet
    /// executed; callers fall back to [`FileHandler::Os`] behavior.
    Cmd(String),
}

impl FileHandler {
    /// Parse a `kind:value` handler spec. Returns `None` for an empty or
    /// malformed spec (unknown kind, or `app:`/`cmd:` with no value) so the
    /// caller can warn and fall through to the next resolution tier.
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec == "os" {
            return Some(FileHandler::Os);
        }
        if let Some(id) = spec.strip_prefix("app:") {
            let id = id.trim();
            return (!id.is_empty()).then(|| FileHandler::App(id.to_string()));
        }
        if let Some(cmd) = spec.strip_prefix("cmd:") {
            let cmd = cmd.trim();
            return (!cmd.is_empty()).then(|| FileHandler::Cmd(cmd.to_string()));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_app_handler() {
        assert_eq!(
            FileHandler::parse("app:text-editor"),
            Some(FileHandler::App("text-editor".to_string()))
        );
    }

    #[test]
    fn parses_os_keyword() {
        assert_eq!(FileHandler::parse("os"), Some(FileHandler::Os));
    }

    #[test]
    fn parses_cmd_handler() {
        assert_eq!(
            FileHandler::parse("cmd:code -w"),
            Some(FileHandler::Cmd("code -w".to_string()))
        );
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            FileHandler::parse("  app:text-editor  "),
            Some(FileHandler::App("text-editor".to_string()))
        );
    }

    #[test]
    fn rejects_unknown_kind() {
        assert_eq!(FileHandler::parse("plexi:text-editor"), None);
        assert_eq!(FileHandler::parse("text-editor"), None);
        assert_eq!(FileHandler::parse(""), None);
    }

    #[test]
    fn rejects_empty_value() {
        assert_eq!(FileHandler::parse("app:"), None);
        assert_eq!(FileHandler::parse("cmd:"), None);
        assert_eq!(FileHandler::parse("app:   "), None);
    }
}
