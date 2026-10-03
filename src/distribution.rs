//! Host-facing package identity and installed runtime discovery.
use std::path::PathBuf;

pub fn build_info() -> serde_json::Value {
    serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "tag": build_tag(), "build_id": env!("PLEXI_BUILD_ID"), "source_commit": env!("PLEXI_SOURCE_COMMIT") })
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
    plexi_distribution::package::Package::at_executable(&exe)
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

/// Seed package-owned definitions by build while retaining user settings/data.
pub fn seed_profile() -> Result<(), String> {
    let Some(resources) = resources()? else {
        return Ok(());
    };
    let profile = crate::config::config_dir();
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
    if !config.exists() || crate::config::build_channel().as_deref() == Some("alpha") {
        std::fs::copy(resources.join("default-config.toml"), &config)
            .map_err(|e| format!("seed config: {e}"))?;
    }
    copy_tree(&resources.join("scripts"), &profile.join("scripts"), false)?;
    copy_tree(&resources.join("agents"), &profile.join("agents"), false)?;
    copy_tree(
        &resources.join("skills"),
        &profile.join(".agents/skills"),
        true,
    )?;
    copy_tree(
        &resources.join("maintained-apps"),
        &profile.join("apps"),
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
