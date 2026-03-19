use std::io;
use std::path::PathBuf;

/// Writes Plexi's zsh shell integration files to ~/.plexi/shell-integration/zsh/
/// and returns the directory path to use as ZDOTDIR.
///
/// The integration:
/// 1. Sources the user's real startup files in zsh's normal order.
/// 2. Tracks the effective user ZDOTDIR after `.zshenv`, so setups that move
///    dotfiles into a custom directory still work.
/// 3. Adds a precmd hook that emits the current directory as an OSC sequence
///    after each prompt, which Plexi reads to keep panel.cwd up to date.
pub fn ensure_shell_integration() -> io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory not found"))?;

    let zsh_dir = home
        .join(".plexi")
        .join("shell-integration")
        .join("zsh");

    std::fs::create_dir_all(&zsh_dir)?;

    let zshenv = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_orig="${PLEXI_ORIG_ZDOTDIR:-$HOME}"
__plexi_integration="${ZDOTDIR:-$__plexi_orig}"
[[ -f "$__plexi_orig/.zshenv" ]] && source "$__plexi_orig/.zshenv"
if [[ -n "${ZDOTDIR:-}" && "$ZDOTDIR" != "$__plexi_integration" ]]; then
    export PLEXI_RESOLVED_ZDOTDIR="$ZDOTDIR"
else
    export PLEXI_RESOLVED_ZDOTDIR="$__plexi_orig"
fi
unset __plexi_orig
unset __plexi_integration
"#;

    // Sources the user's .zprofile (login shell startup).
    let zprofile = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_user="${PLEXI_RESOLVED_ZDOTDIR:-${PLEXI_ORIG_ZDOTDIR:-$HOME}}"
[[ -f "$__plexi_user/.zprofile" ]] && source "$__plexi_user/.zprofile"
unset __plexi_user
"#;

    // Sources the user's .zshrc then appends a precmd hook that emits OSC 7 —
    // the standard cwd protocol supported by iTerm2, Ghostty, Kitty, WezTerm,
    // and fish (built-in). Format: \e]7;file://hostname/path\a
    let zshrc = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_user="${PLEXI_RESOLVED_ZDOTDIR:-${PLEXI_ORIG_ZDOTDIR:-$HOME}}"
[[ -f "$__plexi_user/.zshrc" ]] && source "$__plexi_user/.zshrc"
unset __plexi_user

# Emit OSC 7 after each prompt so Plexi can track cwd for split inheritance
__plexi_precmd() {
    printf '\e]7;file://%s%s\a' "$HOST" "$PWD"
}
precmd_functions+=(__plexi_precmd)
"#;

    // Sources the user's .zlogin and restores the resolved user ZDOTDIR for the
    // interactive session after Plexi's startup files have finished loading.
    let zlogin = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_user="${PLEXI_RESOLVED_ZDOTDIR:-${PLEXI_ORIG_ZDOTDIR:-$HOME}}"
[[ -f "$__plexi_user/.zlogin" ]] && source "$__plexi_user/.zlogin"
export ZDOTDIR="$__plexi_user"
unset __plexi_user
unset PLEXI_RESOLVED_ZDOTDIR
unset PLEXI_ORIG_ZDOTDIR
"#;

    std::fs::write(zsh_dir.join(".zshenv"), zshenv)?;
    std::fs::write(zsh_dir.join(".zprofile"), zprofile)?;
    std::fs::write(zsh_dir.join(".zshrc"), zshrc)?;
    std::fs::write(zsh_dir.join(".zlogin"), zlogin)?;

    Ok(zsh_dir)
}
