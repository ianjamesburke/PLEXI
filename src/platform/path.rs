//! Path helpers shared across subsystems.

use std::path::{Path, PathBuf};

/// Resolve symlinks and `..` where the path exists on disk; return it
/// unchanged where it does not.
///
/// Every store keyed on a workspace or context root needs this: the key must
/// be stable across the same directory reached by different paths, but a root
/// that has since been deleted must still compare equal to its stored key
/// rather than collapsing to an error.
pub fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_is_returned_unchanged() {
        let missing = Path::new("/definitely/not/a/real/plexi/root");
        assert_eq!(canonical_or_self(missing), missing.to_path_buf());
    }

    #[test]
    fn existing_path_is_canonicalized() {
        let dir = tempfile::tempdir().expect("dir");
        let nested = dir.path().join("a");
        std::fs::create_dir(&nested).expect("mkdir");
        let indirect = nested.join("..").join("a");
        assert_eq!(
            canonical_or_self(&indirect),
            nested.canonicalize().expect("canonicalize")
        );
    }
}
