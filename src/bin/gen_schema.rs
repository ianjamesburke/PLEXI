//! Generates the canonical PGAP JSON Schema artifact.
//! Usage: cargo run --bin gen_schema > sdk/protocol/pgap.schema.json

use plexi::app_protocol::{ControlCommand, HostCommand, PlexiEvent, RenderCommand};

fn main() {
    let schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "protocol": "pgap/3",
        "version": 3,
        "definitions": {
            "PlexiEvent": schemars::schema_for!(PlexiEvent),
            "RenderCommand": schemars::schema_for!(RenderCommand),
            "HostCommand": schemars::schema_for!(HostCommand),
            "ControlCommand": schemars::schema_for!(ControlCommand),
        }
    });
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
