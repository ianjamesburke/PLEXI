//! Host-facing package identity and installed runtime discovery.
use std::path::PathBuf;

pub fn build_info() -> serde_json::Value {
    serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "version_text": version_text(), "tag": build_tag(), "build_id": env!("PLEXI_BUILD_ID"), "source_commit": env!("PLEXI_SOURCE_COMMIT"), "test_channel": option_env!("PLEXI_BUILD_TEST_CHANNEL") })
}
/// The version every UI and CLI surface shows: this binary's compiled tag (or
/// crate version) and short commit, e.g. `0.4.2 (abc1234)`. Profile markers
/// such as `installed_tag` never feed it.
pub fn version_text() -> &'static str {
    env!("PLEXI_VERSION_TEXT")
}
/// The same rendering for an installed generation; its build id starts with
/// the full source commit.
pub fn generation_version_text(tag: &str, build_id: &str) -> String {
    let version = Some(tag.trim_start_matches('v')).filter(|t| !t.is_empty()).unwrap_or("unknown");
    let commit = build_id.get(..7).filter(|c| !c.contains('-')).unwrap_or("unknown");
    format!("{version} ({commit})")
}
/// Where a user relaunches the managed build: the Applications bundle on
/// macOS, the active executable elsewhere.
pub fn relaunch_path(receipt: &plexi_distribution::transaction::Receipt) -> PathBuf {
    if cfg!(target_os = "macos") {
        receipt.applications_dir.join(format!("{}.app", plexi_distribution::transaction::display_name(&receipt.channel)))
    } else {
        receipt.active.executable()
    }
}
/// One-line notice when the running binary is not the channel's installed
/// build. Compares build ids, never paths, so the managed Applications copy
/// and the generation it was copied from are the same build.
pub fn installation_mismatch(
    running_build: &str,
    running_text: &str,
    receipt: &plexi_distribution::transaction::Receipt,
) -> Option<String> {
    if receipt.active.build_id == running_build {
        return None;
    }
    Some(format!(
        "Running {running_text} but {}'s installed build is {} — relaunch from {}",
        plexi_distribution::transaction::display_name(&receipt.channel),
        generation_version_text(&receipt.active.tag, &receipt.active.build_id),
        relaunch_path(receipt).display()
    ))
}
pub fn build_tag() -> String {
    let tag = env!("PLEXI_BUILD_TAG");
    if tag.is_empty() {
        format!("v{}", env!("CARGO_PKG_VERSION"))
    } else {
        tag.into()
    }
}
pub fn resources() -> Result<Option<PathBuf>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve executable: {e}"))?;
    plexi_distribution::transaction::package_for_executable(&exe, env!("PLEXI_BUILD_ID"))
        .and_then(|p| p.map(|p| p.resources()).transpose())
        .map_err(|e| e.to_string())
}
pub fn installed() -> Result<Option<plexi_distribution::transaction::Receipt>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve executable: {e}"))?;
    if let Some(receipt) = plexi_distribution::transaction::installed_for_executable(&exe)
        .map_err(|e| e.to_string())?
    {
        return Ok(Some(receipt));
    }
    let channel = crate::config::build_channel().unwrap_or_else(|| "stable".into());
    plexi_distribution::transaction::find_installation(&channel).map_err(|e| e.to_string())
}
pub fn check_runtime() -> Result<(), String> {
    let resources = resources()?.ok_or("this executable is not inside a distribution package")?;
    let config = crate::host::wasm_python::PythonLaunchConfig::from_manifest_file(
        &resources.join("smoke-app"),
    )
    .map_err(|e| e.to_string())?
    .ok_or("package smoke app has no Python entry")?;
    let tree = crate::host::wasm_python::run_headless_frame(&config, (800.0, 600.0), None)
        .map_err(|e| e.to_string())?;
    if tree.nodes.is_empty() {
        return Err("packaged Calculator produced an empty frame".into());
    }
    Ok(())
}

/// Read-only package freshness probe; unpackaged developer binaries have nothing to seed.
pub fn profile_needs_seeding() -> Result<bool, String> {
    Ok(resources()?.is_some()
        && !std::fs::read_to_string(crate::config::config_dir().join(".package_definitions_build"))
            .is_ok_and(|build| build == env!("PLEXI_BUILD_ID")))
}

fn seed_config(template: &std::path::Path, config: &std::path::Path) -> Result<(), String> {
    match std::fs::symlink_metadata(config) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect existing config: {error}")),
    }
    let parent = config.parent().ok_or("config has no parent")?;
    let mut source =
        std::fs::File::open(template).map_err(|error| format!("open default config: {error}"))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("stage default config: {error}"))?;
    std::io::copy(&mut source, staged.as_file_mut())
        .map_err(|error| format!("write default config: {error}"))?;
    staged
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync default config: {error}"))?;
    match staged.persist_noclobber(config) {
        Ok(_) => log::info!("distribution: seeded default config"),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            log::info!("distribution: retained existing config")
        }
        Err(error) => return Err(format!("install default config: {}", error.error)),
    }
    Ok(())
}

/// Seed package-owned definitions by build while retaining user settings/data.
pub fn seed_profile() -> Result<(), String> {
    let Some(resources) = resources()? else {
        return Ok(());
    };
    seed_profile_from(&resources, &crate::config::config_dir())
}

fn seed_profile_from(resources: &std::path::Path, profile: &std::path::Path) -> Result<(), String> {
    let stamp = profile.join(".package_definitions_build");
    if std::fs::read_to_string(&stamp).is_ok_and(|s| s == env!("PLEXI_BUILD_ID")) {
        return Ok(());
    }
    fn copy_tree(
        source: &std::path::Path,
        target: &std::path::Path,
        overwrite: bool,
    ) -> Result<(), String> {
        if !source.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(target).map_err(|e| format!("create {}: {e}", target.display()))?;
        for entry in
            std::fs::read_dir(source).map_err(|e| format!("read {}: {e}", source.display()))?
        {
            let entry = entry.map_err(|e| format!("read package resource: {e}"))?;
            let destination = target.join(entry.file_name());
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                copy_tree(&entry.path(), &destination, overwrite)?;
            } else if overwrite || !destination.exists() {
                std::fs::copy(entry.path(), &destination)
                    .map_err(|e| format!("seed {}: {e}", destination.display()))?;
            }
        }
        Ok(())
    }
    let config = profile.join("config.toml");
    seed_config(&resources.join("default-config.toml"), &config)?;
    copy_tree(&resources.join("scripts"), &profile.join("scripts"), false)?;
    copy_tree(&resources.join("agents"), &profile.join("agents"), false)?;
    copy_tree(
        &resources.join("skills"),
        &profile.join(".agents/skills"),
        true,
    )?;
    copy_tree(
        &resources.join("maintained-apps"),
        &profile.join("plugins"),
        true,
    )?;
    std::fs::write(stamp, env!("PLEXI_BUILD_ID"))
        .map_err(|e| format!("record package definitions: {e}"))?;
    log::info!(
        "distribution: seeded package definitions build={}",
        env!("PLEXI_BUILD_ID")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    fn receipt(tag: &str, build_id: &str) -> plexi_distribution::transaction::Receipt {
        let generation = plexi_distribution::transaction::Generation {
            path: "/home/u/.local/share/plexi/beta/generations/abc".into(),
            build_id: build_id.into(),
            tag: tag.into(),
            executable: "Plexi Beta.app/Contents/MacOS/plexi-beta".into(),
        };
        plexi_distribution::transaction::Receipt {
            schema: 1,
            channel: "beta".into(),
            root: "/home/u/.local/share/plexi/beta".into(),
            bin_dir: "/home/u/.local/bin".into(),
            applications_dir: "/home/u/Applications".into(),
            active: generation,
            previous: None,
            integrations: vec![],
            path_edits: vec![],
            windows_path_added: false,
        }
    }

    #[test]
    fn version_text_is_compiled_build_info() {
        use clap::CommandFactory;
        let text = super::version_text();
        assert!(text.ends_with(&format!("({})", env!("PLEXI_SHORT_COMMIT"))), "{text}");
        let tag = env!("PLEXI_BUILD_TAG").trim_start_matches('v');
        assert!(text.starts_with(if tag.is_empty() { env!("CARGO_PKG_VERSION") } else { tag }));
        assert_eq!(crate::cli::args::Cli::command().get_version(), Some(text));
        assert_eq!(super::build_info()["version_text"], text);
    }

    #[test]
    fn mismatch_compares_build_ids_and_names_the_relaunch_path() {
        let installed = "def5678aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-1234";
        let receipt = receipt("v0.4.1", installed);
        assert_eq!(super::installation_mismatch(installed, "0.4.1 (def5678)", &receipt), None);
        let notice = super::installation_mismatch("abc1234-x", "0.4.2 (abc1234)", &receipt).unwrap();
        assert!(notice.starts_with("Running 0.4.2 (abc1234) but Plexi Beta's installed build is 0.4.1 (def5678) — relaunch from "), "{notice}");
        assert!(notice.ends_with(&super::relaunch_path(&receipt).display().to_string()));
        assert!(!notice.contains('\n'));
        assert_eq!(super::generation_version_text("", "-hash-only"), "unknown (unknown)");
    }

    #[test]
    fn package_seeding_preserves_existing_config_and_creates_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        let template = dir.path().join("template.toml");
        let config = dir.path().join("config.toml");
        std::fs::write(&template, "font_size = 12.0\n").unwrap();
        std::fs::write(&config, "font_size = 18.0\n").unwrap();
        super::seed_config(&template, &config).unwrap();
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "font_size = 18.0\n"
        );
        std::fs::remove_file(&config).unwrap();
        super::seed_config(&template, &config).unwrap();
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "font_size = 12.0\n"
        );
        #[cfg(unix)]
        {
            std::fs::remove_file(&config).unwrap();
            std::os::unix::fs::symlink(dir.path().join("missing"), &config).unwrap();
            super::seed_config(&template, &config).unwrap();
            assert!(config.is_symlink());
        }
    }

    #[test]
    fn alpha_reinstall_leaves_customized_config_byte_identical() {
        let package = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let _channel = crate::config::set_test_channel("alpha");
        let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        std::fs::write(
            package.path().join("default-config.toml"),
            "config_version = 1\nfont_size = 12.0\n",
        )
        .unwrap();
        let customized = "# mine\nconfig_version = 1\nfont_size = 18.0\n\
                          [theme]\npreset = \"catppuccin-mocha\"\n\
                          [ai]\nbackend = \"openrouter\"\n";
        let config = crate::config::config_path();
        assert!(config.starts_with(profile.path()));
        std::fs::write(&config, customized).unwrap();

        super::seed_profile_from(package.path(), &crate::config::config_dir()).unwrap();
        assert_eq!(std::fs::read(&config).unwrap(), customized.as_bytes());
        assert_eq!(
            std::fs::read_to_string(profile.path().join(".package_definitions_build")).unwrap(),
            env!("PLEXI_BUILD_ID")
        );

        // A later build re-seeds package definitions and still retains settings.
        std::fs::write(profile.path().join(".package_definitions_build"), "older").unwrap();
        super::seed_profile_from(package.path(), &crate::config::config_dir()).unwrap();
        assert_eq!(std::fs::read(&config).unwrap(), customized.as_bytes());
    }

    #[test]
    fn fresh_profile_is_seeded_from_the_package_template() {
        let package = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        std::fs::write(
            package.path().join("default-config.toml"),
            "font_size = 12.0\n",
        )
        .unwrap();
        super::seed_profile_from(package.path(), profile.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(profile.path().join("config.toml")).unwrap(),
            "font_size = 12.0\n"
        );
    }
}
