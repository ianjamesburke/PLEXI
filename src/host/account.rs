//! The Unix account this process runs as, read from the passwd database
//! (stint 1682). Never from the process environment: `HOME`, `USER` and
//! `SHELL` are whatever launched the host, which can be another pane.

use std::path::{Path, PathBuf};

/// The running account's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Account {
    pub(crate) user: String,
    pub(crate) home: PathBuf,
    /// `pw_shell` when it names an absolute executable, otherwise `/bin/sh`.
    pub(crate) shell: String,
}

/// The shell every account falls back to.
const FALLBACK_SHELL: &str = "/bin/sh";

impl Account {
    /// The account of the real user id.
    #[cfg(unix)]
    pub(crate) fn current() -> Result<Self, String> {
        let entry = passwd_entry()?;
        let user = entry
            .name
            .into_string()
            .map_err(|_| format!("account uid {}: user name is not UTF-8", entry.uid))?;
        if user.is_empty() {
            return Err(format!(
                "account uid {}: passwd entry has no user name",
                entry.uid
            ));
        }
        if entry.dir.as_os_str().is_empty() || !Path::new(&entry.dir).is_absolute() {
            return Err(format!(
                "account {user}: passwd home {:?} is not an absolute path",
                entry.dir
            ));
        }
        let home = PathBuf::from(entry.dir);
        if home.to_str().is_none() {
            return Err(format!("account {user}: home directory is not UTF-8"));
        }
        let shell = resolve_shell(entry.shell.to_str());
        Ok(Self { user, home, shell })
    }

    pub(crate) fn home_str(&self) -> &str {
        self.home
            .to_str()
            .expect("Account::current admits only a UTF-8 home")
    }
}

/// The account's home directory, the root of every default the host
/// generates: context roots and names, pane working directories and profile
/// locations. Never `$HOME`, which a host started from a pane inherits from
/// that pane (stint 1682). Read once; `None` when the lookup fails, which is
/// logged.
pub(crate) fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        static HOME: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        HOME.get_or_init(|| match Account::current() {
            Ok(account) => Some(account.home),
            Err(error) => {
                log::error!("account: home directory lookup failed: {error}");
                None
            }
        })
        .clone()
    }
    #[cfg(not(unix))]
    {
        // Windows reads the profile folder from the shell's known-folder
        // API, not from the environment (stint 1688 owns Windows panes).
        dirs::home_dir()
    }
}

/// `pw_shell` when it names an absolute executable file, otherwise
/// `/bin/sh`.
pub(crate) fn resolve_shell(pw_shell: Option<&str>) -> String {
    pw_shell
        .filter(|shell| is_absolute_executable(Path::new(shell)))
        .unwrap_or(FALLBACK_SHELL)
        .to_string()
}

#[cfg(unix)]
fn is_absolute_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_absolute()
        && std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_absolute_executable(path: &Path) -> bool {
    path.is_absolute() && path.is_file()
}

#[cfg(unix)]
struct PasswdEntry {
    uid: libc::uid_t,
    name: std::ffi::OsString,
    dir: std::ffi::OsString,
    shell: std::ffi::OsString,
}

/// `getpwuid_r` for the real uid, every field copied out of the buffer.
#[cfg(unix)]
fn passwd_entry() -> Result<PasswdEntry, String> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let mut buf = vec![0u8; 16 * 1024];
    // SAFETY: getpwuid_r writes into `pwd` and `buf`, which outlive every
    // pointer read below; each field is copied before either is dropped.
    unsafe {
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::zeroed();
        let mut result = std::ptr::null_mut();
        let rc = libc::getpwuid_r(
            uid,
            pwd.as_mut_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        );
        if rc != 0 {
            return Err(format!(
                "account uid {uid}: getpwuid_r failed: {}",
                std::io::Error::from_raw_os_error(rc)
            ));
        }
        if result.is_null() {
            return Err(format!("account uid {uid}: no passwd entry"));
        }
        let pwd = pwd.assume_init();
        let field = |ptr: *const libc::c_char| {
            if ptr.is_null() {
                std::ffi::OsString::new()
            } else {
                std::ffi::OsString::from_vec(CStr::from_ptr(ptr).to_bytes().to_vec())
            }
        };
        Ok(PasswdEntry {
            uid,
            name: field(pwd.pw_name),
            dir: field(pwd.pw_dir),
            shell: field(pwd.pw_shell),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Stint 1682: the account comes from the passwd database, so it holds
    /// however the process environment changes.
    #[test]
    fn current_account_is_the_passwd_entry() {
        let account = Account::current().expect("passwd entry");
        assert!(account.home.is_absolute());
        assert!(!account.user.is_empty());
        assert!(Path::new(&account.shell).is_absolute());
    }

    #[test]
    fn shell_falls_back_when_pw_shell_is_not_an_absolute_executable() {
        assert_eq!(resolve_shell(Some("/bin/sh")), "/bin/sh");
        assert_eq!(resolve_shell(Some("zsh")), FALLBACK_SHELL);
        assert_eq!(
            resolve_shell(Some("/nonexistent/shell-1682")),
            FALLBACK_SHELL
        );
        assert_eq!(resolve_shell(Some("")), FALLBACK_SHELL);
        assert_eq!(resolve_shell(None), FALLBACK_SHELL);
        // A directory is not an executable.
        assert_eq!(resolve_shell(Some("/bin")), FALLBACK_SHELL);
    }
}
