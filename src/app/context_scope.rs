use crate::cli::workspace::secret_workspace_root;
use std::path::{Path, PathBuf};

/// Check a workspace target against the calling pane's verified authority root.
pub(crate) fn check_context_scope(ctx_root: PathBuf, target_dir: &Path) -> Result<(), String> {
    let target_canon = crate::platform::path::canonical_or_self(target_dir);
    let denied = if !target_canon.starts_with(&ctx_root) {
        true
    } else if target_canon != ctx_root {
        let nested_ws = secret_workspace_root(&target_canon, Some(&ctx_root))
            .is_some_and(|ws| crate::platform::path::canonical_or_self(&ws) != ctx_root);
        let nested_agent_ws =
            crate::app::registry::resolve_agent_lookup_workspace_root(&target_canon)
                .map(|ws| crate::platform::path::canonical_or_self(&ws))
                .is_some_and(|ws| ws.starts_with(&ctx_root) && ws != ctx_root);
        let nested_folder_secret = crate::workspace::secrets::list_folder_secrets(
            crate::workspace::secrets::folder_store(),
        )
        .iter()
        .any(|meta| {
            let f = crate::platform::path::canonical_or_self(&meta.folder);
            f != ctx_root && f.starts_with(&ctx_root) && target_canon.starts_with(&f)
        });
        nested_ws || nested_agent_ws || nested_folder_secret
    } else {
        false
    };
    if denied {
        return Err(format!(
            "permission_denied: pane has no authority over context '{}' (authority is scoped to '{}')",
            target_canon.display(),
            ctx_root.display()
        ));
    }
    Ok(())
}
