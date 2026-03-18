use std::io;
use std::path::PathBuf;

/// Writes Plexi's zsh shell integration files to ~/.plexi/shell-integration/zsh/
/// and returns the directory path to use as ZDOTDIR.
///
/// The integration:
/// 1. Sources the user's real .zprofile / .zshrc (preserving their config)
/// 2. Adds a precmd hook that emits the current directory as an OSC sequence
///    after each prompt, which Plexi reads to keep panel.cwd up to date.
pub fn ensure_shell_integration() -> io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory not found"))?;

    let zsh_dir = home
        .join(".plexi")
        .join("shell-integration")
        .join("zsh");

    std::fs::create_dir_all(&zsh_dir)?;

    // Sources the user's .zprofile (login shell startup)
    let zprofile = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_orig="${PLEXI_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$__plexi_orig/.zprofile" ]] && source "$__plexi_orig/.zprofile"
unset __plexi_orig
"#;

    // Sources the user's .zshrc then appends a precmd hook that emits OSC 7 —
    // the standard cwd protocol supported by iTerm2, Ghostty, Kitty, WezTerm,
    // and fish (built-in). Format: \e]7;file://hostname/path\a
    let zshrc = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_orig="${PLEXI_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$__plexi_orig/.zshrc" ]] && source "$__plexi_orig/.zshrc"
unset __plexi_orig

# Emit OSC 7 after each prompt so Plexi can track cwd for split inheritance
__plexi_precmd() {
    printf '\e]7;file://%s%s\a' "$HOST" "$PWD"
}
precmd_functions+=(__plexi_precmd)
"#;

    std::fs::write(zsh_dir.join(".zprofile"), zprofile)?;
    std::fs::write(zsh_dir.join(".zshrc"), zshrc)?;

    Ok(zsh_dir)
}
