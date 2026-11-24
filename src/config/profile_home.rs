use std::path::PathBuf;

pub(super) fn honors_home() -> bool {
    cfg!(plexi_test_channel) && super::is_test_channel(super::build_channel().as_deref())
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
    home.ok_or_else(|| {
        #[cfg(unix)]
        let lookup = "getpwuid_r for the current uid";
        #[cfg(not(unix))]
        let lookup = "the account profile directory lookup";
        format!("cannot resolve Plexi profile home: {lookup} returned no home directory")
    })
}

/// Account home independent of the process environment.
#[cfg(unix)]
pub(crate) fn login_home_dir() -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0u8; 16 * 1024];
    let mut result = std::ptr::null_mut();
    loop {
        // SAFETY: getpwuid_r initializes pwd on success; pw_dir points into
        // buf and we copy it before the buffer is dropped.
        unsafe {
            let rc = libc::getpwuid_r(
                libc::getuid(),
                pwd.as_mut_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            );
            if rc == libc::ERANGE {
                buf.resize(buf.len().checked_mul(2)?, 0);
                continue;
            }
            if rc != 0 || result.is_null() || (*result).pw_dir.is_null() {
                return None;
            }
            let path = PathBuf::from(OsStr::from_bytes(
                CStr::from_ptr((*result).pw_dir).to_bytes(),
            ));
            return path.is_absolute().then_some(path);
        }
    }
}

#[cfg(windows)]
pub(crate) fn login_home_dir() -> Option<PathBuf> {
    // dirs uses the current account's FOLDERID_Profile on Windows.
    dirs::home_dir()
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
    assert!(profile_home_from(false, env.clone(), None).is_err());
    assert_eq!(profile_home_from(true, env.clone(), None).ok(), env);
}
