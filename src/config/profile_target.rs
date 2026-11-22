//! This process's one host selection (stints 1695 and 1657). `main` chooses
//! it once, from the validated `--profile` and `--profile-root`, the
//! executable's name and, for an ordinary command only, the pane it runs in.
//! Everything that names a profile directory or a host endpoint reads it from
//! here: `config_dir()`, the lifecycle verbs, uninstall, the command socket
//! and the values a host gives its panes.
//!
//! Precedence by role:
//!
//! | Role | Profile | Root |
//! |---|---|---|
//! | Host launch, lifecycle (`host start/stop/restart/status/log`, uninstall) | `--profile`, else the executable's channel, else `.plexi` | `--profile-root`, else the account home |
//! | Ordinary command | `--profile`; else the executable's channel unless a bare launcher delegated to it; else the pane (`PLEXI_CHANNEL`, `PLEXI_SOCKET`); else `.plexi` | `--profile-root`; else the pane endpoint's directory; else the account home |
//!
//! `HOME` and `PLEXI_CHANNEL` never route a host launch or a lifecycle verb.
//! A private `HOME` that differs from the account home is refused unless
//! `--profile-root` says where profiles live, so a script written for the
//! old `HOME`-rooted profiles fails instead of reaching the real ones.

use super::ProfileSelection;
pub use plexi_distribution::profile_target::{
    ProfileRoot, ProfileSource, ProfileTarget, RootSource,
};
use plexi_distribution::profile_target::{account_home, refuse_private_home, HOME_VAR};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

/// What the invocation does with its selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A host process (GUI launch, `host start` child, restart child).
    HostLaunch,
    /// `host start/stop/restart/status/log` and uninstall.
    Lifecycle,
    /// Every other command.
    Command,
}

/// The explicit selectors an invocation gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selectors {
    pub profile: ProfileSelection,
    pub root: Option<ProfileRoot>,
}

impl Selectors {
    pub fn none() -> Self {
        Self {
            profile: ProfileSelection::Default,
            root: None,
        }
    }
}

/// Inherited values a selection may consult, read once at startup. Only an
/// ordinary command routes by `socket` and `channel`; `home` is consulted only
/// to refuse a private one.
#[derive(Clone, Debug, Default)]
pub struct Ambient {
    pub home: Option<OsString>,
    pub socket: Option<OsString>,
    pub channel: Option<String>,
    /// A bare `plexi` launcher handed this command to this executable
    /// (`plexi_distribution::transaction::DELEGATED_VAR`): the executable's
    /// channel is the build it runs, not a profile it was asked for.
    pub delegated: bool,
}

impl Ambient {
    /// Read the inherited values and remove the launcher's delegation
    /// marker so nothing this process starts inherits it. Call once, during
    /// single-threaded startup.
    pub fn take() -> Self {
        let var = plexi_distribution::transaction::DELEGATED_VAR;
        let delegated = std::env::var_os(var).is_some_and(|value| value == "1");
        if std::env::var_os(var).is_some() {
            std::env::remove_var(var);
        }
        Self {
            home: std::env::var_os(HOME_VAR),
            socket: std::env::var_os("PLEXI_SOCKET").filter(|s| !s.is_empty()),
            channel: std::env::var("PLEXI_CHANNEL")
                .ok()
                .filter(|c| !c.is_empty()),
            delegated,
        }
    }
}

/// Choose the selection. Pure apart from the account home passed in and
/// existence checks a caller already made on `--profile-root`.
pub fn select(
    role: Role,
    selectors: &Selectors,
    executable: &str,
    account: Result<PathBuf, String>,
    ambient: &Ambient,
) -> Result<ProfileTarget, String> {
    let build_channel = match super::channel_from_basename(executable) {
        Some(channel) if super::profile_name::is_profile_name(channel) => Some(channel.to_string()),
        Some(channel) => {
            return Err(format!(
                "executable {executable:?} names channel {channel:?}, which is not a valid profile name; rename the executable"
            ))
        }
        None => None,
    };
    let named = match &selectors.profile {
        ProfileSelection::Named(name) => Some(name.as_str().to_string()),
        ProfileSelection::Default => None,
    };
    let implicit = role == Role::Command
        && named.is_none()
        && selectors.root.is_none()
        && (build_channel.is_none() || ambient.delegated);
    let account_root = |account: Result<PathBuf, String>| -> Result<PathBuf, String> {
        let home = account.map_err(|error| {
            format!("cannot resolve the account home ({error}); pass --profile-root <absolute dir>")
        })?;
        refuse_private_home(ambient.home.as_deref(), &home)?;
        Ok(home)
    };
    if implicit {
        if let Some((dir_name, pane_root)) = pane_profile(ambient)? {
            let (root, root_source) = match pane_root {
                Some(root) => (root, RootSource::Pane),
                None => (account_root(account)?, RootSource::Account),
            };
            return Ok(ProfileTarget::new(
                root,
                root_source,
                ProfileSource::Pane(dir_name),
                build_channel,
            ));
        }
    }
    let source = match (named, &build_channel) {
        (Some(name), _) => ProfileSource::Named(name),
        (None, Some(channel)) if !implicit => ProfileSource::Channel(channel.clone()),
        _ => ProfileSource::Default,
    };
    let (root, root_source) = match &selectors.root {
        Some(root) => (root.path().to_path_buf(), RootSource::Explicit),
        None => (account_root(account)?, RootSource::Account),
    };
    Ok(ProfileTarget::new(root, root_source, source, build_channel))
}

/// The surrounding pane's profile for a bare command: `PLEXI_CHANNEL` names
/// the directory when set, else the pane endpoint's own directory does. The
/// root is the endpoint's parent when the endpoint lives in that directory
/// (Unix), else `None` for the account home.
fn pane_profile(ambient: &Ambient) -> Result<Option<(String, Option<PathBuf>)>, String> {
    let from_channel = match ambient.channel.as_deref() {
        Some(channel) if super::profile_name::is_profile_name(channel) => {
            Some(plexi_distribution::host_presence::profile_dir_name(channel))
        }
        Some(channel) => {
            return Err(format!(
                "PLEXI_CHANNEL={channel:?} is not a valid profile name; unset it or pass --profile <name>"
            ))
        }
        None => None,
    };
    let from_socket = ambient.socket.as_deref().and_then(endpoint_profile);
    Ok(match (from_channel, from_socket) {
        (Some(dir), Some((socket_dir, root))) if dir == socket_dir => Some((dir, root)),
        (Some(dir), _) => Some((dir, None)),
        (None, Some((dir, root))) => Some((dir, root)),
        (None, None) => None,
    })
}

/// The profile directory an endpoint belongs to: `<root>/.plexi[-name]/notify.sock`
/// on Unix, `\\.\pipe\plexi[-name]` on Windows (whose root is the account home).
fn endpoint_profile(endpoint: &std::ffi::OsStr) -> Option<(String, Option<PathBuf>)> {
    let valid_dir = |name: &str| {
        name == ".plexi"
            || name
                .strip_prefix(".plexi-")
                .is_some_and(super::profile_name::is_profile_name)
    };
    #[cfg(unix)]
    {
        let path = std::path::Path::new(endpoint);
        if path.file_name()? != "notify.sock" {
            return None;
        }
        let dir = path.parent()?;
        let name = dir.file_name()?.to_str()?;
        let root = dir.parent()?;
        (valid_dir(name) && root.is_absolute()).then(|| (name.to_string(), Some(root.into())))
    }
    #[cfg(windows)]
    {
        let name = endpoint.to_str()?.strip_prefix(r"\\.\pipe\plexi")?;
        let dir = format!(".plexi{name}");
        valid_dir(&dir).then_some((dir, None))
    }
}

static TARGET: OnceLock<ProfileTarget> = OnceLock::new();

/// Install this process's selection. `main` calls it once, before any
/// profile access.
pub fn install(target: ProfileTarget) {
    if TARGET.set(target).is_err() {
        log::error!("profile_target: a selection was already installed; keeping the first");
    }
}

/// This process's selection. A read before `main` installed one (a unit
/// test, or a startup ordering bug) gets the executable's own profile under
/// the account home, read from no environment, and logs the miss.
pub fn profile_target() -> &'static ProfileTarget {
    TARGET.get_or_init(|| {
        #[cfg(not(test))]
        log::error!("profile_target: read before main installed a selection; using the executable's own profile");
        let executable = super::current_exe_basename();
        select(
            Role::Lifecycle,
            &Selectors::none(),
            &executable,
            Ok(account_home().unwrap_or_else(|_| PathBuf::from("/"))),
            &Ambient::default(),
        )
        .unwrap_or_else(|error| {
            log::error!("profile_target: fallback selection failed: {error}");
            ProfileTarget::new(
                PathBuf::from("/"),
                RootSource::Account,
                ProfileSource::Default,
                None,
            )
        })
    })
}

/// One line naming the selection, for startup and lifecycle traces.
pub fn describe(target: &ProfileTarget) -> String {
    format!(
        "dir={} source={:?} root_source={:?} endpoint={} build_channel={:?}",
        target.dir().display(),
        target.source(),
        target.root_source(),
        target.endpoint().display(),
        target.build_channel()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn named(name: &str) -> ProfileSelection {
        ProfileSelection::parse(name).unwrap()
    }

    fn sel(profile: ProfileSelection, root: Option<ProfileRoot>) -> Selectors {
        Selectors { profile, root }
    }

    fn poisoned() -> Ambient {
        Ambient {
            home: None,
            socket: Some("/poison/.plexi-beta/notify.sock".into()),
            channel: Some("beta".into()),
            delegated: false,
        }
    }

    /// A host and every lifecycle verb select from flags and the executable,
    /// never from an inherited `PLEXI_CHANNEL`, `PLEXI_SOCKET` or `HOME`.
    #[test]
    fn host_and_lifecycle_select_from_flags_then_executable_never_env() {
        let home = Path::new("/account/home");
        let root = tempfile::tempdir().unwrap();
        let explicit = ProfileRoot::parse(root.path().as_os_str()).unwrap();
        let cases = [
            (sel(ProfileSelection::Default, None), "plexi", home.join(".plexi")),
            (
                sel(named("alpha"), None),
                "plexi",
                home.join(".plexi-alpha"),
            ),
            (
                sel(ProfileSelection::Default, None),
                "plexi-pr-3279",
                home.join(".plexi-pr-3279"),
            ),
            (
                sel(ProfileSelection::Default, None),
                "plexi-alpha.exe",
                home.join(".plexi-alpha"),
            ),
            (
                sel(named("t1682"), None),
                "plexi-pr-3279",
                home.join(".plexi-t1682"),
            ),
            (
                sel(ProfileSelection::Default, Some(explicit.clone())),
                "plexi-pr-3279",
                root.path().join(".plexi-pr-3279"),
            ),
        ];
        for role in [Role::HostLaunch, Role::Lifecycle] {
            for (selectors, exe, dir) in &cases {
                let target =
                    select(role, selectors, exe, Ok(home.into()), &poisoned()).unwrap();
                assert_eq!(&target.dir(), dir, "{role:?} {selectors:?} {exe}");
                assert!(
                    !target.endpoint().to_string_lossy().contains("poison"),
                    "{target:?}"
                );
            }
        }
    }

    /// A bare command in a pane addresses that pane's profile, rooted where
    /// its endpoint lives; a channel executable or an explicit selector
    /// outranks the pane; a delegated build keeps the pane's profile.
    #[test]
    fn a_bare_command_follows_its_pane_and_explicit_addressing_wins() {
        let home = Path::new("/account/home");
        let pane = Ambient {
            home: None,
            socket: Some("/scratch/root/.plexi-t1682/notify.sock".into()),
            channel: Some("t1682".into()),
            delegated: false,
        };
        let none = sel(ProfileSelection::Default, None);
        let bare = select(Role::Command, &none, "plexi", Ok(home.into()), &pane).unwrap();
        assert_eq!(bare.dir(), Path::new("/scratch/root/.plexi-t1682"));
        assert_eq!(bare.root_source(), RootSource::Pane);
        assert!(!bare.is_addressed());

        let shim = select(Role::Command, &none, "plexi-pr-3279", Ok(home.into()), &pane).unwrap();
        assert_eq!(shim.dir(), home.join(".plexi-pr-3279"));
        assert!(shim.is_addressed());

        let delegated = Ambient {
            delegated: true,
            ..pane.clone()
        };
        let handed =
            select(Role::Command, &none, "plexi-pr-3279", Ok(home.into()), &delegated).unwrap();
        assert_eq!(handed.dir(), Path::new("/scratch/root/.plexi-t1682"));
        assert_eq!(handed.build_channel(), Some("pr-3279"));

        // Lifecycle verbs never follow the pane, delegated or not.
        let lifecycle =
            select(Role::Lifecycle, &none, "plexi", Ok(home.into()), &delegated).unwrap();
        assert_eq!(lifecycle.dir(), home.join(".plexi"));

        // `PLEXI_CHANNEL` alone (outside a pane) still names the profile of a
        // bare command, under the account home.
        let channel_only = Ambient {
            channel: Some("alpha".into()),
            ..Ambient::default()
        };
        let plugin_dev =
            select(Role::Command, &none, "plexi", Ok(home.into()), &channel_only).unwrap();
        assert_eq!(plugin_dev.dir(), home.join(".plexi-alpha"));

        let invalid = Ambient {
            channel: Some("../x".into()),
            ..Ambient::default()
        };
        assert!(select(Role::Command, &none, "plexi", Ok(home.into()), &invalid).is_err());
    }

    /// A private `HOME` no longer moves profiles. Without `--profile-root`
    /// it is refused rather than silently reaching the account's profiles.
    #[test]
    fn a_private_home_needs_an_explicit_root() {
        let account = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let ambient = Ambient {
            home: Some(private.path().into()),
            ..Ambient::default()
        };
        let none = sel(ProfileSelection::Default, None);
        for role in [Role::HostLaunch, Role::Lifecycle, Role::Command] {
            let error =
                select(role, &none, "plexi-pr-1", Ok(account.path().into()), &ambient).unwrap_err();
            assert!(error.contains("--profile-root"), "{role:?}: {error}");
        }
        let root = ProfileRoot::parse(private.path().as_os_str()).unwrap();
        let rooted = select(
            Role::Lifecycle,
            &sel(ProfileSelection::Default, Some(root)),
            "plexi-pr-1",
            Ok(account.path().into()),
            &ambient,
        )
        .unwrap();
        assert_eq!(rooted.dir(), private.path().join(".plexi-pr-1"));
    }
}
