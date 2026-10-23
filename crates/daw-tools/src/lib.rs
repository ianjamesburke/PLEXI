//! The `daw.*` connector tool surface (stint 0517): the typed API the
//! assistant drives a DAW app through.
//!
//! Pure over `plexi-daw-model` — specs plus dispatch, no engine, no host, no
//! I/O. Every mutation routes through [`DawModel::apply`]; a model rejection
//! surfaces as a tool error carrying the model's reason verbatim. The
//! embedding app re-prepares its engine when [`DawModel::revision`] changed
//! across a dispatch.
//!
//! Track arguments accept an integer id or a track name. Names resolve only
//! when unambiguous (one exact match, else one case-insensitive match);
//! every other case errors and names the candidate tracks. Ids are never
//! reused by the model, so a dangling id is reported as deleted — never
//! rebound.

use plexi_daw_model::{
    ApplyOutcome, DawCommand, DawModel, Project, SourceId, Track, TrackId, TrackKind,
    TEMPO_MAX, TEMPO_MIN, TICKS_PER_BEAT, VOLUME_MAX, VOLUME_MIN,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub const PROJECT_INFO: &str = "daw.project_info";
pub const LIST_TRACKS: &str = "daw.list_tracks";
pub const GET_TRACK: &str = "daw.get_track";
pub const TRANSPORT_STATE: &str = "daw.transport_state";
pub const ADD_TRACK: &str = "daw.add_track";
pub const SET_TRACK_VOLUME: &str = "daw.set_track_volume";
pub const MUTE_TRACK: &str = "daw.mute_track";
pub const SOLO_TRACK: &str = "daw.solo_track";
pub const SET_BPM: &str = "daw.set_bpm";
pub const ADD_CLIP: &str = "daw.add_clip";
pub const PLAY: &str = "daw.play";
pub const STOP: &str = "daw.stop";

/// One declared tool: name, LLM-facing description, JSON schemas, and
/// whether the tool only reads the model. `read_only` gates the host
/// permission prompt — a mutation must never carry `true`.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub read_only: bool,
}

/// Schema fragment for an id-or-name track reference.
fn track_ref_schema() -> Value {
    json!({
        "description": "Track id (integer) or track name (string). A name resolves only when it matches exactly one track (exact match first, then case-insensitive); ambiguous or unknown names error and list the candidates.",
        "anyOf": [
            { "type": "integer", "minimum": 1 },
            { "type": "string", "minLength": 1 }
        ]
    })
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

/// The full `daw.*` tool surface, 1:1 with `DawCommand` for mutations.
pub fn tools() -> Vec<ToolSpec> {
    let track_summary_props = json!({
        "id": { "type": "integer" },
        "name": { "type": "string" },
        "kind": { "type": "string", "enum": ["audio", "midi"] },
        "volume": { "type": "number" },
        "pan": { "type": "number" },
        "mute": { "type": "boolean" },
        "solo": { "type": "boolean" },
        "clip_count": { "type": "integer" }
    });
    let mutation_result_props = |extra: Value| -> Value {
        let mut props = json!({
            "outcome": { "type": "string", "enum": ["applied", "no_op"] },
            "revision": { "type": "integer" }
        });
        if let (Some(base), Some(add)) = (props.as_object_mut(), extra.as_object()) {
            for (k, v) in add {
                base.insert(k.clone(), v.clone());
            }
        }
        props
    };
    vec![
        ToolSpec {
            name: PROJECT_INFO,
            description: format!(
                "Read project-level DAW model state: tempo, revision, transport flag, and every \
                 registered media source (id, kind, path, duration in ticks; {TICKS_PER_BEAT} \
                 ticks per beat). Read this before daw.add_clip to pick a source id."
            ),
            input_schema: object_schema(json!({}), &[]),
            output_schema: object_schema(
                json!({
                    "tempo_bpm": { "type": "number" },
                    "revision": { "type": "integer" },
                    "playing": { "type": "boolean" },
                    "track_count": { "type": "integer" },
                    "source_count": { "type": "integer" },
                    "sources": { "type": "array", "items": object_schema(json!({
                        "id": { "type": "integer" },
                        "kind": { "type": "string", "enum": ["audio", "midi"] },
                        "path": { "type": "string" },
                        "duration": { "type": "integer" }
                    }), &["id", "kind", "path", "duration"]) }
                }),
                &["tempo_bpm", "revision", "playing", "track_count", "source_count", "sources"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: LIST_TRACKS,
            description: "List every track in the DAW model: id, name, kind, mixer state \
                          (volume, pan, mute, solo), and clip count. Use the ids with the \
                          daw.* mutation tools."
                .into(),
            input_schema: object_schema(json!({}), &[]),
            output_schema: object_schema(
                json!({ "tracks": { "type": "array", "items": object_schema(track_summary_props.clone(), &["id", "name", "kind", "volume", "pan", "mute", "solo", "clip_count"]) } }),
                &["tracks"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: GET_TRACK,
            description: "Read one track of the DAW model in full, including its clips (id, \
                          source id, timeline position, length, source_offset; all in ticks)."
                .into(),
            input_schema: object_schema(json!({ "track": track_ref_schema() }), &["track"]),
            output_schema: object_schema(
                {
                    let mut props = track_summary_props;
                    props["clips"] = json!({ "type": "array", "items": object_schema(json!({
                        "id": { "type": "integer" },
                        "source": { "type": "integer" },
                        "position": { "type": "integer" },
                        "length": { "type": "integer" },
                        "source_offset": { "type": "integer" }
                    }), &["id", "source", "position", "length", "source_offset"]) });
                    props
                },
                &["id", "name", "kind", "volume", "pan", "mute", "solo", "clip_count", "clips"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: TRANSPORT_STATE,
            description: "Read the DAW transport: playing flag, playhead position in ticks, \
                          and the loop range."
                .into(),
            input_schema: object_schema(json!({}), &[]),
            output_schema: object_schema(
                json!({
                    "playing": { "type": "boolean" },
                    "position": { "type": "integer" },
                    "loop_enabled": { "type": "boolean" },
                    "loop_start": { "type": "integer" },
                    "loop_end": { "type": "integer" }
                }),
                &["playing", "position", "loop_enabled", "loop_start", "loop_end"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: ADD_TRACK,
            description: "Add an empty track to the DAW model (DawCommand::AddTrack). Returns \
                          the new track's id."
                .into(),
            input_schema: object_schema(
                json!({
                    "kind": { "type": "string", "enum": ["audio", "midi"] },
                    "name": { "type": "string", "minLength": 1 }
                }),
                &["kind", "name"],
            ),
            output_schema: object_schema(
                mutation_result_props(json!({ "track_id": { "type": "integer" } })),
                &["outcome", "revision"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: SET_TRACK_VOLUME,
            description: format!(
                "Set a track's linear gain (DawCommand::SetTrackVolume). Finite values clamp \
                 to {VOLUME_MIN}..={VOLUME_MAX}; the result reports the applied value. \
                 Non-finite values are rejected."
            ),
            input_schema: object_schema(
                json!({
                    "track": track_ref_schema(),
                    "volume": { "type": "number", "minimum": VOLUME_MIN, "maximum": VOLUME_MAX }
                }),
                &["track", "volume"],
            ),
            output_schema: object_schema(
                mutation_result_props(json!({
                    "track_id": { "type": "integer" },
                    "volume": { "type": "number" }
                })),
                &["outcome", "revision", "track_id", "volume"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: MUTE_TRACK,
            description: "Set a track's mute flag (DawCommand::SetTrackMute). Muting is \
                          undoable and independent of solo."
                .into(),
            input_schema: object_schema(
                json!({ "track": track_ref_schema(), "mute": { "type": "boolean" } }),
                &["track", "mute"],
            ),
            output_schema: object_schema(
                mutation_result_props(json!({
                    "track_id": { "type": "integer" },
                    "mute": { "type": "boolean" }
                })),
                &["outcome", "revision", "track_id", "mute"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: SOLO_TRACK,
            description: "Set a track's solo flag (DawCommand::SetTrackSolo). When any track \
                          is soloed, only soloed tracks are audible."
                .into(),
            input_schema: object_schema(
                json!({ "track": track_ref_schema(), "solo": { "type": "boolean" } }),
                &["track", "solo"],
            ),
            output_schema: object_schema(
                mutation_result_props(json!({
                    "track_id": { "type": "integer" },
                    "solo": { "type": "boolean" }
                })),
                &["outcome", "revision", "track_id", "solo"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: SET_BPM,
            description: format!(
                "Set the project tempo (DawCommand::SetTempo). Finite values clamp to \
                 {TEMPO_MIN}..={TEMPO_MAX}; the result reports the applied value. Non-finite \
                 values are rejected."
            ),
            input_schema: object_schema(
                json!({ "bpm": { "type": "number", "minimum": TEMPO_MIN, "maximum": TEMPO_MAX } }),
                &["bpm"],
            ),
            output_schema: object_schema(
                mutation_result_props(json!({ "tempo_bpm": { "type": "number" } })),
                &["outcome", "revision", "tempo_bpm"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: ADD_CLIP,
            description: format!(
                "Place a clip of a registered source on a track (DawCommand::AddClip). The \
                 source kind must match the track kind and source_offset + length must fit \
                 inside the source's duration. All values in ticks ({TICKS_PER_BEAT} per \
                 beat). Returns the new clip's id. Read daw.project_info for source ids."
            ),
            input_schema: object_schema(
                json!({
                    "track": track_ref_schema(),
                    "source": { "type": "integer", "minimum": 1, "description": "Source id from daw.project_info" },
                    "position": { "type": "integer", "minimum": 0, "description": "Timeline start in ticks" },
                    "length": { "type": "integer", "minimum": 1, "description": "Clip length in ticks" },
                    "source_offset": { "type": "integer", "minimum": 0, "description": "Offset into the source in ticks; defaults to 0" }
                }),
                &["track", "source", "position", "length"],
            ),
            output_schema: object_schema(
                mutation_result_props(json!({ "clip_id": { "type": "integer" } })),
                &["outcome", "revision"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: PLAY,
            description: "Start transport playback from the current playhead position \
                          (DawCommand::Play). Not undoable."
                .into(),
            input_schema: object_schema(json!({}), &[]),
            output_schema: object_schema(
                mutation_result_props(json!({
                    "playing": { "type": "boolean" },
                    "position": { "type": "integer" }
                })),
                &["outcome", "revision", "playing", "position"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: STOP,
            description: "Stop transport playback; the playhead position is untouched \
                          (DawCommand::Stop). Not undoable."
                .into(),
            input_schema: object_schema(json!({}), &[]),
            output_schema: object_schema(
                mutation_result_props(json!({
                    "playing": { "type": "boolean" },
                    "position": { "type": "integer" }
                })),
                &["outcome", "revision", "playing", "position"],
            ),
            read_only: false,
        },
    ]
}

// ─── Inputs ──────────────────────────────────────────────────────────────────

/// Id-or-name track reference; resolution rules in [`resolve_track`].
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TrackRef {
    Id(u64),
    Name(String),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KindInput {
    Audio,
    Midi,
}

impl From<KindInput> for TrackKind {
    fn from(k: KindInput) -> Self {
        match k {
            KindInput::Audio => TrackKind::Audio,
            KindInput::Midi => TrackKind::Midi,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackInput {
    track: TrackRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTrackInput {
    kind: KindInput,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetVolumeInput {
    track: TrackRef,
    volume: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MuteInput {
    track: TrackRef,
    mute: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoloInput {
    track: TrackRef,
    solo: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetBpmInput {
    bpm: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddClipInput {
    track: TrackRef,
    source: u64,
    position: u64,
    length: u64,
    #[serde(default)]
    source_offset: u64,
}

fn parse<'a, T: Deserialize<'a>>(tool: &str, input_json: &'a str) -> Result<T, String> {
    let body = if input_json.trim().is_empty() { "{}" } else { input_json };
    serde_json::from_str(body).map_err(|e| format!("{tool} input: {e}"))
}

// ─── Track resolution ────────────────────────────────────────────────────────

fn describe(t: &Track) -> String {
    format!("\"{}\" (id {})", t.name, t.id.0)
}

fn listing(tracks: &[&Track]) -> String {
    tracks.iter().map(|t| describe(t)).collect::<Vec<_>>().join(", ")
}

/// Resolves an id-or-name reference against the project's tracks.
///
/// - Id: must exist. The model never reuses ids, so a missing id means the
///   track was deleted — the error says so instead of guessing.
/// - Name: resolves only when unambiguous — exactly one exact match, else
///   exactly one case-insensitive match. Anything else errors and names the
///   candidate tracks (substring matches when nothing matched outright).
fn resolve_track(project: &Project, r: &TrackRef) -> Result<TrackId, String> {
    match r {
        TrackRef::Id(raw) => {
            let id = TrackId(*raw);
            if project.track(id).is_some() {
                Ok(id)
            } else {
                Err(format!(
                    "track id {raw} not found; DAW ids are never reused, so a missing id \
                     means the track was deleted"
                ))
            }
        }
        TrackRef::Name(name) => {
            let exact: Vec<&Track> =
                project.tracks.iter().filter(|t| t.name == *name).collect();
            if let [only] = exact[..] {
                return Ok(only.id);
            }
            if exact.len() > 1 {
                return Err(format!(
                    "track name \"{name}\" is ambiguous: matches {}; call again with the id",
                    listing(&exact)
                ));
            }
            let lower = name.to_lowercase();
            let ci: Vec<&Track> = project
                .tracks
                .iter()
                .filter(|t| t.name.to_lowercase() == lower)
                .collect();
            if let [only] = ci[..] {
                return Ok(only.id);
            }
            if ci.len() > 1 {
                return Err(format!(
                    "track name \"{name}\" is ambiguous (case-insensitive): matches {}; \
                     call again with the id",
                    listing(&ci)
                ));
            }
            let candidates: Vec<&Track> = project
                .tracks
                .iter()
                .filter(|t| {
                    let tl = t.name.to_lowercase();
                    tl.contains(&lower) || lower.contains(&tl)
                })
                .collect();
            if !candidates.is_empty() {
                return Err(format!(
                    "no track named \"{name}\"; closest matches: {}",
                    listing(&candidates)
                ));
            }
            if project.tracks.is_empty() {
                Err(format!("no track named \"{name}\"; the project has no tracks"))
            } else {
                let all: Vec<&Track> = project.tracks.iter().collect();
                Err(format!(
                    "no track named \"{name}\"; existing tracks: {}",
                    listing(&all)
                ))
            }
        }
    }
}

// ─── Dispatch ────────────────────────────────────────────────────────────────

fn kind_str(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "audio",
        TrackKind::Midi => "midi",
    }
}

fn track_summary(t: &Track) -> Value {
    json!({
        "id": t.id.0,
        "name": t.name,
        "kind": kind_str(t.kind),
        "volume": t.mixer.volume,
        "pan": t.mixer.pan,
        "mute": t.mixer.mute,
        "solo": t.mixer.solo,
        "clip_count": t.clips.len(),
    })
}

/// Applies a command, mapping the outcome to the result's `outcome` string
/// and a model rejection to a tool error with the model's reason verbatim.
fn apply(model: &mut DawModel, cmd: DawCommand) -> Result<&'static str, String> {
    match model.apply(cmd) {
        ApplyOutcome::Applied => Ok("applied"),
        ApplyOutcome::NoOp => Ok("no_op"),
        ApplyOutcome::Rejected(reason) => Err(reason),
    }
}

/// Handles one `daw.*` tool call against the model. Returns `None` for names
/// outside this surface so the embedding app can route its own tools.
///
/// Mutations go through [`DawModel::apply`] only; callers detect model
/// changes by comparing [`DawModel::revision`] around the call.
pub fn dispatch(
    model: &mut DawModel,
    name: &str,
    input_json: &str,
) -> Option<Result<Value, String>> {
    let result = match name {
        PROJECT_INFO => parse::<EmptyInput>(name, input_json).map(|_| {
            let p = model.project();
            json!({
                "tempo_bpm": p.tempo_bpm,
                "revision": model.revision(),
                "playing": model.transport().playing,
                "track_count": p.tracks.len(),
                "source_count": p.sources.len(),
                "sources": p.sources.iter().map(|s| json!({
                    "id": s.id.0,
                    "kind": kind_str(s.kind),
                    "path": s.path,
                    "duration": s.duration,
                })).collect::<Vec<_>>(),
            })
        }),
        LIST_TRACKS => parse::<EmptyInput>(name, input_json).map(|_| {
            json!({
                "tracks": model.project().tracks.iter().map(track_summary).collect::<Vec<_>>(),
            })
        }),
        GET_TRACK => parse::<TrackInput>(name, input_json).and_then(|input| {
            let id = resolve_track(model.project(), &input.track)?;
            let t = model
                .project()
                .track(id)
                .expect("resolve_track returned an existing id");
            let mut out = track_summary(t);
            out["clips"] = t
                .clips
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id.0,
                        "source": c.source.0,
                        "position": c.position,
                        "length": c.length,
                        "source_offset": c.source_offset,
                    })
                })
                .collect::<Vec<_>>()
                .into();
            Ok(out)
        }),
        TRANSPORT_STATE => parse::<EmptyInput>(name, input_json).map(|_| {
            let t = model.transport();
            json!({
                "playing": t.playing,
                "position": t.position,
                "loop_enabled": t.loop_enabled,
                "loop_start": t.loop_start,
                "loop_end": t.loop_end,
            })
        }),
        ADD_TRACK => parse::<AddTrackInput>(name, input_json).and_then(|input| {
            let expected_id = model.project().next_id;
            let outcome = apply(
                model,
                DawCommand::AddTrack {
                    kind: input.kind.into(),
                    name: input.name,
                },
            )?;
            let mut out = json!({
                "outcome": outcome,
                "revision": model.revision(),
            });
            if outcome == "applied" {
                out["track_id"] = expected_id.into();
            }
            Ok(out)
        }),
        SET_TRACK_VOLUME => parse::<SetVolumeInput>(name, input_json).and_then(|input| {
            let id = resolve_track(model.project(), &input.track)?;
            let outcome = apply(
                model,
                DawCommand::SetTrackVolume {
                    track: id,
                    volume: input.volume,
                },
            )?;
            let applied = model
                .project()
                .track(id)
                .expect("track existed before a mixer set")
                .mixer
                .volume;
            Ok(json!({
                "outcome": outcome,
                "revision": model.revision(),
                "track_id": id.0,
                "volume": applied,
            }))
        }),
        MUTE_TRACK => parse::<MuteInput>(name, input_json).and_then(|input| {
            let id = resolve_track(model.project(), &input.track)?;
            let outcome = apply(
                model,
                DawCommand::SetTrackMute {
                    track: id,
                    mute: input.mute,
                },
            )?;
            Ok(json!({
                "outcome": outcome,
                "revision": model.revision(),
                "track_id": id.0,
                "mute": input.mute,
            }))
        }),
        SOLO_TRACK => parse::<SoloInput>(name, input_json).and_then(|input| {
            let id = resolve_track(model.project(), &input.track)?;
            let outcome = apply(
                model,
                DawCommand::SetTrackSolo {
                    track: id,
                    solo: input.solo,
                },
            )?;
            Ok(json!({
                "outcome": outcome,
                "revision": model.revision(),
                "track_id": id.0,
                "solo": input.solo,
            }))
        }),
        SET_BPM => parse::<SetBpmInput>(name, input_json).and_then(|input| {
            let outcome = apply(model, DawCommand::SetTempo { bpm: input.bpm })?;
            Ok(json!({
                "outcome": outcome,
                "revision": model.revision(),
                "tempo_bpm": model.project().tempo_bpm,
            }))
        }),
        ADD_CLIP => parse::<AddClipInput>(name, input_json).and_then(|input| {
            let track = resolve_track(model.project(), &input.track)?;
            let expected_id = model.project().next_id;
            let outcome = apply(
                model,
                DawCommand::AddClip {
                    track,
                    source: SourceId(input.source),
                    position: input.position,
                    length: input.length,
                    source_offset: input.source_offset,
                },
            )?;
            let mut out = json!({
                "outcome": outcome,
                "revision": model.revision(),
            });
            if outcome == "applied" {
                out["clip_id"] = expected_id.into();
            }
            Ok(out)
        }),
        PLAY => parse::<EmptyInput>(name, input_json).and_then(|_| {
            let outcome = apply(model, DawCommand::Play)?;
            Ok(transport_result(model, &outcome))
        }),
        STOP => parse::<EmptyInput>(name, input_json).and_then(|_| {
            let outcome = apply(model, DawCommand::Stop)?;
            Ok(transport_result(model, &outcome))
        }),
        _ => return None,
    };
    Some(result)
}

fn transport_result(model: &DawModel, outcome: &str) -> Value {
    json!({
        "outcome": outcome,
        "revision": model.revision(),
        "playing": model.transport().playing,
        "position": model.transport().position,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> DawModel {
        let mut model = DawModel::new();
        for cmd in [
            DawCommand::AddTrack { kind: TrackKind::Audio, name: "Pluck".into() },
            DawCommand::AddSource { kind: TrackKind::Audio, path: "demo:pluck".into(), duration: 4 * TICKS_PER_BEAT },
            DawCommand::AddTrack { kind: TrackKind::Midi, name: "Bass".into() },
            DawCommand::AddSource { kind: TrackKind::Midi, path: "demo:arp".into(), duration: 4 * TICKS_PER_BEAT },
        ] {
            assert_eq!(model.apply(cmd), ApplyOutcome::Applied);
        }
        model
    }

    fn call(model: &mut DawModel, name: &str, input: &str) -> Result<Value, String> {
        dispatch(model, name, input).unwrap_or_else(|| panic!("{name} not dispatched"))
    }

    #[test]
    fn every_read_tool_is_read_only_and_every_mutation_is_not() {
        let read_only: Vec<&str> = tools().iter().filter(|t| t.read_only).map(|t| t.name).collect();
        assert_eq!(
            read_only,
            vec![PROJECT_INFO, LIST_TRACKS, GET_TRACK, TRANSPORT_STATE],
            "exactly the four read tools may auto-grant"
        );
        // Prove the flag is truthful: no read-only tool changes the revision.
        let mut model = seeded();
        for name in [PROJECT_INFO, LIST_TRACKS, TRANSPORT_STATE] {
            let before = model.revision();
            call(&mut model, name, "{}").unwrap();
            assert_eq!(model.revision(), before, "{name} mutated the model");
        }
        let before = model.revision();
        call(&mut model, GET_TRACK, r#"{"track": 1}"#).unwrap();
        assert_eq!(model.revision(), before);
    }

    #[test]
    fn schemas_are_tight_objects() {
        for tool in tools() {
            assert!(tool.name.starts_with("daw."), "{} not namespaced", tool.name);
            for (label, schema) in [("input", &tool.input_schema), ("output", &tool.output_schema)] {
                assert_eq!(
                    schema["type"], "object",
                    "{} {label} schema must be an object",
                    tool.name
                );
                assert!(schema["properties"].is_object());
            }
            assert_eq!(
                tool.input_schema["additionalProperties"], false,
                "{} input schema must reject unknown fields",
                tool.name
            );
            assert!(!tool.description.is_empty());
        }
    }

    #[test]
    fn unknown_names_are_not_handled() {
        let mut model = seeded();
        assert!(dispatch(&mut model, "daw_mixdown", "{}").is_none());
        assert!(dispatch(&mut model, "daw.remove_track", "{}").is_none());
    }

    #[test]
    fn add_track_then_list_reflects_it() {
        let mut model = seeded();
        let out = call(&mut model, ADD_TRACK, r#"{"kind":"midi","name":"Lead"}"#).unwrap();
        assert_eq!(out["outcome"], "applied");
        let id = out["track_id"].as_u64().expect("new track id");
        assert_eq!(model.project().tracks.last().unwrap().id.0, id);

        let listed = call(&mut model, LIST_TRACKS, "{}").unwrap();
        let tracks = listed["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 3);
        assert!(tracks.iter().any(|t| t["name"] == "Lead" && t["id"] == id && t["kind"] == "midi"));
    }

    #[test]
    fn set_volume_by_name_resolves_case_insensitively() {
        let mut model = seeded();
        let out =
            call(&mut model, SET_TRACK_VOLUME, r#"{"track":"bass","volume":0.25}"#).unwrap();
        assert_eq!(out["outcome"], "applied");
        assert_eq!(out["track_id"], 3);
        assert!((out["volume"].as_f64().unwrap() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_volume_clamps_and_reports_applied_value() {
        let mut model = seeded();
        let out = call(&mut model, SET_TRACK_VOLUME, r#"{"track":1,"volume":9.5}"#).unwrap();
        assert_eq!(out["outcome"], "applied");
        let applied = out["volume"].as_f64().unwrap() as f32;
        assert_eq!(applied, VOLUME_MAX);
    }

    #[test]
    fn dangling_track_id_reports_deleted_never_rebinds() {
        let mut model = seeded();
        assert_eq!(
            model.apply(DawCommand::RemoveTrack { track: TrackId(1) }),
            ApplyOutcome::Applied
        );
        let err = call(&mut model, GET_TRACK, r#"{"track":1}"#).unwrap_err();
        assert!(err.contains("track id 1 not found"), "{err}");
        assert!(err.contains("never reused"), "{err}");
    }

    #[test]
    fn ambiguous_name_errors_and_names_candidates() {
        let mut model = seeded();
        call(&mut model, ADD_TRACK, r#"{"kind":"audio","name":"Bass"}"#).unwrap();
        let err = call(&mut model, MUTE_TRACK, r#"{"track":"Bass","mute":true}"#).unwrap_err();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("id 3") && err.contains("id 5"), "{err}");
    }

    #[test]
    fn unknown_name_errors_with_candidates() {
        let mut model = seeded();
        let err = call(&mut model, GET_TRACK, r#"{"track":"bas"}"#).unwrap_err();
        assert!(err.contains("no track named \"bas\""), "{err}");
        assert!(err.contains("\"Bass\" (id 3)"), "{err}");

        let err = call(&mut model, GET_TRACK, r#"{"track":"zzz"}"#).unwrap_err();
        assert!(err.contains("existing tracks"), "{err}");
        assert!(err.contains("\"Pluck\" (id 1)"), "{err}");
    }

    #[test]
    fn model_rejections_surface_verbatim() {
        let mut model = seeded();
        // Kind mismatch: MIDI source 4 onto audio track 1. Capture the
        // model's own reason on a clone, then require the identical string
        // from the tool path.
        let mut twin = seeded();
        let ApplyOutcome::Rejected(reason) = twin.apply(DawCommand::AddClip {
            track: TrackId(1),
            source: SourceId(4),
            position: 0,
            length: TICKS_PER_BEAT,
            source_offset: 0,
        }) else {
            panic!("expected the model to reject a kind-mismatched clip");
        };
        let err = call(
            &mut model,
            ADD_CLIP,
            &format!(r#"{{"track":1,"source":4,"position":0,"length":{TICKS_PER_BEAT}}}"#),
        )
        .unwrap_err();
        assert_eq!(err, reason, "tool error must carry the model's reason verbatim");
    }

    #[test]
    fn add_clip_reports_new_clip_id_and_get_track_shows_it() {
        let mut model = seeded();
        let out = call(
            &mut model,
            ADD_CLIP,
            &format!(r#"{{"track":"Pluck","source":2,"position":0,"length":{TICKS_PER_BEAT}}}"#),
        )
        .unwrap();
        assert_eq!(out["outcome"], "applied");
        let clip_id = out["clip_id"].as_u64().expect("new clip id");

        let track = call(&mut model, GET_TRACK, r#"{"track":"Pluck"}"#).unwrap();
        let clips = track["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0]["id"], clip_id);
        assert_eq!(clips[0]["source"], 2);
    }

    #[test]
    fn set_bpm_clamps_and_reports_applied_tempo() {
        let mut model = seeded();
        let out = call(&mut model, SET_BPM, r#"{"bpm":5000}"#).unwrap();
        assert_eq!(out["outcome"], "applied");
        assert_eq!(out["tempo_bpm"].as_f64().unwrap(), TEMPO_MAX);

        let err = call(&mut model, SET_BPM, r#"{"bpm":"fast"}"#).unwrap_err();
        assert!(err.contains("daw.set_bpm input"), "{err}");
    }

    #[test]
    fn play_stop_round_trip_keeps_position() {
        let mut model = seeded();
        let out = call(&mut model, PLAY, "").unwrap();
        assert_eq!(out["outcome"], "applied");
        assert_eq!(out["playing"], true);

        // Playing again is a NoOp, not an error, and not a rejection.
        let out = call(&mut model, PLAY, "{}").unwrap();
        assert_eq!(out["outcome"], "no_op");

        let out = call(&mut model, STOP, "{}").unwrap();
        assert_eq!(out["outcome"], "applied");
        assert_eq!(out["playing"], false);
        assert_eq!(out["position"], 0);

        let state = call(&mut model, TRANSPORT_STATE, "{}").unwrap();
        assert_eq!(state["playing"], false);
    }

    #[test]
    fn unknown_fields_are_rejected_with_the_field_named() {
        let mut model = seeded();
        let err = call(
            &mut model,
            ADD_TRACK,
            r#"{"kind":"midi","name":"X","color":"red"}"#,
        )
        .unwrap_err();
        assert!(err.contains("color"), "{err}");
        assert_eq!(model.project().tracks.len(), 2, "rejected input must not mutate");
    }

    #[test]
    fn empty_and_missing_input_bodies_parse_for_no_arg_tools() {
        let mut model = seeded();
        assert!(call(&mut model, PROJECT_INFO, "").is_ok());
        assert!(call(&mut model, PROJECT_INFO, "  ").is_ok());
        let info = call(&mut model, PROJECT_INFO, "{}").unwrap();
        assert_eq!(info["source_count"], 2);
        assert_eq!(info["sources"][0]["id"], 2);
        assert_eq!(info["sources"][0]["kind"], "audio");
    }
}
