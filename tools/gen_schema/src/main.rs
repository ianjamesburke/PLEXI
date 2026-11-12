//! Generates the canonical PGAP JSON Schema artifact.
//! Usage: cargo run -p gen_schema > sdk/protocol/pgap.schema.json

use plexi::protocol::{HostRequest, PlexiEvent};

fn main() {
    let schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "protocol": "pgap/4",
        "version": 3,
        "definitions": {
            "PlexiEvent": schemars::schema_for!(PlexiEvent),
            "HostRequest": schemars::schema_for!(HostRequest),
        }
    });
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
