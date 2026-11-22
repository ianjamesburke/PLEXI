//! The base of every Unix pane's environment (stint 1682).
//!
//! A pane is a fresh terminal. Its environment is built from independent
//! sources, never copied from the host's own: the host may have been started
//! from another pane, whose exported secrets, `HOME`, `PATH` or
//! `PLEXI_CHANNEL` would otherwise reach every pane this host opens. The base
//! is resolved once at host launch from the account (passwd database), the
//! resolved host profile, the executable and fixed platform lists. At each
//! spawn a pane adds its own metadata and only the secret classes its grant
//! permits (`pane_ops::construct`), and the Unix PTY clears the inherited
//! environment before applying the result (`egui_term::backend::pane_pty`).
//!
//! The boundary is the pane's first exec. A user's shell startup files then
//! run with that user's permissions and may export whatever they like.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::host::account::Account;

/// The terminal Plexi emulates.
pub(crate) const TERM: &str = "xterm-256color";
pub(crate) const COLORTERM: &str = "truecolor";

/// Locales tried in order, by explicit name; `C` when none is available.
#[cfg(target_os = "macos")]
const LOCALES: &[&str] = &["en_US.UTF-8"];
#[cfg(not(target_os = "macos"))]
const LOCALES: &[&str] = &["C.UTF-8", "en_US.UTF-8"];
const FALLBACK_LOCALE: &str = "C";

/// Homebrew's standard prefixes.
#[cfg(target_os = "macos")]
const PACKAGE_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
];
#[cfg(not(target_os = "macos"))]
const PACKAGE_DIRS: &[&str] = &["/usr/local/bin", "/usr/local/sbin"];

/// System binary directories, last on PATH.
const SYSTEM_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// Everything a Unix pane's environment starts from. Immutable once built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneBase {
    account: Account,
    path: String,
    lang: String,
    tmpdir: String,
    /// The host profile's suffix, exported as `PLEXI_CHANNEL` so a bare
    /// client in a pane routes to this host; `None` for `.plexi`.
    routing: Option<String>,
    /// The host's listener endpoint, exported as `PLEXI_SOCKET`.
    socket: String,
    /// A validated signing-agent socket from a trusted source, offered only
    /// to desktop-owned panes.
    ssh_auth_sock: Option<String>,
    /// Linux XDG base directories under the account home, and a verified
    /// runtime directory.
    xdg: Vec<(&'static str, String)>,
}

impl PaneBase {
    /// The base for this host process: the account, the installed host
    /// profile's routing and endpoint, and the running executable.
    #[cfg(unix)]
    pub(crate) fn for_host(account: Account) -> Result<Self, String> {
        let routing = crate::config::host_profile()
            .ok_or("pane base: no host profile installed")?
            .routing_suffix()
            .map(str::to_string);
        let exe = std::env::current_exe()
            .map_err(|error| format!("pane base: resolve executable: {error}"))?;
        Ok(Self::resolve(
            account,
            routing,
            crate::platform::ipc::endpoint(),
            &exe,
        ))
    }

    /// The base from explicit inputs. Reads the passwd-derived account, the
    /// filesystem (receipts, locale data, runtime and agent sockets) and the
    /// executable path; reads no environment variable.
    pub(crate) fn resolve(
        account: Account,
        routing: Option<String>,
        socket: String,
        exe: &Path,
    ) -> Self {
        let receipt_bin = receipt_bin_dir(exe, &account.home);
        let path = pane_path(&account.home, receipt_bin.as_deref(), exe);
        Self {
            path,
            lang: available_locale(LOCALES),
            tmpdir: account_tmpdir(),
            routing,
            socket,
            ssh_auth_sock: trusted_ssh_agent(),
            xdg: xdg_dirs(&account.home),
            account,
        }
    }

    pub(crate) fn shell(&self) -> &str {
        &self.account.shell
    }

    /// The base variables of pane `pane_id` running `shell`. A desktop-owned
    /// pane also gets the signing agent.
    pub(crate) fn env(&self, pane_id: u64, desktop: bool, shell: &str) -> HashMap<String, String> {
        let pane = pane_id.to_string();
        let mut env: HashMap<String, String> = [
            ("HOME", self.account.home_str().to_string()),
            ("USER", self.account.user.clone()),
            ("LOGNAME", self.account.user.clone()),
            ("SHELL", shell.to_string()),
            ("PATH", self.path.clone()),
            ("TERM", TERM.to_string()),
            ("COLORTERM", COLORTERM.to_string()),
            ("WINDOWID", pane.clone()),
            ("ALACRITTY_WINDOW_ID", pane),
            ("LANG", self.lang.clone()),
            ("TMPDIR", self.tmpdir.clone()),
            ("PLEXI_SOCKET", self.socket.clone()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
        if let Some(routing) = &self.routing {
            env.insert("PLEXI_CHANNEL".into(), routing.clone());
        }
        if let Some(sock) = self.ssh_auth_sock.as_ref().filter(|_| desktop) {
            env.insert("SSH_AUTH_SOCK".into(), sock.clone());
        }
        for (key, value) in &self.xdg {
            env.insert((*key).to_string(), value.clone());
        }
        env
    }

    /// The base a test host starts with: [`Self::fixture`] on Unix; none on
    /// Windows, whose panes do not use one yet (stint 1688).
    #[cfg(test)]
    pub(crate) fn test_base() -> Option<Arc<Self>> {
        #[cfg(unix)]
        {
            Some(Arc::new(Self::fixture()))
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// A base for tests: the real account, a fixed PATH, no agent socket.
    #[cfg(all(test, unix))]
    pub(crate) fn fixture() -> Self {
        Self {
            account: Account::current().expect("test account"),
            path: "/usr/bin:/bin:/usr/sbin:/sbin".into(),
            lang: FALLBACK_LOCALE.into(),
            tmpdir: "/tmp".into(),
            routing: None,
            socket: crate::platform::ipc::endpoint(),
            ssh_auth_sock: None,
            xdg: Vec::new(),
        }
    }
}

#[cfg(all(test, unix))]
impl PaneBase {
    /// [`Self::fixture`] with every optional variable present.
    pub(crate) fn fixture_with_every_variable() -> Self {
        Self {
            routing: Some("t1682".into()),
            ssh_auth_sock: Some("/tmp/agent-1682.sock".into()),
            xdg: [
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
            ]
            .into_iter()
            .map(|key| (key, "/tmp/xdg-1682".to_string()))
            .collect(),
            ..Self::fixture()
        }
    }
}

static BASE: OnceLock<Arc<PaneBase>> = OnceLock::new();
/// Host environment entries present at launch, counted (never read) once.
static INHERITED: OnceLock<usize> = OnceLock::new();

/// Resolve and install this host's pane base. Called once from `main` for a
/// host launch, right after the profile is installed and before logging,
/// profile access or the login-shell probes can change the process
/// environment. Counts the inherited entries it ignores for the startup
/// trace ([`log_installed`]).
#[cfg(unix)]
pub(crate) fn install_for_host(account: Account) -> Result<(), String> {
    let _ = INHERITED.set(std::env::vars_os().count());
    let base = PaneBase::for_host(account)?;
    BASE.set(Arc::new(base))
        .map_err(|_| "pane base: already installed".to_string())
}

/// The installed base; `None` outside a host process.
pub(crate) fn installed() -> Option<Arc<PaneBase>> {
    BASE.get().cloned()
}

/// Trace the installed base once the logger is up: sources and counts, no
/// values.
pub(crate) fn log_installed() {
    let Some(base) = BASE.get() else {
        log::info!("pane_env: no pane base installed");
        return;
    };
    log::info!(
        "pane_env: base from account {} (home {}, shell {}), PATH of {} dirs, LANG={}, routing={:?}, socket={}, ssh_agent={}, xdg={}; {} inherited host environment entries ignored",
        base.account.user,
        base.account.home.display(),
        base.account.shell,
        base.path.split(':').count(),
        base.lang,
        base.routing,
        base.socket,
        base.ssh_auth_sock.is_some(),
        base.xdg.len(),
        INHERITED.get().copied().unwrap_or(0),
    );
}

/// The installed CLI directory from a verified receipt: the executable's
/// own installation, else its channel's, looked up under the account home.
fn receipt_bin_dir(exe: &Path, home: &Path) -> Option<PathBuf> {
    #[cfg(not(windows))]
    {
        use plexi_distribution::transaction::{
            account_root, find_installation_in, installed_for_executable_in,
        };
        let root = account_root(home);
        let channel = exe
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(crate::config::channel_from_basename)
            .unwrap_or("stable")
            .to_string();
        let receipt = installed_for_executable_in(exe, &root).and_then(|found| match found {
            Some(receipt) => Ok(Some(receipt)),
            None => find_installation_in(&root, &channel),
        });
        match receipt {
            Ok(receipt) => receipt.map(|receipt| receipt.bin_dir),
            Err(error) => {
                log::warn!(
                    "pane_env: no install receipt for {}: {error}",
                    exe.display()
                );
                None
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = (exe, home);
        None
    }
}

/// Ordered, deduplicated absolute directories: the receipt's CLI directory,
/// `~/.local/bin`, the executable's directory when it holds the CLI,
/// package-manager prefixes, then system directories.
pub(crate) fn pane_path(home: &Path, receipt_bin: Option<&Path>, exe: &Path) -> String {
    let cli_dir = exe.parent().filter(|dir| {
        exe.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| dir.join(cli_command_name(name)).is_file())
    });
    let candidates = receipt_bin
        .map(Path::to_path_buf)
        .into_iter()
        .chain([home.join(".local/bin")])
        .chain(cli_dir.map(Path::to_path_buf))
        .chain(PACKAGE_DIRS.iter().chain(SYSTEM_DIRS).map(PathBuf::from));
    let mut dirs: Vec<String> = Vec::new();
    for dir in candidates {
        let Some(text) = dir.to_str() else { continue };
        if text.is_empty() || !dir.is_absolute() || text.contains(':') {
            continue;
        }
        if !dirs.iter().any(|seen| seen == text) {
            dirs.push(text.to_string());
        }
    }
    dirs.join(":")
}

/// The CLI command an executable named `exe_name` installs:
/// `plexi-alpha` for `plexi-alpha`, `plexi` for `plexi`.
fn cli_command_name(exe_name: &str) -> String {
    match crate::config::channel_from_basename(exe_name) {
        Some(channel) => plexi_distribution::release::command_name(channel),
        None => "plexi".to_string(),
    }
}

/// The first of `candidates` the C library can load, checked by explicit
/// name (never `""`, which would consult the environment).
fn available_locale(candidates: &[&str]) -> String {
    candidates
        .iter()
        .find(|name| locale_available(name))
        .copied()
        .unwrap_or(FALLBACK_LOCALE)
        .to_string()
}

#[cfg(unix)]
fn locale_available(name: &str) -> bool {
    let Ok(name) = std::ffi::CString::new(name) else {
        return false;
    };
    // SAFETY: `name` is a valid C string; a non-null locale is freed at once.
    unsafe {
        let locale = libc::newlocale(libc::LC_ALL_MASK, name.as_ptr(), std::ptr::null_mut());
        if locale.is_null() {
            return false;
        }
        libc::freelocale(locale);
        true
    }
}

#[cfg(not(unix))]
fn locale_available(_name: &str) -> bool {
    false
}

/// The account's temporary directory from the OS, not from an inherited
/// `TMPDIR` (`std::env::temp_dir` reads that).
fn account_tmpdir() -> String {
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: `buf` is writable for its full length.
        let len = unsafe {
            libc::confstr(
                libc::_CS_DARWIN_USER_TEMP_DIR,
                buf.as_mut_ptr().cast(),
                buf.len(),
            )
        };
        if len > 0 && len <= buf.len() {
            buf.truncate(len - 1);
            if let Ok(dir) = String::from_utf8(buf) {
                if Path::new(&dir).is_absolute() {
                    return dir;
                }
            }
        }
        log::warn!("pane_env: no per-user temporary directory from confstr; using /tmp");
    }
    "/tmp".to_string()
}

/// A signing-agent socket from a trusted source, validated as a socket the
/// account owns. An inherited `SSH_AUTH_SOCK` is not a source: a path that
/// happens to exist carries no authority.
fn trusted_ssh_agent() -> Option<String> {
    let candidate = os_agent_socket()?;
    match validate_owned_socket(Path::new(&candidate)) {
        Ok(()) => Some(candidate),
        Err(error) => {
            log::info!("pane_env: SSH agent socket {candidate} refused: {error}");
            None
        }
    }
}

/// macOS: the agent launchd provides to the account's GUI session.
#[cfg(target_os = "macos")]
fn os_agent_socket() -> Option<String> {
    let output = plexi_distribution::bounded::Helper::new("/bin/launchctl")
        .args(["getenv", "SSH_AUTH_SOCK"])
        .run(
            "launchctl getenv SSH_AUTH_SOCK",
            std::time::Duration::from_secs(5),
        );
    match output {
        Ok(output) => String::from_utf8(output.stdout)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
        Err(error) => {
            log::info!("pane_env: launchd has no SSH agent: {error}");
            None
        }
    }
}

/// Linux: a session agent inside the verified runtime directory.
#[cfg(target_os = "linux")]
fn os_agent_socket() -> Option<String> {
    let runtime = verified_runtime_dir()?;
    [
        "ssh-agent.socket",
        "gcr/ssh",
        "keyring/ssh",
        "openssh_agent",
    ]
    .iter()
    .map(|name| runtime.join(name))
    .find(|path| validate_owned_socket(path).is_ok())
    .and_then(|path| path.to_str().map(str::to_string))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn os_agent_socket() -> Option<String> {
    None
}

#[cfg(unix)]
fn validate_owned_socket(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    if !path.is_absolute() {
        return Err("not an absolute path".into());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !meta.file_type().is_socket() {
        return Err("not a Unix socket".into());
    }
    // SAFETY: getuid cannot fail.
    if meta.uid() != unsafe { libc::getuid() } {
        return Err("owned by another user".into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owned_socket(_path: &Path) -> Result<(), String> {
    Err("unsupported platform".into())
}

/// `/run/user/<uid>` when it is a directory the account owns with mode 0700.
#[cfg(target_os = "linux")]
fn verified_runtime_dir() -> Option<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(format!("/run/user/{uid}"));
    let meta = std::fs::symlink_metadata(&dir).ok()?;
    (meta.is_dir() && meta.uid() == uid && meta.permissions().mode() & 0o777 == 0o700)
        .then_some(dir)
}

/// Linux XDG base directories at their account-home defaults, plus a
/// verified runtime directory. Display, Wayland, Xauthority, D-Bus and
/// session overrides stay unset until a trusted desktop-session source
/// exists for them.
fn xdg_dirs(home: &Path) -> Vec<(&'static str, String)> {
    #[cfg(target_os = "linux")]
    {
        let mut dirs: Vec<(&'static str, String)> = [
            ("XDG_CONFIG_HOME", ".config"),
            ("XDG_DATA_HOME", ".local/share"),
            ("XDG_CACHE_HOME", ".cache"),
            ("XDG_STATE_HOME", ".local/state"),
        ]
        .into_iter()
        .filter_map(|(key, rel)| home.join(rel).to_str().map(|dir| (key, dir.to_string())))
        .collect();
        if let Some(runtime) =
            verified_runtime_dir().and_then(|dir| dir.to_str().map(str::to_string))
        {
            dirs.push(("XDG_RUNTIME_DIR", runtime));
        }
        dirs
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = home;
        Vec::new()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn pane_path_is_ordered_deduplicated_and_absolute() {
        let home = Path::new("/home/u");
        let path = pane_path(
            home,
            Some(Path::new("/home/u/.local/bin")),
            Path::new("/nonexistent-1682/plexi"),
        );
        let dirs: Vec<&str> = path.split(':').collect();
        assert_eq!(dirs[0], "/home/u/.local/bin");
        assert_eq!(
            dirs.iter().filter(|d| **d == "/home/u/.local/bin").count(),
            1
        );
        assert!(dirs.ends_with(&["/usr/bin", "/bin", "/usr/sbin", "/sbin"]));
        assert!(dirs.iter().all(|d| d.starts_with('/')));
        // A relative or empty receipt directory is ignored.
        let path = pane_path(home, Some(Path::new("relative/bin")), Path::new("/x/plexi"));
        assert!(!path.split(':').any(|d| d == "relative/bin" || d.is_empty()));
    }

    #[test]
    fn pane_path_includes_the_executables_cli_dir_only_when_it_holds_the_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("plexi-pr-1682");
        let dir = tmp.path().to_str().unwrap().to_string();
        assert!(!pane_path(Path::new("/h"), None, &exe)
            .split(':')
            .any(|d| d == dir));
        std::fs::write(tmp.path().join("plexi-pr-1682"), "").unwrap();
        assert!(pane_path(Path::new("/h"), None, &exe)
            .split(':')
            .any(|d| d == dir));
    }

    #[test]
    fn locale_is_chosen_by_explicit_name() {
        assert_eq!(available_locale(&["no_SUCH.locale-1682"]), FALLBACK_LOCALE);
        assert_eq!(available_locale(&[]), FALLBACK_LOCALE);
        assert_eq!(available_locale(&["C"]), "C");
    }

    #[test]
    fn a_regular_file_is_not_an_agent_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("agent.sock");
        std::fs::write(&file, "").unwrap();
        assert!(validate_owned_socket(&file).is_err());
        assert!(validate_owned_socket(Path::new("relative.sock")).is_err());
        let socket = tmp.path().join("real.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(validate_owned_socket(&socket).is_ok());
    }
}
