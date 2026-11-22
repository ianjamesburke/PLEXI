//! The one host selection (stints 1695 and 1657): the profile directory a
//! Plexi process works in, the endpoint its host listens on, and the build
//! it runs. A process chooses it once, from explicit inputs, and never from
//! the `HOME` or `PLEXI_CHANNEL` it inherited: a process started from a pane
//! inherits that pane's values, which name the launching host, not this one.
//!
//! The root is the parent of `.plexi[-name]`: an explicit, validated
//! `--profile-root <absolute dir>`, else the account home from the passwd
//! database. Shared by the host, the CLI and the installer so all three agree
//! on one directory and one endpoint.

use crate::host_presence::{endpoint_in, profile_dir_name};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// The option that names the root, accepted by `plexi` at every command
/// level and by the installer.
pub const PROFILE_ROOT_FLAG: &str = "--profile-root";

/// A validated explicit `--profile-root`: an absolute path with no `.` or
/// `..` component that names an existing directory. Validated before any
/// profile is read, connected to, signalled or deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRoot(PathBuf);

impl ProfileRoot {
    /// Parse one `--profile-root` value. The error is the user-facing reason.
    pub fn parse(raw: &OsStr) -> Result<Self, String> {
        let path = Path::new(raw);
        let shown = path.display();
        if raw.is_empty() {
            return Err("--profile-root needs an absolute directory".to_string());
        }
        if !path.is_absolute() {
            return Err(format!(
                "--profile-root {shown} is not an absolute path; give the absolute directory that holds the .plexi profiles"
            ));
        }
        if path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        {
            return Err(format!(
                "--profile-root {shown} contains a . or .. component; give a normalized absolute directory"
            ));
        }
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => Ok(Self(path.to_path_buf())),
            Ok(_) => Err(format!("--profile-root {shown} is not a directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("--profile-root {shown} does not exist"))
            }
            Err(error) => Err(format!("--profile-root {shown} cannot be inspected: {error}")),
        }
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Where a target's root came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootSource {
    /// `--profile-root`.
    Explicit,
    /// The account home (passwd), the default.
    Account,
    /// The directory holding the calling pane's endpoint.
    Pane,
}

/// What chose the profile directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSource {
    /// An explicit `--profile <name>`: `.plexi-<name>`.
    Named(String),
    /// The executable's channel (`plexi-alpha` → `.plexi-alpha`), or the
    /// channel an installer acts on.
    Channel(String),
    /// A bare client's surrounding pane (`PLEXI_SOCKET`, `PLEXI_CHANNEL`).
    /// Only ordinary commands ever select this.
    Pane(String),
    /// Nothing named one: `.plexi`.
    Default,
}

/// One immutable host selection: profile directory, listener endpoint and
/// executable build identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTarget {
    root: PathBuf,
    root_source: RootSource,
    dir_name: String,
    source: ProfileSource,
    build_channel: Option<String>,
}

impl ProfileTarget {
    /// Assemble a target. `source` names the directory; `Pane` carries its
    /// directory name (`.plexi` or `.plexi-<name>`), already validated.
    pub fn new(
        root: PathBuf,
        root_source: RootSource,
        source: ProfileSource,
        build_channel: Option<String>,
    ) -> Self {
        let dir_name = match &source {
            ProfileSource::Named(name) => format!(".plexi-{name}"),
            ProfileSource::Channel(channel) => profile_dir_name(channel),
            ProfileSource::Pane(dir_name) => dir_name.clone(),
            ProfileSource::Default => ".plexi".to_string(),
        };
        Self {
            root,
            root_source,
            dir_name,
            source,
            build_channel,
        }
    }

    /// The profile an installer acts on for `channel`: under `root` when one
    /// was given, else under the account home.
    pub fn for_channel(root: Option<&ProfileRoot>, channel: &str) -> Result<Self, String> {
        let (root, root_source) = match root {
            Some(root) => (root.path().to_path_buf(), RootSource::Explicit),
            None => (account_home()?, RootSource::Account),
        };
        Ok(Self::new(
            root,
            root_source,
            ProfileSource::Channel(channel.to_string()),
            Some(channel.to_string()),
        ))
    }

    /// The profile directory: config, plugins, host records, the endpoint.
    pub fn dir(&self) -> PathBuf {
        self.root.join(&self.dir_name)
    }

    /// The endpoint this profile's host listens on.
    pub fn endpoint(&self) -> PathBuf {
        endpoint_in(&self.dir())
    }

    pub fn dir_name(&self) -> &str {
        &self.dir_name
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn root_source(&self) -> RootSource {
        self.root_source
    }

    pub fn source(&self) -> &ProfileSource {
        &self.source
    }

    /// The channel of the executable that runs this selection (`None` for a
    /// bare `plexi`). Build identity, never profile routing.
    pub fn build_channel(&self) -> Option<&str> {
        self.build_channel.as_deref()
    }

    /// What a bare client inside one of this host's panes routes by
    /// (`PLEXI_CHANNEL`): the profile suffix, none for `.plexi`.
    pub fn routing_suffix(&self) -> Option<&str> {
        self.dir_name
            .strip_prefix(".plexi-")
            .filter(|suffix| !suffix.is_empty())
    }

    /// The explicit `--profile` name, if one chose this target.
    pub fn named_profile(&self) -> Option<&str> {
        match &self.source {
            ProfileSource::Named(name) => Some(name),
            _ => None,
        }
    }

    /// The explicit `--profile-root`, if one was given.
    pub fn explicit_root(&self) -> Option<&Path> {
        (self.root_source == RootSource::Explicit).then_some(self.root.as_path())
    }

    /// The explicit root as a [`ProfileRoot`], for an installer step that
    /// must act on the same profile root as this process.
    pub fn profile_root(&self) -> Option<ProfileRoot> {
        self.explicit_root().map(|root| ProfileRoot(root.to_path_buf()))
    }

    /// True when the invocation itself names this target (explicit profile,
    /// explicit root or a channel executable), so its endpoint outranks an
    /// inherited pane socket.
    pub fn is_addressed(&self) -> bool {
        self.root_source == RootSource::Explicit
            || matches!(
                self.source,
                ProfileSource::Named(_) | ProfileSource::Channel(_)
            )
    }

    /// The arguments a child Plexi process takes to select this same target:
    /// `--profile-root <root>` when explicit and `--profile <name>` when
    /// named. The executable supplies the rest.
    pub fn forward_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if let Some(root) = self.explicit_root() {
            args.push(OsString::from(PROFILE_ROOT_FLAG));
            args.push(root.as_os_str().to_os_string());
        }
        if let Some(name) = self.named_profile() {
            args.push(OsString::from("--profile"));
            args.push(OsString::from(name));
        }
        args
    }
}

/// The account's home directory, read from the passwd database on Unix and
/// the known-folder API on Windows, never from `HOME` or `USERPROFILE`.
/// Resolved once per process.
pub fn account_home() -> Result<PathBuf, String> {
    static HOME: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let home = lookup_account_home();
        match &home {
            Ok(home) => log::info!("profile_target: account home {}", home.display()),
            Err(error) => log::error!("profile_target: account home lookup failed: {error}"),
        }
        home
    })
    .clone()
}

#[cfg(unix)]
fn lookup_account_home() -> Result<PathBuf, String> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let mut buf = vec![0u8; 16 * 1024];
    // SAFETY: getpwuid_r writes into `pwd` and `buf`, which outlive every
    // pointer read below; the home field is copied before either is dropped.
    let dir = unsafe {
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
        if pwd.pw_dir.is_null() {
            return Err(format!("account uid {uid}: passwd entry has no home"));
        }
        OsString::from_vec(CStr::from_ptr(pwd.pw_dir).to_bytes().to_vec())
    };
    let home = PathBuf::from(dir);
    if !home.is_absolute() {
        return Err(format!(
            "account uid {uid}: passwd home {} is not an absolute path",
            home.display()
        ));
    }
    Ok(home)
}

#[cfg(windows)]
fn lookup_account_home() -> Result<PathBuf, String> {
    // `dirs` reads FOLDERID_Profile through SHGetKnownFolderPath, not the
    // USERPROFILE variable.
    dirs::home_dir().ok_or_else(|| "the account profile folder could not be resolved".to_string())
}

/// The variable a launcher or script uses to point a process at a private
/// home: `HOME` on Unix, `USERPROFILE` on Windows.
#[cfg(unix)]
pub const HOME_VAR: &str = "HOME";
#[cfg(windows)]
pub const HOME_VAR: &str = "USERPROFILE";

/// Refuse an inherited private home that differs from the account home when
/// no explicit root was given. Before stint 1695 such a home chose where
/// profiles lived; now it does not, so a caller that still relies on it would
/// reach the account's real profiles. It must say `--profile-root` instead.
pub fn refuse_private_home(inherited: Option<&OsStr>, account: &Path) -> Result<(), String> {
    let Some(inherited) = inherited.filter(|home| !home.is_empty()) else {
        return Ok(());
    };
    let inherited = Path::new(inherited);
    if inherited == account {
        return Ok(());
    }
    let same = match (inherited.canonicalize(), account.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Ok(());
    }
    Err(format!(
        "{HOME_VAR} is {} but this account's home is {}; {HOME_VAR} no longer chooses where Plexi profiles live. Pass {PROFILE_ROOT_FLAG} <absolute dir> to use profiles under another directory, or run with the account's own {HOME_VAR}",
        inherited.display(),
        account.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_root_accepts_only_an_absolute_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = ProfileRoot::parse(dir.path().as_os_str()).unwrap();
        assert_eq!(root.path(), dir.path());
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        for (raw, reason) in [
            (OsString::from("relative/root"), "not an absolute path"),
            (OsString::new(), "needs an absolute directory"),
            (dir.path().join("missing").into_os_string(), "does not exist"),
            (file.into_os_string(), "is not a directory"),
            (dir.path().join("..").into_os_string(), ". or .."),
        ] {
            let error = ProfileRoot::parse(&raw).unwrap_err();
            assert!(error.contains(reason), "{raw:?}: {error}");
        }
    }

    #[test]
    fn a_target_names_one_directory_endpoint_and_forwarding() {
        let root = Path::new("/r");
        let named = ProfileTarget::new(
            root.into(),
            RootSource::Explicit,
            ProfileSource::Named("t1695".into()),
            Some("pr-1".into()),
        );
        assert_eq!(named.dir(), root.join(".plexi-t1695"));
        assert_eq!(named.routing_suffix(), Some("t1695"));
        assert_eq!(named.build_channel(), Some("pr-1"));
        assert!(named.is_addressed());
        assert_eq!(
            named.forward_args(),
            ["--profile-root", "/r", "--profile", "t1695"].map(OsString::from)
        );
        let stable = ProfileTarget::new(
            root.into(),
            RootSource::Account,
            ProfileSource::Channel("stable".into()),
            None,
        );
        assert_eq!(stable.dir_name(), ".plexi");
        assert_eq!(stable.routing_suffix(), None);
        assert!(stable.forward_args().is_empty());
        let pane = ProfileTarget::new(
            root.into(),
            RootSource::Pane,
            ProfileSource::Pane(".plexi-beta".into()),
            None,
        );
        assert!(!pane.is_addressed());
        assert_eq!(pane.dir(), root.join(".plexi-beta"));
    }

    #[test]
    fn a_private_home_is_refused_without_an_explicit_root() {
        let account = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        assert!(refuse_private_home(None, account.path()).is_ok());
        assert!(refuse_private_home(Some(OsStr::new("")), account.path()).is_ok());
        assert!(refuse_private_home(Some(account.path().as_os_str()), account.path()).is_ok());
        let error = refuse_private_home(Some(private.path().as_os_str()), account.path())
            .unwrap_err();
        assert!(error.contains(PROFILE_ROOT_FLAG), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn account_home_comes_from_passwd_not_the_environment() {
        let home = account_home().unwrap();
        assert!(home.is_absolute());
    }
}
