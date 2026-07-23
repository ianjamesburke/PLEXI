// The `jukebox.*` connector surface (stint 0513) — the assistant-callable
// tools, and the single dispatch that enforces them. Specs and dispatch live
// together so a tool's declared `read_only` flag and its actual effect can
// never drift apart: a read-only tool here provably calls only `&self`
// accessors, and every mutating tool takes `&mut Jukebox`.
//
// `read_only` is load-bearing security state: the host auto-grants read-only
// calls but prompts for mutating ones, so a mutation mislabeled read-only
// would bypass the permission prompt. The `read_only` flags below are checked
// against this invariant by the host harness test.

use serde_json::{json, Value};

use crate::model::Jukebox;

/// One declared tool: the wire name, human description, JSON-Schema strings,
/// and the read-only flag the host uses to decide auto-grant vs. prompt.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: &'static str,
    pub output_schema: &'static str,
    pub read_only: bool,
}

pub const TOOL_LIST_FILES: &str = "jukebox.list_files";
pub const TOOL_NOW_PLAYING: &str = "jukebox.now_playing";
pub const TOOL_PLAY: &str = "jukebox.play";
pub const TOOL_PAUSE: &str = "jukebox.pause";
pub const TOOL_NEXT: &str = "jukebox.next";
pub const TOOL_SET_VOLUME: &str = "jukebox.set_volume";

/// The full tool set, declared verbatim at `init`. Read-only tools first,
/// then mutating ones — order is stable so the harness can assert it.
#[must_use]
pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: TOOL_LIST_FILES,
            description: "List the jukebox playlist: index, name, source, load state, and \
                          duration for every track.",
            input_schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
            output_schema: r#"{"type":"object","properties":{"count":{"type":"integer"},"tracks":{"type":"array"}},"required":["count","tracks"]}"#,
            read_only: true,
        },
        ToolSpec {
            name: TOOL_NOW_PLAYING,
            description: "Report the current track: index, name, whether it is playing, volume, \
                          and playhead/duration in milliseconds.",
            input_schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
            output_schema: r#"{"type":"object","properties":{"index":{"type":"integer"},"playing":{"type":"boolean"},"volume":{"type":"number"}},"required":["index","playing"]}"#,
            read_only: true,
        },
        ToolSpec {
            name: TOOL_PLAY,
            description: "Start (or resume) playback of the current track. No-op on an empty \
                          playlist.",
            input_schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
            output_schema: r#"{"type":"object","properties":{"playing":{"type":"boolean"},"index":{"type":"integer"}},"required":["playing"]}"#,
            read_only: false,
        },
        ToolSpec {
            name: TOOL_PAUSE,
            description: "Pause playback, holding the current playhead position.",
            input_schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
            output_schema: r#"{"type":"object","properties":{"playing":{"type":"boolean"}},"required":["playing"]}"#,
            read_only: false,
        },
        ToolSpec {
            name: TOOL_NEXT,
            description: "Advance to the next track (wraps to the first after the last) and \
                          rewind its playhead.",
            input_schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
            output_schema: r#"{"type":"object","properties":{"index":{"type":"integer"},"name":{"type":["string","null"]}},"required":["index"]}"#,
            read_only: false,
        },
        ToolSpec {
            name: TOOL_SET_VOLUME,
            description: "Set output volume in [0.0, 2.0] (1.0 = unity). Values outside the \
                          range are clamped.",
            input_schema: r#"{"type":"object","properties":{"volume":{"type":"number","minimum":0.0,"maximum":2.0}},"required":["volume"],"additionalProperties":false}"#,
            output_schema: r#"{"type":"object","properties":{"volume":{"type":"number"}},"required":["volume"]}"#,
            read_only: false,
        },
    ]
}

/// Routes one `jukebox.*` call to the model. Returns `None` for names outside
/// the surface (so the caller can fall through), `Some(Ok(json))` on success,
/// or `Some(Err(reason))` on a malformed call. Mutating tools mutate `jb`;
/// read-only tools only read it.
#[must_use]
pub fn dispatch(jb: &mut Jukebox, name: &str, input_json: &str) -> Option<Result<Value, String>> {
    let result = match name {
        TOOL_LIST_FILES => Ok(jb.list_files_value()),
        TOOL_NOW_PLAYING => Ok(jb.now_playing_value()),
        TOOL_PLAY => {
            jb.play();
            Ok(json!({ "playing": jb.is_playing(), "index": jb.current_index() }))
        }
        TOOL_PAUSE => {
            jb.pause();
            Ok(json!({ "playing": jb.is_playing() }))
        }
        TOOL_NEXT => {
            jb.next();
            let name = jb
                .tracks()
                .get(jb.current_index())
                .map(|t| Value::String(t.name.clone()))
                .unwrap_or(Value::Null);
            Ok(json!({ "index": jb.current_index(), "name": name }))
        }
        TOOL_SET_VOLUME => match parse_volume(input_json) {
            Ok(volume) => {
                jb.set_volume(volume);
                Ok(json!({ "volume": jb.volume() }))
            }
            Err(e) => Err(e),
        },
        _ => return None,
    };
    Some(result)
}

fn parse_volume(input_json: &str) -> Result<f32, String> {
    let value: Value = if input_json.trim().is_empty() {
        return Err("jukebox.set_volume requires a \"volume\" number".to_string());
    } else {
        serde_json::from_str(input_json).map_err(|e| format!("jukebox.set_volume input: {e}"))?
    };
    value
        .get("volume")
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .ok_or_else(|| "jukebox.set_volume requires a \"volume\" number".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_flags_match_the_surface_spec() {
        let read_only: Vec<&str> = tools()
            .iter()
            .filter(|t| t.read_only)
            .map(|t| t.name)
            .collect();
        assert_eq!(read_only, vec![TOOL_LIST_FILES, TOOL_NOW_PLAYING]);

        let mutating: Vec<&str> = tools()
            .iter()
            .filter(|t| !t.read_only)
            .map(|t| t.name)
            .collect();
        assert_eq!(
            mutating,
            vec![TOOL_PLAY, TOOL_PAUSE, TOOL_NEXT, TOOL_SET_VOLUME]
        );
    }

    #[test]
    fn every_schema_is_valid_json() {
        for spec in tools() {
            let _: Value = serde_json::from_str(spec.input_schema)
                .unwrap_or_else(|e| panic!("{} input schema: {e}", spec.name));
            let _: Value = serde_json::from_str(spec.output_schema)
                .unwrap_or_else(|e| panic!("{} output schema: {e}", spec.name));
        }
    }

    #[test]
    fn dispatch_unknown_tool_is_none() {
        let mut jb = Jukebox::new(48_000, 2);
        assert!(dispatch(&mut jb, "daw.play", "{}").is_none());
    }

    #[test]
    fn play_pause_next_mutate_the_model() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", vec![1.0; 20]);
        jb.add_loaded("b", "demo:b", vec![1.0; 20]);

        let out = dispatch(&mut jb, TOOL_PLAY, "{}").unwrap().unwrap();
        assert_eq!(out["playing"], true);
        assert!(jb.is_playing());

        let out = dispatch(&mut jb, TOOL_NEXT, "{}").unwrap().unwrap();
        assert_eq!(out["index"], 1);
        assert_eq!(out["name"], "b");

        let out = dispatch(&mut jb, TOOL_PAUSE, "{}").unwrap().unwrap();
        assert_eq!(out["playing"], false);
        assert!(!jb.is_playing());
    }

    #[test]
    fn set_volume_reads_and_clamps_input() {
        let mut jb = Jukebox::new(48_000, 2);
        let out = dispatch(&mut jb, TOOL_SET_VOLUME, r#"{"volume":0.3}"#)
            .unwrap()
            .unwrap();
        assert_eq!(out["volume"], 0.3_f64 as f32 as f64);
        assert!((jb.volume() - 0.3).abs() < 1e-6);

        let out = dispatch(&mut jb, TOOL_SET_VOLUME, r#"{"volume":9.0}"#)
            .unwrap()
            .unwrap();
        assert_eq!(out["volume"], 2.0);
    }

    #[test]
    fn set_volume_without_argument_errors() {
        let mut jb = Jukebox::new(48_000, 2);
        assert!(dispatch(&mut jb, TOOL_SET_VOLUME, "{}").unwrap().is_err());
        assert!(dispatch(&mut jb, TOOL_SET_VOLUME, "").unwrap().is_err());
    }

    #[test]
    fn read_only_tools_return_expected_shape() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", vec![1.0; 20]);
        let out = dispatch(&mut jb, TOOL_LIST_FILES, "{}").unwrap().unwrap();
        assert_eq!(out["count"], 1);
        let out = dispatch(&mut jb, TOOL_NOW_PLAYING, "{}").unwrap().unwrap();
        assert_eq!(out["index"], 0);
        assert_eq!(out["playing"], false);
    }
}
