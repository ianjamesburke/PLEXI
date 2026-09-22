//! Short, platform-honest installation guidance for optional CLI dependencies.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallPlatform {
    Macos,
    Linux,
    Windows,
    Other,
}

impl InstallPlatform {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// The supported package-manager identifiers and official installation page for
/// one optional dependency. Leave a manager identifier absent when it is not a
/// supported installation path for that platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstallTarget {
    pub(crate) brew: Option<&'static str>,
    pub(crate) apt: Option<&'static str>,
    pub(crate) winget: Option<&'static str>,
    pub(crate) url: &'static str,
}

pub(crate) fn install_hint(target: InstallTarget) -> String {
    install_hint_for(InstallPlatform::current(), target)
}

fn install_hint_for(platform: InstallPlatform, target: InstallTarget) -> String {
    match platform {
        InstallPlatform::Macos => target
            .brew
            .map(|package| format!("brew install {package}"))
            .unwrap_or_else(|| format!("install from {}", target.url)),
        InstallPlatform::Linux => target
            .apt
            .map(|package| format!("sudo apt install {package} (or {})", target.url))
            .unwrap_or_else(|| format!("install from {}", target.url)),
        InstallPlatform::Windows => target
            .winget
            .map(|package| format!("winget install {package} (or {})", target.url))
            .unwrap_or_else(|| format!("install from {}", target.url)),
        InstallPlatform::Other => format!("install from {}", target.url),
    }
}

#[cfg(test)]
mod tests {
    use super::{install_hint_for, InstallPlatform, InstallTarget};

    const TARGET: InstallTarget = InstallTarget {
        brew: Some("example"),
        apt: Some("example-cli"),
        winget: Some("Example.Cli"),
        url: "https://example.test/install",
    };

    #[test]
    fn install_hints_use_the_native_package_manager_with_a_fallback_url() {
        assert_eq!(
            install_hint_for(InstallPlatform::Macos, TARGET),
            "brew install example"
        );
        assert_eq!(
            install_hint_for(InstallPlatform::Linux, TARGET),
            "sudo apt install example-cli (or https://example.test/install)"
        );
        assert_eq!(
            install_hint_for(InstallPlatform::Windows, TARGET),
            "winget install Example.Cli (or https://example.test/install)"
        );
    }

    #[test]
    fn install_hints_fall_back_to_the_official_url_when_no_package_is_supported() {
        let url_only = InstallTarget {
            brew: None,
            apt: None,
            winget: None,
            url: "https://example.test/install",
        };
        assert_eq!(
            install_hint_for(InstallPlatform::Linux, url_only),
            "install from https://example.test/install"
        );
        assert_eq!(
            install_hint_for(InstallPlatform::Other, url_only),
            "install from https://example.test/install"
        );
    }
}
