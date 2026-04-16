use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A discovered `.plexi` depth boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthNode {
    /// Stable identifier derived from the directory's workspace-relative path.
    /// The workspace root is represented as ".".
    pub node_id: String,
    /// Absolute path to the directory that owns the `.plexi` boundary.
    pub directory_path: PathBuf,
    /// Human-readable label used by simple tree views.
    pub display_name: String,
    /// Boundary nesting level. The workspace root is level 0.
    pub depth_level: usize,
    /// Parent depth node identifier, if any.
    pub parent_id: Option<String>,
    /// Number of direct child depth nodes below this node.
    pub child_count: usize,
    /// Timestamp of the scan that discovered this node.
    pub last_scan_time: DateTime<Utc>,
}

/// A full scan result for a workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthTree {
    pub workspace_root: PathBuf,
    pub scanned_at: DateTime<Utc>,
    pub nodes: Vec<DepthNode>,
}

impl DepthTree {
    /// Discover every `.plexi` boundary beneath `workspace_root`.
    pub fn scan(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let scanned_at = Utc::now();
        let mut nodes = Vec::new();

        let root_node = if workspace_root.join(".plexi").is_dir() {
            let node = make_depth_node(&workspace_root, &workspace_root, None, 0, scanned_at);
            nodes.push(node.clone());
            Some(node)
        } else {
            None
        };

        let child_depth_level = if root_node.is_some() { 1 } else { 0 };
        scan_dir(
            &workspace_root,
            &workspace_root,
            root_node.as_ref(),
            child_depth_level,
            scanned_at,
            &mut nodes,
        )?;

        let mut child_counts: HashMap<String, usize> = HashMap::new();
        for node in &nodes {
            if let Some(parent_id) = &node.parent_id {
                *child_counts.entry(parent_id.clone()).or_default() += 1;
            }
        }
        for node in &mut nodes {
            node.child_count = child_counts.get(&node.node_id).copied().unwrap_or(0);
        }

        Ok(Self {
            workspace_root,
            scanned_at,
            nodes,
        })
    }
}

/// Directories to skip during recursive scanning — large or irrelevant subtrees.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "__pycache__",
    ".cargo",
    ".rustup",
    "build",
    "dist",
    ".tox",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    "Library",
    ".Trash",
    ".npm",
    ".cache",
    ".local",
    "Caches",
];

fn scan_dir(
    workspace_root: &Path,
    dir: &Path,
    parent: Option<&DepthNode>,
    depth_level: usize,
    scanned_at: DateTime<Utc>,
    nodes: &mut Vec<DepthNode>,
) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return Ok(()),
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Skip .plexi itself (it's the boundary marker, not a child).
        if name_str == ".plexi" {
            continue;
        }
        // Skip all other hidden directories.
        if name_str.starts_with('.') {
            continue;
        }
        // Skip known-heavy directories that never contain .plexi boundaries.
        if SKIP_DIRS.contains(&&*name_str) {
            continue;
        }

        let is_boundary = path.join(".plexi").is_dir();
        if is_boundary {
            let node = make_depth_node(workspace_root, &path, parent, depth_level, scanned_at);
            nodes.push(node.clone());
            scan_dir(
                workspace_root,
                &path,
                Some(&node),
                depth_level + 1,
                scanned_at,
                nodes,
            )?;
        } else {
            scan_dir(
                workspace_root,
                &path,
                parent,
                depth_level,
                scanned_at,
                nodes,
            )?;
        }
    }

    Ok(())
}

fn make_depth_node(
    workspace_root: &Path,
    path: &Path,
    parent: Option<&DepthNode>,
    depth_level: usize,
    scanned_at: DateTime<Utc>,
) -> DepthNode {
    DepthNode {
        node_id: relative_node_id(workspace_root, path),
        directory_path: path.to_path_buf(),
        display_name: display_name_for(workspace_root, path),
        depth_level,
        parent_id: parent.map(|p| p.node_id.clone()),
        child_count: 0,
        last_scan_time: scanned_at,
    }
}

fn relative_node_id(workspace_root: &Path, path: &Path) -> String {
    if path == workspace_root {
        return ".".to_string();
    }
    path.strip_prefix(workspace_root)
        .map(path_to_tree_id)
        .unwrap_or_else(|_| path_to_tree_id(path))
}

fn path_to_tree_id(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn display_name_for(workspace_root: &Path, path: &Path) -> String {
    if path == workspace_root {
        return workspace_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| workspace_root.display().to_string());
    }

    path.strip_prefix(workspace_root)
        .map(path_to_tree_id)
        .unwrap_or_else(|_| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempTree {
        path: PathBuf,
    }

    impl TempTree {
        fn new() -> Self {
            let unique = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "plexi-depth-tree-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0),
                unique
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn mark_depth_boundary(dir: &Path) {
        let plexi = dir.join(".plexi");
        fs::create_dir_all(&plexi).unwrap();
        fs::write(plexi.join("config.toml"), "mode = \"embedded\"\n").unwrap();
        fs::write(plexi.join("canvas.toml"), "selected = true\n").unwrap();
    }

    #[test]
    fn scan_discovers_nested_depth_nodes_with_parents_and_counts() {
        let tree = TempTree::new();
        mark_depth_boundary(tree.path());

        let agents = tree.path().join("agents");
        let services = tree.path().join("services");
        let scratch = tree.path().join("scratch");
        fs::create_dir_all(&agents).unwrap();
        fs::create_dir_all(&services).unwrap();
        fs::create_dir_all(scratch.join("cache")).unwrap();

        let scraper = agents.join("scraper");
        let reviewer = agents.join("reviewer");
        let api = services.join("api");
        fs::create_dir_all(&scraper).unwrap();
        fs::create_dir_all(&reviewer).unwrap();
        fs::create_dir_all(&api).unwrap();
        mark_depth_boundary(&scraper);
        mark_depth_boundary(&reviewer);
        mark_depth_boundary(&api);

        let tools = scraper.join("tools");
        fs::create_dir_all(&tools).unwrap();
        mark_depth_boundary(&tools);

        let discovered = DepthTree::scan(tree.path()).unwrap();

        let mut by_id = discovered
            .nodes
            .iter()
            .map(|node| (node.node_id.clone(), node))
            .collect::<HashMap<_, _>>();

        let root = by_id.remove(".").expect("root node");
        assert_eq!(root.directory_path, tree.path());
        assert_eq!(
            root.display_name,
            tree.path().file_name().unwrap().to_string_lossy()
        );
        assert_eq!(root.depth_level, 0);
        assert_eq!(root.parent_id, None);
        assert_eq!(root.child_count, 3);

        let scraper = by_id.remove("agents/scraper").expect("scraper node");
        assert_eq!(scraper.directory_path, tree.path().join("agents/scraper"));
        assert_eq!(scraper.display_name, "agents/scraper");
        assert_eq!(scraper.depth_level, 1);
        assert_eq!(scraper.parent_id.as_deref(), Some("."));
        assert_eq!(scraper.child_count, 1);

        let reviewer = by_id.remove("agents/reviewer").expect("reviewer node");
        assert_eq!(reviewer.parent_id.as_deref(), Some("."));
        assert_eq!(reviewer.depth_level, 1);
        assert_eq!(reviewer.child_count, 0);

        let api = by_id.remove("services/api").expect("api node");
        assert_eq!(api.parent_id.as_deref(), Some("."));
        assert_eq!(api.depth_level, 1);
        assert_eq!(api.child_count, 0);

        let tools = by_id.remove("agents/scraper/tools").expect("tools node");
        assert_eq!(tools.parent_id.as_deref(), Some("agents/scraper"));
        assert_eq!(tools.depth_level, 2);
        assert_eq!(tools.child_count, 0);

        assert!(!discovered
            .nodes
            .iter()
            .any(|node| node.node_id.contains("scratch/cache")));
        assert!(by_id.is_empty());
    }

    #[test]
    fn scan_ignores_directories_without_plexi_boundaries() {
        let tree = TempTree::new();
        mark_depth_boundary(tree.path());

        fs::create_dir_all(tree.path().join("docs/reference")).unwrap();
        fs::create_dir_all(tree.path().join("tools/generated")).unwrap();
        fs::create_dir_all(tree.path().join("agents/nested")).unwrap();

        let agent = tree.path().join("agents/nested");
        mark_depth_boundary(&agent);

        let discovered = DepthTree::scan(tree.path()).unwrap();
        let ids: Vec<_> = discovered
            .nodes
            .iter()
            .map(|node| node.node_id.as_str())
            .collect();

        assert!(ids.contains(&"."));
        assert!(ids.contains(&"agents/nested"));
        assert!(!ids.contains(&"docs"));
        assert!(!ids.contains(&"docs/reference"));
        assert!(!ids.contains(&"tools"));
        assert!(!ids.contains(&"tools/generated"));
    }
}
