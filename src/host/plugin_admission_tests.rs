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
/// plugins folder (as a git clone would) is not loaded until its id is allowed
/// by a click. The allow is keyed to the id in this profile, not to content
/// (stint 1279 owns content binding).
#[test]
fn planted_profile_plugin_loads_only_after_it_is_allowed() {
    let rig = Rig::new();
    write_plugin(&rig.plugins(), "cloned");

    let refusal = rig.refusal(&rig.load(), "cloned");
    assert!(
        matches!(&refusal, Refusal::NotAllowed { id, .. } if id == "cloned"),
        "{refusal:?}"
    );
    assert!(
        rig.click(&refusal, ApprovalChoice::Once).is_err(),
        "a plugin load is allowed or denied, never once"
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
}

/// Bundled plugins need no click. "Bundled" is decided by comparing bytes at
/// scan time: a profile copy of a bundled id whose files differ at that scan is
/// treated as any other profile copy. This is not a guarantee about what a
/// guest reads later (stint 1279).
#[test]
fn bundled_is_a_scan_time_comparison_with_the_embedded_copy() {
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
/// as a trustable folder: a folder trust there would cover every install.
#[test]
fn a_path_inside_a_profile_install_is_decided_as_that_install() {
    let rig = Rig::new();
    let dir = write_plugin(&rig.plugins(), "mine");
    let refusal = rig
        .admission()
        .decide("mine", &dir.join("main.py"), Launch::ExplicitPath)
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

/// (b) A pack naming a git source installs nothing and clones nothing: the
/// refusal sits at the shared install boundary every caller goes through.
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
    let outcomes = crate::cli::install_host::apply_workspace_pack(rig.repo.path(), &cloner)
        .expect("the pack is read");
    for outcome in &outcomes {
        let crate::cli::install_host::InstallStatus::Failed(error) = &outcome.status else {
            panic!("{} must be refused: {:?}", outcome.id, outcome.status);
        };
        assert!(error.contains("github:someone/remote"), "{error}");
    }
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

/// (a) The shared install boundary refuses a git source before the cloner
/// runs, whoever calls it.
#[test]
fn install_one_refuses_a_git_source_before_cloning() {
    let rig = Rig::new();
    let cloner = MockCloner::new();
    let error = crate::cli::install_host::install_one(
        &cloner,
        &crate::app::packs::SourceSpec::Git("https://github.com/someone/plugin.git".into()),
        None,
        &rig.plugins(),
    )
    .expect_err("git source refused");
    assert!(error.contains("someone/plugin"), "{error}");
    assert!(cloner.clones.lock().unwrap().is_empty(), "nothing fetched");
}

/// (g) A manifest whose entry resolves outside its plugin folder (an absolute
/// path, `..`, or a symlink out) is refused: the folder that was trusted or
/// allowed is not where the code would come from.
#[test]
fn an_entry_outside_the_plugin_folder_is_refused() {
    let outside = tempfile::tempdir().unwrap();
    let foreign = outside.path().join("foreign.py");
    std::fs::write(&foreign, "print('outside')\n").unwrap();
    let parent = tempfile::tempdir().unwrap();
    let cases = [
        ("absolute", foreign.display().to_string()),
        ("parent", "../foreign.py".to_string()),
        ("symlink", "linked.py".to_string()),
    ];
    for (name, entry) in cases {
        let dir = parent.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[plugin]\nid = \"{name}\"\nname = \"{name}\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"{entry}\"\n\n[[contributes.views]]\nid = \"{name}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
        std::fs::write(parent.path().join("foreign.py"), "print('outside')\n").unwrap();
        if name == "symlink" {
            std::os::unix::fs::symlink(&foreign, dir.join("linked.py")).unwrap();
        }
        let error = PluginRegistry::empty()
            .load_plugin(&dir)
            .expect_err("an entry outside the plugin folder must be refused");
        assert!(error.contains("outside"), "{name}: {error}");
    }
}

/// (c) Even an identical copy of a bundled plugin in a trusted workspace
/// folder does not own the id: discovery keeps the profile's bundled entry.
#[test]
fn an_identical_workspace_copy_never_owns_a_bundled_id() {
    let rig = Rig::new();
    extract_bundled("todo", &rig.plugins());
    extract_bundled("todo", &rig.repo_plugins());
    rig.admission().trust_folder_for_test(&rig.repo_plugins());
    for registry in [
        PluginRegistry::load_with_global(rig.repo.path(), &rig.plugins()),
        rig.load(),
    ] {
        let todo = registry.get("todo").expect("bundled todo");
        assert_eq!(todo.source, RegistrySource::Global);
        assert_eq!(
            crate::platform::path::canonical_or_self(&todo.plugin_dir),
            crate::platform::path::canonical_or_self(&rig.plugins().join("todo"))
        );
        assert!(matches!(
            rig.refusal(&registry, "todo"),
            Refusal::ShadowsBundled { .. }
        ));
    }
}

/// (e) Bundled-plugin development: an explicit path launch from a trusted
/// folder may run a tree with a bundled id; discovery of that same tree is
/// still refused, and an untrusted folder still asks.
#[test]
fn an_explicit_path_launch_of_a_bundled_id_needs_only_folder_trust() {
    let rig = Rig::new();
    let dev = tempfile::tempdir().unwrap();
    let checkout = write_plugin(dev.path(), "todo");
    let admission = rig.admission();
    assert!(matches!(
        admission.decide("todo", &checkout, Launch::ExplicitPath),
        Err(Refusal::UntrustedFolder { .. })
    ));
    admission.trust_folder_for_test(dev.path());
    assert_eq!(
        admission.decide("todo", &checkout, Launch::ExplicitPath),
        Ok(Admitted::TrustedFolder)
    );
    assert!(matches!(
        admission.decide("todo", &checkout, Launch::Discovery),
        Err(Refusal::ShadowsBundled { .. })
    ));
}

/// (h) The packager copies each maintained plugin from the checkout without
/// bytecode (`scripts/package.py` ignore patterns). That copy must count as
/// bundled whether or not the checkout held `__pycache__` when the binary was
/// built.
#[test]
fn a_profile_seeded_from_the_checkout_like_the_packager_asks_nothing() {
    fn copy_like_packager(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap().flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "__pycache__"
                || name_str == ".venv"
                || name_str == ".DS_Store"
                || name_str.ends_with(".pyc")
            {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                copy_like_packager(&path, &dst.join(&name));
            } else {
                std::fs::copy(&path, dst.join(&name)).unwrap();
            }
        }
    }
    let rig = Rig::new();
    let repo_plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&repo_plugins).unwrap().flatten() {
        if entry.path().join("manifest.toml").is_file() {
            let id = entry.file_name().to_string_lossy().into_owned();
            copy_like_packager(&entry.path(), &rig.plugins().join(&id));
            ids.push(id);
        }
    }
    let registry = rig.load();
    for id in &ids {
        assert!(
            registry.get(id).is_some(),
            "{id} asks on a packaged profile: {:?}",
            registry.skipped_reason(id)
        );
    }
    assert!(rig.monitor().list_pending().is_empty());
}

/// The plugin tree a Python pane runs, canonical.
fn pane_tree(h: &crate::testing::HostHarness, pane: crate::spatial::tiling::PaneId) -> PathBuf {
    h.app
        .windows
        .iter()
        .find_map(|window| window.panes.get(&pane))
        .and_then(|pane| pane.as_view())
        .and_then(|view| match &view.runtime {
            crate::host::pane::ViewRuntime::Python(runtime) => Some(
                crate::platform::path::canonical_or_self(runtime.plugin_dir()),
            ),
            _ => None,
        })
        .expect("a Python pane")
}

/// (e) A dev copy of a bundled plugin path-launched from a trusted folder runs
/// in its own pane only. Opening the plugin by id (Cmd+P) still launches and
/// then focuses the bundled copy, never the dev pane, and the dev launch does
/// not focus the bundled pane either (chess is focus_existing_in_context).
#[test]
fn a_dev_copy_never_stands_in_for_the_bundled_plugin_opened_by_id() {
    let mut h = crate::testing::HostHarness::new();
    let profile_plugins = h.profile_dir().join("plugins");
    let bundled = extract_bundled("chess", &profile_plugins);
    h.app.registries = crate::app::registry_views::RegistryViews::new_with_global(&profile_plugins);
    let dev = tempfile::tempdir().unwrap();
    let dev_chess = extract_bundled("chess", dev.path());
    let source = dev_chess.join("chess.py");
    let mut text = std::fs::read_to_string(&source).unwrap();
    text.push_str("\n# dev edit\n");
    std::fs::write(&source, text).unwrap();
    h.trust_plugin_folder(dev.path());
    let workspace = h.workspace_root();

    let open = |h: &mut crate::testing::HostHarness| h.state().open_panes;
    let before = open(&mut h);
    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace.clone()))
        .expect("bundled chess by id");
    let bundled_pane = *open(&mut h)
        .iter()
        .find(|id| !before.contains(id))
        .expect("bundled pane");
    assert_eq!(
        pane_tree(&h, bundled_pane),
        crate::platform::path::canonical_or_self(&bundled)
    );

    let before = open(&mut h);
    h.app
        .launch_view_by_path_with_layout(
            &dev_chess.to_string_lossy(),
            None,
            Some(workspace.clone()),
            &[],
        )
        .expect("dev chess by path from a trusted folder");
    let dev_pane = *open(&mut h)
        .iter()
        .find(|id| !before.contains(id))
        .expect("the dev launch opens its own pane, not the bundled one");
    assert_eq!(
        pane_tree(&h, dev_pane),
        crate::platform::path::canonical_or_self(&dev_chess)
    );

    let before = open(&mut h);
    h.app
        .launch_view_by_id_with_layout("chess", None, &[], Some(workspace))
        .expect("chess by id again");
    assert_eq!(open(&mut h), before, "by id focuses an existing pane");
    let focused = h.app.windows[h.app.active_window]
        .focused_pane
        .and_then(|tile| {
            h.app.windows[h.app.active_window]
                .tree
                .tiles
                .get_pane(&tile)
                .copied()
        })
        .expect("a focused pane");
    assert_eq!(
        focused, bundled_pane,
        "by id focuses the bundled pane, never the dev copy"
    );
    assert!(
        h.app
            .registries
            .view_for_root(&h.workspace_root())
            .get("chess")
            .is_some_and(|entry| {
                crate::platform::path::canonical_or_self(&entry.plugin_dir)
                    == crate::platform::path::canonical_or_self(&bundled)
            }),
        "the dev copy never enters the registry"
    );
}

/// (c) A workspace directory link that resolves to the bundled install itself
/// still never owns the id: the reservation applies to the scan source, before
/// the link's destination is classified.
#[test]
fn a_workspace_link_to_the_bundled_install_never_owns_its_id() {
    let rig = Rig::new();
    let bundled = extract_bundled("todo", &rig.plugins());
    std::os::unix::fs::symlink(&bundled, rig.repo_plugins().join("todo")).unwrap();
    rig.admission().trust_folder_for_test(&rig.repo_plugins());
    let registry = rig.load();
    let todo = registry.get("todo").expect("bundled todo");
    assert_eq!(todo.source, RegistrySource::Global);
    assert!(todo.workspace_root.is_none());
    assert!(matches!(
        rig.refusal(&registry, "todo"),
        Refusal::ShadowsBundled { .. }
    ));
}
