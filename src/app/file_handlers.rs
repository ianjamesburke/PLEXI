//! File-open handler specs for the `[file_handlers]` config table and the
//! File Explorer open path (#2283).
//!
//! A handler spec is a `kind:value` string chosen by the user per file
//! extension. The grammar is prefix-namespaced by *handler kind* (not by
//! `plexi:`) so the value space stays open-ended — new kinds (`mcp:`, `url:`,
//! …) slot in without colliding with the plugin-id namespace:
//!
//!   `app:<id>`      — stored spelling for opening a plugin's default view
//! This persisted config grammar stays unchanged to preserve user associations.
//! It is not a CLI alias; new CLI calls use `view open`.
//!   `os`            — hand to the OS default opener (`open` / `xdg-open`)
//!
//! `cmd:<command>` is not supported yet. It is rejected with a message rather
//! than accepted and silently treated as `os`.
//!
//! `os` is the lone bare keyword (it is a singleton with no value), kept bare
//! for ergonomics rather than forcing `os:default`.

/// A parsed file-open handler. See the module docs for the grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileHandler {
    /// Open in a Plexi plugin by registry id.
    View(String),
    /// Hand to the OS default opener.
    Os,
}

impl FileHandler {
    /// Parse a `kind:value` handler spec. An empty, malformed or unsupported
    /// spec (unknown kind, `app:` with no value, any `cmd:`) is an error that
    /// names the spec and the accepted forms, so the config loader can report
    /// it and the open path can skip it.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        if spec == "os" {
            return Ok(FileHandler::Os);
        }
        if let Some(id) = spec.strip_prefix("app:") {
            let id = id.trim();
            if id.is_empty() {
                return Err(format!("{spec:?} names no plugin; use `app:<id>`"));
            }
            return Ok(FileHandler::View(id.to_string()));
        }
        if spec.starts_with("cmd:") {
            return Err(format!(
                "{spec:?} is not supported: `cmd:` handlers are not implemented; use `os` or `app:<id>`"
            ));
        }
        Err(format!(
            "{spec:?} is not a file handler; use `os` or `app:<id>`"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plugin_handler() {
        assert_eq!(
            FileHandler::parse("app:text-editor"),
            Ok(FileHandler::View("text-editor".to_string()))
        );
    }

    #[test]
    fn parses_os_keyword() {
        assert_eq!(FileHandler::parse("os"), Ok(FileHandler::Os));
    }

    #[test]
    fn rejects_cmd_handler_until_implemented() {
        let err = FileHandler::parse("cmd:code -w").unwrap_err();
        assert!(err.contains("cmd:"), "{err}");
        assert!(err.contains("not implemented"), "{err}");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            FileHandler::parse("  app:text-editor  "),
            Ok(FileHandler::View("text-editor".to_string()))
        );
    }

    #[test]
    fn rejects_unknown_kind() {
        assert!(FileHandler::parse("plexi:text-editor").is_err());
        assert!(FileHandler::parse("text-editor").is_err());
        assert!(FileHandler::parse("").is_err());
    }

    #[test]
    fn rejects_empty_value() {
        assert!(FileHandler::parse("app:").is_err());
        assert!(FileHandler::parse("cmd:").is_err());
        assert!(FileHandler::parse("app:   ").is_err());
    }
}
