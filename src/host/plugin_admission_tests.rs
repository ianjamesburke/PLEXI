//! Stint 1398: the host loads only bundled plugins, allowed profile copies and
//! trusted folders; remote install sources are refused before any fetch.

use super::*;
use crate::app::registry::{PluginRegistry, RegistrySource};
use crate::broker::gate::ApprovalChoice;
use crate::cli::install_host::test_support::{extract_bundled, MockCloner};

fn write_plugin(parent: &Path, id: &str) -> PathBuf {
    let dir = parent.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("main.py"), "print('hi')\n").unwrap();
    dir
}

/// A profile (grant store) with its plugins folder, and a cloned repo that is
/// a workspace for this channel.
struct Rig {
    profile: tempfile::TempDir,
    repo: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let rig = Self {
            profile: tempfile::tempdir().unwrap(),
            repo: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(rig.plugins()).unwrap();
        std::fs::create_dir_all(rig.repo_plugins()).unwrap();
        rig
    }
    fn plugins(&self) -> PathBuf {
        self.profile.path().join("plugins")
    }
    fn repo_plugins(&self) -> PathBuf {
        self.repo
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("plugins")
    }
    fn admission(&self) -> PluginAdmission {
        PluginAdmission::new(self.profile.path(), &self.plugins())
    }
    fn load(&self) -> PluginRegistry {
        PluginRegistry::load_admitted(self.repo.path(), &self.plugins(), &self.admission())
    }
    fn monitor(&self) -> std::sync::Arc<PermissionMonitor> {
        PermissionMonitor::for_profile(self.profile.path()).unwrap()
    }
    /// The refusal the scan recorded for `id`.
    fn refusal(&self, registry: &PluginRegistry, id: &str) -> Refusal {
        registry
            .skipped_reason(id)
            .and_then(|skipped| skipped.refusal.clone())
            .unwrap_or_else(|| panic!("{id} should be refused: {:?}", registry.skipped()))
    }
    /// What a desktop click does: settle the filed ask.
    fn click(&self, refusal: &Refusal, choice: ApprovalChoice) -> Result<(), String> {
        let pending = self.admission().ask(refusal).expect("ask filed");
        self.monitor().approve_pending(&pending, choice)
    }
}

/// (a) A cloned repo's `<channel_dir>/plugins/<bundled id>` took the bundled
/// plugin's place with no install step. The inventory the CLI lists from
/// refuses it too.
#[test]
fn workspace_plugin_with_a_bundled_id_never_shadows_the_bundled_one() {
    let rig = Rig::new();
    extract_bundled("todo", &rig.plugins());
    let planted = write_plugin(&rig.repo_plugins(), "todo");

    let inventory = PluginRegistry::load_with_global(rig.repo.path(), &rig.plugins());
    let todo = inventory.get("todo").expect("bundled todo listed");
    assert_eq!(todo.source, RegistrySource::Global, "bundled copy wins");
    assert_eq!(
        rig.refusal(&inventory, "todo"),
        Refusal::ShadowsBundled {
            id: "todo".into(),
            dir: planted.clone()
        }
    );

    let host = rig.load();
    assert_eq!(host.get("todo").unwrap().source, RegistrySource::Global);
    let refusal = rig.refusal(&host, "todo");
    assert!(
        matches!(refusal, Refusal::ShadowsBundled { .. }),
        "{refusal:?}"
    );
    assert!(
        !refusal.can_ask(),
        "no click lets a workspace take a bundled id"
    );
    assert!(
        refusal.to_string().contains("Change [plugin] id"),
        "{refusal}"
    );
}

/// A cloned repo's own plugin folder is not loaded until the person trusts
/// that folder; once trusted, an edit does not ask again.
#[test]
fn cloned_repo_plugin_loads_only_after_its_folder_is_trusted() {
    let rig = Rig::new();
    let dir = write_plugin(&rig.repo_plugins(), "repo-tool");

    let registry = rig.load();
    assert!(registry.get("repo-tool").is_none(), "untrusted folder");
    let refusal = rig.refusal(&registry, "repo-tool");
    assert_eq!(
        refusal,
        Refusal::UntrustedFolder {
            id: "repo-tool".into(),
            folder: crate::platform::path::canonical_or_self(&rig.repo_plugins()),
        }
    );
    assert!(refusal.to_string().contains("Cmd+I"), "{refusal}");

    rig.click(&refusal, ApprovalChoice::Deny).unwrap();
    assert!(rig.load().get("repo-tool").is_none(), "deny admits nothing");

    let refusal = rig.refusal(&rig.load(), "repo-tool");
    rig.click(&refusal, ApprovalChoice::Always).unwrap();
    assert!(
        rig.load().get("repo-tool").is_some(),
        "trusted folder loads"
    );

    std::fs::write(dir.join("main.py"), "print('edited')\n").unwrap();
    assert!(
        rig.load().get("repo-tool").is_some(),
        "folder trust covers edits"
    );
}

/// The CLI cannot bypass the host: a tree written straight into the profile
/// plugins folder (as a git clone would) is not loaded until this exact copy
/// is allowed by a click, and a changed copy is refused again.
#[test]
fn planted_profile_plugin_loads_only_after_this_copy_is_allowed() {
    let rig = Rig::new();
    let dir = write_plugin(&rig.plugins(), "cloned");

    let refusal = rig.refusal(&rig.load(), "cloned");
    assert!(
        matches!(&refusal, Refusal::NotAllowed { id, .. } if id == "cloned"),
        "{refusal:?}"
    );
    assert!(
        rig.click(&refusal, ApprovalChoice::Once).is_err(),
        "a plugin load is allowed until it changes, never once"
    );
    rig.click(&refusal, ApprovalChoice::Always).unwrap();
    let loaded = rig.load();
    assert_eq!(
        loaded.get("cloned").map(|p| p.source),
        Some(RegistrySource::Global)
    );
    PermissionMonitor::reload_for_test(rig.profile.path());
    assert!(
        rig.load().get("cloned").is_some(),
        "the allow is sealed on disk and survives a restart"
    );

    std::fs::write(dir.join("main.py"), "print('swapped')\n").unwrap();
    let refusal = rig.refusal(&rig.load(), "cloned");
    assert!(
        matches!(refusal, Refusal::NotAllowed { .. }),
        "a changed copy asks again: {refusal:?}"
    );
}

/// Bundled plugins need no click; a bundled id whose files were changed is
/// treated as any other copy.
#[test]
fn bundled_plugins_load_without_asking_unless_their_files_changed() {
    let rig = Rig::new();
    let todo = extract_bundled("todo", &rig.plugins());
    assert!(rig.load().get("todo").is_some());
    assert!(
        rig.monitor().list_pending().is_empty(),
        "no ask for bundled"
    );

    std::fs::write(todo.join("main.py"), "print('not bundled')\n").unwrap();
    let refusal = rig.refusal(&rig.load(), "todo");
    assert!(matches!(refusal, Refusal::NotAllowed { .. }), "{refusal:?}");
}

/// A path under the profile plugins folder is decided as that install, never
/// as a trustable folder: trusting a profile install's own folder would skip
/// the content check.
#[test]
fn a_path_inside_a_profile_install_is_decided_as_that_install() {
    let rig = Rig::new();
    let dir = write_plugin(&rig.plugins(), "mine");
    let refusal = rig
        .admission()
        .decide("mine", &dir.join("main.py"))
        .unwrap_err();
    assert!(
        matches!(&refusal, Refusal::NotAllowed { dir: d, .. } if *d == crate::platform::path::canonical_or_self(&dir)),
        "{refusal:?}"
    );
}

/// (b) `plugin install github:...` was admitted. The decision refuses every
/// remote source; bundled sources and local folders pass.
#[test]
fn remote_install_sources_are_refused() {
    for spec in [
        "github:someone/plugin",
        "git+https://example.com/plugin.git",
        "some-marketplace-id",
    ] {
        let refusal = crate::cli::install::admit_install_spec(spec).unwrap_err();
        let text = refusal.to_string();
        assert!(text.contains(spec), "names what was refused: {text}");
        assert!(text.contains("no setting turns them on"), "{text}");
    }
    assert!(crate::cli::install::admit_install_spec("local:todo").is_ok());
    assert!(admit_install_source(InstallSource::Folder(Path::new("/tmp/mine"))).is_ok());
    assert!(admit_install_source(InstallSource::PackageFile(Path::new("x.plexipkg"))).is_err());
}

/// (b) A pack naming a git source installs nothing and clones nothing.
#[test]
fn workspace_pack_with_a_git_entry_clones_nothing() {
    let rig = Rig::new();
    let channel = rig.repo.path().join(crate::config::workspace_channel_dir());
    std::fs::write(
        channel.join("plugins.toml"),
        "schema_version = 2\n\n[[plugin]]\nid = \"todo\"\nsource = \"local:todo\"\nversion = \"bundled\"\n\n[[plugin]]\nid = \"remote\"\nsource = \"github:someone/remote\"\nversion = \"v1\"\n",
    )
    .unwrap();
    let cloner = MockCloner::new();
    let error = crate::cli::install_host::apply_workspace_pack(rig.repo.path(), &cloner)
        .expect_err("git entry refused");
    assert!(error.contains("github:someone/remote"), "{error}");
    assert!(cloner.clones.lock().unwrap().is_empty(), "nothing fetched");
    assert!(
        !rig.repo_plugins().join("todo").exists(),
        "the pack is refused whole, not half-applied"
    );
}

/// The host itself refuses: a path launch from an untrusted folder and an id
/// launch of an untrusted workspace plugin both fail with the refusal and file
/// the ask the Plexi window shows. Nothing is approved here.
#[test]
fn host_launches_from_untrusted_folders_are_refused_and_ask() {
    let mut h = crate::testing::HostHarness::new();
    let fixtures = tempfile::tempdir().unwrap();
    let dir = write_plugin(fixtures.path(), "outside");
    let error = h
        .app
        .launch_view_by_path_with_layout(&dir.to_string_lossy(), None, None, &[])
        .unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");
    assert!(error.contains("waiting there"), "{error}");

    let repo_plugins = h
        .workspace_root()
        .join(crate::config::workspace_channel_dir())
        .join("plugins");
    write_plugin(&repo_plugins, "repo-tool");
    let error = h
        .app
        .launch_view_by_id_with_layout("repo-tool", None, &[], None)
        .unwrap_err();
    assert!(error.contains("are not trusted"), "{error}");

    let monitor = PermissionMonitor::for_profile(h.profile_dir()).unwrap();
    let asks: Vec<_> = monitor
        .list_pending()
        .into_iter()
        .filter(|pending| pending.tool == TRUST_FOLDER_TOOL)
        .collect();
    assert_eq!(asks.len(), 2, "one ask per folder: {asks:?}");
}

/// Remove every `__pycache__` folder and `.pyc` under `dir`, as the packager's
/// ignore patterns do when it seeds an alpha profile.
fn strip_bytecode(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "__pycache__") {
                std::fs::remove_dir_all(&path).unwrap();
            } else {
                strip_bytecode(&path);
            }
        } else if path.extension().is_some_and(|ext| ext == "pyc") {
            std::fs::remove_file(&path).unwrap();
        }
    }
}

/// A fresh alpha profile holds every maintained plugin, copied without
/// bytecode. None of them may ask on first open, even when the binary was
/// built from a checkout whose plugin folders hold `__pycache__`.
#[test]
fn a_fresh_alpha_profile_opens_every_maintained_plugin_without_asking() {
    let rig = Rig::new();
    let repo_plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&repo_plugins).unwrap().flatten() {
        if entry.path().join("manifest.toml").is_file() {
            let id = entry.file_name().to_string_lossy().into_owned();
            strip_bytecode(&extract_bundled(&id, &rig.plugins()));
            ids.push(id);
        }
    }
    assert!(!ids.is_empty());
    let registry = rig.load();
    for id in &ids {
        assert!(
            registry.get(id).is_some(),
            "{id} asks on a fresh profile: {:?}",
            registry.skipped_reason(id)
        );
    }
    assert!(rig.monitor().list_pending().is_empty());
}

/// Bytecode the binary does not embed is not bundled: a planted `.pyc` in a
/// bundled plugin's folder makes it an ordinary copy that must be allowed.
#[test]
fn a_planted_pyc_in_a_bundled_plugin_asks() {
    let rig = Rig::new();
    let todo = extract_bundled("todo", &rig.plugins());
    let cache = todo.join("__pycache__");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("planted.cpython-312.pyc"), b"not bundled").unwrap();
    let refusal = rig.refusal(&rig.load(), "todo");
    assert!(matches!(refusal, Refusal::NotAllowed { .. }), "{refusal:?}");
}

/// Stint 1497: an unsigned update rotates the seal key and resets grants.
/// Every plugin allow and folder trust goes with them, and the plugins are
/// refused and asked again; nothing loads silently.
#[test]
fn a_seal_key_reset_drops_admissions_and_fails_closed() {
    let rig = Rig::new();
    write_plugin(&rig.plugins(), "mine");
    write_plugin(&rig.repo_plugins(), "repo-tool");
    let copy = rig.refusal(&rig.load(), "mine");
    rig.click(&copy, ApprovalChoice::Always).unwrap();
    let folder = rig.refusal(&rig.load(), "repo-tool");
    rig.click(&folder, ApprovalChoice::Always).unwrap();
    let loaded = rig.load();
    assert!(loaded.get("mine").is_some() && loaded.get("repo-tool").is_some());

    crate::broker::seal::reset_unverified_profile(
        &crate::broker::custody::HostProcess::for_test(),
        rig.profile.path(),
    )
    .unwrap();
    PermissionMonitor::reload_for_test(rig.profile.path());

    let after = rig.load();
    assert!(matches!(
        rig.refusal(&after, "mine"),
        Refusal::NotAllowed { .. }
    ));
    assert!(matches!(
        rig.refusal(&after, "repo-tool"),
        Refusal::UntrustedFolder { .. }
    ));
}
