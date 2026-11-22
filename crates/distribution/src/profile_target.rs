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

use crate::host_presence::{endpoint_in, ensure_real_profile_dir, profile_dir_name};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// The option that names the root, accepted by `plexi` at every command
/// level and by the installer.
pub const PROFILE_ROOT_FLAG: &str = "--profile-root";

/// Longest accepted profile name, in bytes.
pub const PROFILE_NAME_MAX: usize = 64;

/// The `--profile` name grammar, `[A-Za-z0-9][A-Za-z0-9-]{0,63}`: one path
/// component that cannot traverse, alias or nest. Shared by the CLI and the
/// installer's restart.
pub fn is_profile_name(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= PROFILE_NAME_MAX
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
}

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
        // Rebuilt from components: a trailing slash would make lstat follow a
        // link. Ancestors may be links (macOS /tmp and /var are); the root's
        // own last component may not, or it would name another directory.
        let normalized: PathBuf = path.components().collect();
        match std::fs::symlink_metadata(&normalized) {
            Ok(meta) if meta.file_type().is_symlink() => Err(format!(
                "--profile-root {shown} is a symbolic link; give the directory itself"
            )),
            Ok(meta) if meta.is_dir() => Ok(Self(normalized)),
            Ok(_) => Err(format!("--profile-root {shown} is not a directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("--profile-root {shown} does not exist"))
            }
            Err(error) => Err(format!(
                "--profile-root {shown} cannot be inspected: {error}"
            )),
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
    canonical_dir: PathBuf,
    account_rooted: bool,
}

impl ProfileTarget {
    /// The only way to get a target. `source` names the directory; `Pane`
    /// carries its directory name (`.plexi` or `.plexi-<name>`), already
    /// validated. Refuses a linked profile, `host` or spawn-queue directory
    /// and a linked endpoint, so no holder of a target has to remember to
    /// check it.
    pub fn resolve(
        root: PathBuf,
        root_source: RootSource,
        source: ProfileSource,
        build_channel: Option<String>,
    ) -> Result<Self, String> {
        let dir_name = match &source {
            ProfileSource::Named(name) => format!(".plexi-{name}"),
            ProfileSource::Channel(channel) => profile_dir_name(channel),
            ProfileSource::Pane(dir_name) => dir_name.clone(),
            ProfileSource::Default => ".plexi".to_string(),
        };
        ensure_real_profile_dir(&root.join(&dir_name))?;
        let canonical_root = canonical_or_given(&root);
        let account_rooted = root_source == RootSource::Account
            || account_home().is_ok_and(|home| canonical_or_given(&home) == canonical_root);
        Ok(Self {
            canonical_dir: canonical_root.join(&dir_name),
            account_rooted,
            root,
            root_source,
            dir_name,
            source,
            build_channel,
        })
    }

    /// The profile an installer acts on for `channel`: under `root` when one
    /// was given, else under the account home.
    /// Without a root, an inherited private home is refused like the CLI
    /// refuses it ([`refuse_private_home`]).
    pub fn for_channel(root: Option<&ProfileRoot>, channel: &str) -> Result<Self, String> {
        Self::for_build(root, None, channel)
    }

    /// The profile a build of `channel` runs on: the named profile when one
    /// was selected (`--profile <name>`), else the channel's own, under
    /// `root` or the account home. An update restart relaunches exactly the
    /// selection the old host ran with.
    pub fn for_build(
        root: Option<&ProfileRoot>,
        named: Option<&str>,
        channel: &str,
    ) -> Result<Self, String> {
        if let Some(name) = named.filter(|name| !is_profile_name(name)) {
            return Err(format!(
                "invalid profile name {name:?}: use 1-{PROFILE_NAME_MAX} ASCII letters, digits or hyphens, starting with a letter or digit"
            ));
        }
        let (root, root_source) = match root {
            Some(root) => (root.path().to_path_buf(), RootSource::Explicit),
            None => {
                let home = account_home()?;
                refuse_private_home(std::env::var_os(HOME_VAR).as_deref(), &home)?;
                (home, RootSource::Account)
            }
        };
        let source = match named {
            Some(name) => ProfileSource::Named(name.to_string()),
            None => ProfileSource::Channel(channel.to_string()),
        };
        Self::resolve(root, root_source, source, Some(channel.to_string()))
    }

    /// The profile directory: config, plugins, host records, the endpoint.
    pub fn dir(&self) -> PathBuf {
        self.root.join(&self.dir_name)
    }

    /// The profile directory with every link in its root resolved: the
    /// identity of this profile, whatever spelling reached it.
    pub fn canonical_dir(&self) -> &Path {
        &self.canonical_dir
    }

    /// True when this profile lives directly under the account's home.
    pub fn account_rooted(&self) -> bool {
        self.account_rooted
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
        self.explicit_root()
            .map(|root| ProfileRoot(root.to_path_buf()))
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

/// `path` with its links resolved; as given when it does not exist yet.
fn canonical_or_given(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
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
    use std::fs;

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
            (
                dir.path().join("missing").into_os_string(),
                "does not exist",
            ),
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
        let named = ProfileTarget::resolve(
            root.into(),
            RootSource::Explicit,
            ProfileSource::Named("t1695".into()),
            Some("pr-1".into()),
        )
        .unwrap();
        assert_eq!(named.dir(), root.join(".plexi-t1695"));
        assert_eq!(named.routing_suffix(), Some("t1695"));
        assert_eq!(named.build_channel(), Some("pr-1"));
        assert!(named.is_addressed());
        assert_eq!(
            named.forward_args(),
            ["--profile-root", "/r", "--profile", "t1695"].map(OsString::from)
        );
        let stable = ProfileTarget::resolve(
            root.into(),
            RootSource::Account,
            ProfileSource::Channel("stable".into()),
            None,
        )
        .unwrap();
        assert_eq!(stable.dir_name(), ".plexi");
        assert_eq!(stable.routing_suffix(), None);
        assert!(stable.forward_args().is_empty());
        let pane = ProfileTarget::resolve(
            root.into(),
            RootSource::Pane,
            ProfileSource::Pane(".plexi-beta".into()),
            None,
        )
        .unwrap();
        assert!(!pane.is_addressed());
        assert_eq!(pane.dir(), root.join(".plexi-beta"));
    }

    /// Design 1695 (1): no target exists for a linked root, profile, `host`
    /// dir, spawn-queue dir or endpoint, whichever role asks. A symlinked
    /// ancestor stays legal.
    #[cfg(unix)]
    #[test]
    fn no_target_exists_for_a_linked_profile() {
        use std::os::unix::fs::symlink;
        let base = tempfile::tempdir().unwrap();
        let base_path = base.path().canonicalize().unwrap();
        let other = base_path.join("other");
        fs_mkdir(&other);
        let resolve = |root: &Path| {
            ProfileTarget::resolve(
                root.to_path_buf(),
                RootSource::Explicit,
                ProfileSource::Named("x".into()),
                None,
            )
        };

        let linked_root = base_path.join("linked-root");
        symlink(&other, &linked_root).unwrap();
        for raw in [
            linked_root.clone(),
            PathBuf::from(format!("{}/", linked_root.display())),
        ] {
            let error = ProfileRoot::parse(raw.as_os_str()).unwrap_err();
            assert!(error.contains("symbolic link"), "{error}");
        }
        let through = linked_root.join("real");
        fs_mkdir(&through);
        assert!(
            ProfileRoot::parse(through.as_os_str()).is_ok(),
            "a symlinked ancestor is legal"
        );

        let profile_link = base_path.join("r1");
        fs_mkdir(&profile_link);
        symlink(&other, profile_link.join(".plexi-x")).unwrap();
        assert!(resolve(&profile_link).is_err());

        for inner in ["host", "spawn-queue"] {
            let root = base_path.join(format!("r-{inner}"));
            fs_mkdir(&root.join(".plexi-x"));
            symlink(&other, root.join(".plexi-x").join(inner)).unwrap();
            assert!(resolve(&root).is_err(), "{inner}");
        }

        let hard = base_path.join("r-hard");
        fs_mkdir(&hard.join(".plexi-x"));
        let listener_dir = base_path.join("listener");
        fs_mkdir(&listener_dir);
        let socket = listener_dir.join("notify.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        fs::hard_link(&socket, hard.join(".plexi-x/notify.sock")).unwrap();
        assert!(resolve(&hard).is_err());
    }

    #[cfg(unix)]
    fn fs_mkdir(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    /// Design 1695: `account_rooted` compares canonically, so a link to the
    /// account home is the account home and a scratch root is not.
    #[cfg(unix)]
    #[test]
    fn a_target_knows_whether_it_is_account_rooted() {
        let home = account_home().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let target = |root: &Path, source| {
            ProfileTarget::resolve(
                root.to_path_buf(),
                source,
                ProfileSource::Named("t1695-absent".into()),
                None,
            )
            .unwrap()
        };
        assert!(target(&home, RootSource::Account).account_rooted());
        assert!(target(&home, RootSource::Explicit).account_rooted());
        let spelled = scratch.path().join("home-link");
        std::os::unix::fs::symlink(&home, &spelled).unwrap();
        assert!(target(&spelled, RootSource::Pane).account_rooted());
        let scratch_target = target(scratch.path(), RootSource::Explicit);
        assert!(!scratch_target.account_rooted());
        assert_eq!(
            scratch_target.canonical_dir(),
            scratch
                .path()
                .canonicalize()
                .unwrap()
                .join(".plexi-t1695-absent")
        );
    }

    #[test]
    fn a_private_home_is_refused_without_an_explicit_root() {
        let account = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        assert!(refuse_private_home(None, account.path()).is_ok());
        assert!(refuse_private_home(Some(OsStr::new("")), account.path()).is_ok());
        assert!(refuse_private_home(Some(account.path().as_os_str()), account.path()).is_ok());
        let error =
            refuse_private_home(Some(private.path().as_os_str()), account.path()).unwrap_err();
        assert!(error.contains(PROFILE_ROOT_FLAG), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn account_home_comes_from_passwd_not_the_environment() {
        let home = account_home().unwrap();
        assert!(home.is_absolute());
    }
}
