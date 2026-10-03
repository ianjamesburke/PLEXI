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
