//! Only test executables can read these overrides. Launchers also need to
//! recognize their names when removing them from the child environment.
use std::ffi::OsString;

#[derive(Clone, Copy)]
pub(crate) struct TestHook {
    suffix: &'static str,
    #[cfg(any(test, plexi_test_channel))]
    pub(crate) name: &'static str,
}

macro_rules! hook {
    ($suffix:literal) => {
        TestHook {
            suffix: $suffix,
            #[cfg(any(test, plexi_test_channel))]
            name: concat!("PLEXI_", $suffix),
        }
    };
}

pub(crate) const KEYCHAIN_PATH: TestHook = hook!("KEYCHAIN_PATH");
pub(crate) const KEYCHAIN_PASSWORD: TestHook = hook!("KEYCHAIN_PASSWORD");
pub(crate) const OPENROUTER_BASE_URL: TestHook = hook!("OPENROUTER_BASE_URL");
pub(crate) const PICKER_SCRIPT: TestHook = hook!("PICKER_SCRIPT");
pub(crate) const TEST_HOOKS: [TestHook; 4] = [
    KEYCHAIN_PATH,
    KEYCHAIN_PASSWORD,
    OPENROUTER_BASE_URL,
    PICKER_SCRIPT,
];

impl TestHook {
    pub(crate) fn value(self) -> Option<OsString> {
        #[cfg(any(test, plexi_test_channel))]
        {
            let enabled = cfg!(test) || super::is_test_channel(super::build_channel().as_deref());
            self.read_with(enabled, |name| std::env::var_os(name))
        }
        #[cfg(not(any(test, plexi_test_channel)))]
        {
            None
        }
    }

    #[cfg(any(test, plexi_test_channel))]
    fn read_with(
        self,
        enabled: bool,
        read: impl FnOnce(&str) -> Option<OsString>,
    ) -> Option<OsString> {
        if enabled {
            read(self.name)
        } else {
            None
        }
    }
}

impl std::fmt::Display for TestHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(any(test, plexi_test_channel))]
        return f.write_str(self.name);
        #[cfg(not(any(test, plexi_test_channel)))]
        f.write_str(self.suffix)
    }
}

pub(crate) fn is_hook(name: &str) -> bool {
    #[cfg(windows)]
    let uppercase = name.to_ascii_uppercase();
    #[cfg(windows)]
    let name = uppercase.as_str();
    name.strip_prefix("PLEXI_")
        .is_some_and(|suffix| TEST_HOOKS.iter().any(|hook| hook.suffix == suffix))
}

/// Called once at startup after the logger is ready, including for a launcher
/// that will remove overrides before starting a child.
#[cfg(not(test))]
pub(crate) fn report_ignored() {
    if !super::is_test_channel(super::build_channel().as_deref())
        && std::env::vars_os().any(|(key, _)| key.to_str().is_some_and(is_hook))
    {
        log::info!("a test-only environment override is set and ignored");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_release_configuration_never_reads_a_hook() {
        for hook in TEST_HOOKS {
            assert_eq!(
                hook.read_with(false, |_| panic!("read disabled hook")),
                None
            );
            assert_eq!(
                hook.read_with(true, |name| Some(name.into())),
                Some(hook.name.into())
            );
        }
    }
}
