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

/// The scope error for a `caller_context` request. A pane caller whose root
/// cannot be resolved is refused, never approved without a root.
pub(crate) fn caller_scope_error(
    is_pane: bool,
    root: Option<&Path>,
    workspace_cwd: Option<&Path>,
) -> Option<String> {
    let cwd = workspace_cwd?;
    match root {
        Some(root) => check_context_scope(root.to_path_buf(), cwd).err(),
        None if is_pane => {
            log::warn!(
                "caller_context: pane caller has no resolvable context root; refusing scope"
            );
            Some(format!(
                "permission_denied: host could not resolve this pane's context root for {}",
                cwd.display()
            ))
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_without_a_root_is_refused_and_a_plain_caller_is_not() {
        let cwd = Path::new("/tmp");
        let refused = caller_scope_error(true, None, Some(cwd)).unwrap();
        assert!(refused.starts_with("permission_denied:"), "{refused}");
        assert_eq!(caller_scope_error(false, None, Some(cwd)), None);
        assert_eq!(caller_scope_error(true, None, None), None);
    }
}
