use std::path::PathBuf;

pub(crate) use plexi_distribution::host_presence::account_home as login_home_dir;

/// Only a binary that cannot ship honors `HOME` for the profile: a build
/// compiled for the matching numbered PR channel, or a debug-assertions build,
/// which is what `cargo test` compiles for the binaries integration tests
/// spawn. Release channel builds never do.
pub(crate) fn honors_home() -> bool {
    cfg!(debug_assertions)
        || (cfg!(plexi_test_channel) && super::is_test_channel(super::build_channel().as_deref()))
}

/// The parent of Plexi's profile directories is fixed for the invocation.
pub(crate) fn profile_home() -> Result<PathBuf, String> {
    let resolve = || {
        let test_channel = honors_home();
        let env_home = if test_channel { dirs::home_dir() } else { None };
        profile_home_from(test_channel, env_home, login_home_dir())
    };
    #[cfg(test)]
    return resolve(); // build_channel() can be overridden per test thread.
    #[cfg(not(test))]
    {
        static HOME: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
        HOME.get_or_init(resolve).clone()
    }
}

pub(crate) fn profile_home_from(
    test_channel: bool,
    env_home: Option<PathBuf>,
    passwd_home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let home = env_home.filter(|_| test_channel).or(passwd_home);
    home.ok_or_else(|| plexi_distribution::host_presence::ACCOUNT_HOME_ERROR.to_owned())
}

fn profile_home_failure_message(error: &str) -> String {
    format!("error: {error}. The account must have a home directory in the account database.")
}

pub(crate) fn profile_home_or_exit() -> PathBuf {
    profile_home().unwrap_or_else(|error| {
        let message = profile_home_failure_message(&error);
        log::error!("{message}");
        eprintln!("{message}");
        std::process::exit(1)
    })
}

#[cfg(test)]
#[test]
fn profile_home_release_ignores_a_changed_home() {
    let env = Some(PathBuf::from("/sandbox"));
    let passwd = Some(PathBuf::from("/account"));
    assert_eq!(
        profile_home_from(false, env.clone(), passwd.clone()).ok(),
        passwd
    );
    let error = profile_home_from(false, env.clone(), None).unwrap_err();
    #[cfg(unix)]
    assert!(error.contains("getpwuid_r for the current uid"));
    let message = profile_home_failure_message(&error);
    assert!(message.starts_with(&format!("error: {error}")));
    assert!(message.contains("home directory in the account database"));
    assert_eq!(profile_home_from(true, env.clone(), None).ok(), env);
}

#[cfg(test)]
#[test]
fn debug_assertions_builds_honor_home_for_spawned_test_binaries() {
    // `cargo test` compiles the binary integration tests spawn with debug
    // assertions, so their temporary HOME holds the profile. A release build
    // has neither this nor the matching PR channel and ignores HOME.
    assert!(honors_home());
    assert!(
        profile_home_from(true, Some("/tmp/h".into()), Some("/account".into()))
            .is_ok_and(|home| home == PathBuf::from("/tmp/h"))
    );
}
