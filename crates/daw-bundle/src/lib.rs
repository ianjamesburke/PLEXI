//! Portable DAW project bundle (stint 0518): the on-disk format for a saved
//! project.
//!
//! A bundle is a directory holding [`PROJECT_FILE`] — the deterministically
//! serialized 0514 model ([`BundleDoc`]) — beside a [`MEDIA_DIR`] folder of
//! content-hash-named `.wav` assets. Every source path stored in the project
//! is either a `demo:` synthesized source or a **bundle-relative** media
//! reference ([`media_relpath`]); no absolute machine path is ever persisted,
//! so a bundle stays openable after it is moved or renamed.
//!
//! This crate is pure: it owns the format (hashing, layout, serialization,
//! validation) and performs no I/O. The host app resolves relative refs
//! against the open bundle directory and drives the reads/writes through the
//! SDK file effects.

use serde::{Deserialize, Serialize};

use plexi_daw_model::{DawModel, Project, Source, Transport, ValidationError};

/// File name of the serialized model at the bundle root.
pub const PROJECT_FILE: &str = "project.json";
/// Sub-directory holding content-hash-named `.wav` assets.
pub const MEDIA_DIR: &str = "media";

/// FNV-1a 64-bit content hash as a lowercase 16-char hex string.
///
/// A media asset's name is a pure function of its bytes, so identical content
/// hashes to the same name and dedupes on import; the hash is stable across
/// builds and platforms.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// Bundle-relative media path for a content hash: `media/<hash>.wav`.
#[must_use]
pub fn media_relpath(hash: &str) -> String {
    format!("{MEDIA_DIR}/{hash}.wav")
}

/// Whether a source path is a bundle-relative media reference — the only kind
/// of file-backed path a bundle persists. Rejects absolute paths and any
/// `..` traversal so a foreign bundle can never point outside its own tree.
#[must_use]
pub fn is_media_ref(path: &str) -> bool {
    path.starts_with(&format!("{MEDIA_DIR}/")) && !path.contains("..")
}

/// Prefix marking a synthesized source that carries no media file.
pub const DEMO_PREFIX: &str = "demo:";

/// Whether a source path may portably live in a bundle: a `demo:` synthesized
/// source, or a relative media ref. An absolute or foreign path is not portable
/// — it would break the moment the bundle moved machines — and is rejected on
/// both load and save.
#[must_use]
pub fn is_portable_path(path: &str) -> bool {
    path.starts_with(DEMO_PREFIX) || is_media_ref(path)
}

/// The distinct bundle-relative media files a project depends on, sorted for
/// deterministic write order and deduplicated (two sources with identical
/// content share one asset).
#[must_use]
pub fn media_refs(project: &Project) -> Vec<String> {
    let mut refs: Vec<String> = project
        .sources
        .iter()
        .map(|s| s.path.clone())
        .filter(|p| is_media_ref(p))
        .collect();
    refs.sort();
    refs.dedup();
    refs
}

/// Finds an existing source whose path is the given bundle-relative media
/// reference — the hit that makes import content-addressed (a re-imported file
/// reuses the source already registered for its hash instead of adding a
/// duplicate).
#[must_use]
pub fn source_with_ref<'a>(project: &'a Project, media_ref: &str) -> Option<&'a Source> {
    project.sources.iter().find(|s| s.path == media_ref)
}

/// The serialized project document: the pure 0514 model state — project data
/// plus transport. This is exactly what [`PROJECT_FILE`] holds; undo history
/// and the in-session revision counter deliberately never persist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleDoc {
    pub project: Project,
    pub transport: Transport,
}

impl BundleDoc {
    /// Snapshots the persistable state of a live model.
    #[must_use]
    pub fn from_model(model: &DawModel) -> Self {
        Self {
            project: model.project().clone(),
            transport: *model.transport(),
        }
    }

    /// Deterministic pretty JSON — the on-disk `project.json`. Two calls on
    /// equal documents produce byte-identical output (the save-determinism
    /// contract), because the model holds only ordered `Vec`s and scalars.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("daw bundle serialization failed: {e}"))
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json)
            .map_err(|e| format!("daw bundle deserialization failed: {e}"))
    }

    /// Rebuilds a live model, rejecting invariant-violating state (corrupted
    /// or hand-edited bundles) through the model's shared validation path.
    pub fn into_model(self) -> Result<DawModel, ValidationError> {
        DawModel::from_parts(self.project, self.transport)
    }

    /// Rejects a document that persists a non-portable source path (absolute or
    /// traversing) — the relative-or-`demo:`-only contract that keeps a bundle
    /// openable after it moves machines. Enforced on both load and save.
    pub fn validate_portable(&self) -> Result<(), String> {
        for source in &self.project.sources {
            if !is_portable_path(&source.path) {
                return Err(format!(
                    "non-portable source path in bundle: {} (import audio to copy it into media/)",
                    source.path
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plexi_daw_model::{DawCommand, SourceId, TrackId, TrackKind, TICKS_PER_BEAT};

    /// A model with one audio track carrying a bundle-relative media source and
    /// one demo source — the two path kinds a real bundle persists.
    fn seeded_model() -> DawModel {
        let beat = TICKS_PER_BEAT;
        let hash = content_hash(b"fake wav bytes");
        let media = media_relpath(&hash);
        let mut model = DawModel::new();
        for cmd in [
            DawCommand::AddTrack { kind: TrackKind::Audio, name: "Guitar".into() },
            DawCommand::AddSource { kind: TrackKind::Audio, path: media, duration: 4 * beat },
            DawCommand::AddClip {
                track: TrackId(1),
                source: SourceId(2),
                position: 0,
                length: 2 * beat,
                source_offset: 0,
            },
            DawCommand::AddTrack { kind: TrackKind::Audio, name: "Synth".into() },
            DawCommand::AddSource {
                kind: TrackKind::Audio,
                path: "demo:pluck".into(),
                duration: 2 * beat,
            },
            DawCommand::SetTempo { bpm: 128.0 },
            DawCommand::Seek { position: beat },
        ] {
            model.apply(cmd);
        }
        model
    }

    #[test]
    fn save_reopen_round_trips_the_model() {
        let model = seeded_model();
        let doc = BundleDoc::from_model(&model);
        let json = doc.to_json().unwrap();

        let reopened = BundleDoc::from_json(&json).unwrap().into_model().unwrap();
        assert_eq!(reopened.project(), model.project());
        assert_eq!(reopened.transport(), model.transport());
    }

    #[test]
    fn two_saves_of_the_same_model_are_byte_identical() {
        let doc = BundleDoc::from_model(&seeded_model());
        assert_eq!(doc.to_json().unwrap(), doc.to_json().unwrap());
    }

    #[test]
    fn persisted_project_holds_only_relative_or_demo_paths() {
        // The moved/renamed-bundle guarantee: nothing in project.json is an
        // absolute machine path, so re-parsing it from any directory works.
        let doc = BundleDoc::from_model(&seeded_model());
        let json = doc.to_json().unwrap();
        for source in &doc.project.sources {
            assert!(
                source.path.starts_with("demo:") || is_media_ref(&source.path),
                "non-portable source path persisted: {}",
                source.path
            );
            assert!(!source.path.starts_with('/'), "absolute path persisted");
        }
        // A reparse yields the same document regardless of any bundle location.
        assert_eq!(BundleDoc::from_json(&json).unwrap(), doc);
    }

    #[test]
    fn media_refs_are_sorted_deduped_and_exclude_demo() {
        let refs = media_refs(seeded_model().project());
        assert_eq!(refs.len(), 1);
        assert!(is_media_ref(&refs[0]));
        let mut sorted = refs.clone();
        sorted.sort();
        assert_eq!(refs, sorted);
    }

    #[test]
    fn content_hash_is_stable_deterministic_and_distinguishing() {
        assert_eq!(content_hash(b"same"), content_hash(b"same"));
        assert_ne!(content_hash(b"a"), content_hash(b"b"));
        assert_eq!(content_hash(b"x").len(), 16);
        // Known FNV-1a 64 vector for the empty input.
        assert_eq!(content_hash(b""), "cbf29ce484222325");
    }

    #[test]
    fn is_media_ref_rejects_absolute_and_traversal_paths() {
        assert!(is_media_ref("media/abc.wav"));
        assert!(!is_media_ref("/abs/media/abc.wav"));
        assert!(!is_media_ref("media/../escape.wav"));
        assert!(!is_media_ref("demo:pluck"));
    }

    #[test]
    fn validate_portable_rejects_absolute_source_paths() {
        let mut doc = BundleDoc::from_model(&seeded_model());
        // A seeded bundle is portable (demo: + media/ refs only).
        doc.validate_portable().unwrap();
        // A hand-edited absolute path is not.
        doc.project.sources[0].path = "/tmp/audio.wav".into();
        assert!(doc.validate_portable().is_err());
    }

    #[test]
    fn into_model_rejects_a_corrupt_bundle() {
        let mut doc = BundleDoc::from_model(&seeded_model());
        doc.project.tempo_bpm = 1_000_000.0; // outside TEMPO_MAX
        assert!(doc.into_model().is_err());
    }

    #[test]
    fn source_with_ref_finds_the_content_addressed_source() {
        let model = seeded_model();
        let hash = content_hash(b"fake wav bytes");
        let media = media_relpath(&hash);
        let found = source_with_ref(model.project(), &media);
        assert!(found.is_some());
        assert!(source_with_ref(model.project(), "media/absent.wav").is_none());
    }
}
