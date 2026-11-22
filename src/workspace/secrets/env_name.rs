//! The one rule for the environment name a secret is exported under
//! (stint 1682).
//!
//! A pane's environment is the host's base (account, terminal, locale,
//! routing) plus launch metadata plus the secrets its grant permits. A
//! secret under a base or metadata name would let the value travel as
//! base environment, for example when an eligible pane that received it
//! launches another host. So every name a base or metadata writer emits,
//! every legacy base name that writer no longer emits, and the whole
//! `PLEXI_` namespace are reserved. The rule applies when a canonical name
//! is created or imported, when terminal injection is enabled, and when a
//! pane's secrets are resolved; a stored record under a reserved name is
//! quarantined (never read for a pane) rather than deleted.

/// The prefix every Plexi-owned pane variable carries.
const PLEXI_NAMESPACE: &str = "PLEXI_";

/// Base names the pane environment writer emits or once inherited.
const RESERVED: &[&str] = &[
    // Account.
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "PATH",
    "TMPDIR",
    // Terminal contract.
    "TERM",
    "COLORTERM",
    "WINDOWID",
    "ALACRITTY_WINDOW_ID",
    // Locale: LANG is emitted; the rest are omitted base names.
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_ADDRESS",
    "LC_IDENTIFICATION",
    "LC_MEASUREMENT",
    "LC_NAME",
    "LC_PAPER",
    "LC_TELEPHONE",
    "__CF_USER_TEXT_ENCODING",
    // Shell integration.
    "ZDOTDIR",
    // Signing agent.
    "SSH_AUTH_SOCK",
    // Linux desktop session.
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_DIRS",
    "XDG_DATA_DIRS",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_ACTIVATION_TOKEN",
    "DESKTOP_STARTUP_ID",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
];

/// True for a name no secret may be exported under.
pub(crate) fn is_reserved_env_name(name: &str) -> bool {
    name.starts_with(PLEXI_NAMESPACE) || RESERVED.contains(&name)
}

/// Environment-variable syntax: a letter or `_`, then letters, digits, `_`.
pub(crate) fn is_env_syntax(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// A name a secret may be exported to a pane under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretEnvName(String);

impl SecretEnvName {
    pub fn parse(name: &str) -> Result<Self, String> {
        if !is_env_syntax(name) {
            return Err(format!(
                "secret name '{name}' is not an environment variable name (letters, digits and underscores, not starting with a digit)"
            ));
        }
        if is_reserved_env_name(name) {
            return Err(format!(
                "secret name '{name}' is reserved for the pane environment Plexi builds; choose another name"
            ));
        }
        Ok(Self(name.to_string()))
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_and_malformed_names_are_refused() {
        for name in [
            "HOME",
            "PATH",
            "LC_PAPER",
            "LC_ALL",
            "SSH_AUTH_SOCK",
            "PLEXI_ORIG_ZDOTDIR",
            "PLEXI_CHANNEL",
            "PLEXI_ANYTHING_NEW",
            "ZDOTDIR",
        ] {
            assert!(SecretEnvName::parse(name).is_err(), "{name}");
        }
        for name in ["1BAD", "HAS-DASH", "", "openrouter-api-key"] {
            assert!(SecretEnvName::parse(name).is_err(), "{name}");
        }
        // Only exact standard locale categories are reserved.
        for name in [
            "LC_SERVICE_TOKEN",
            "OPENROUTER_API_KEY",
            "_PRIVATE",
            "PLEXIBLE",
        ] {
            assert_eq!(SecretEnvName::parse(name).unwrap().into_string(), name);
        }
    }
}
