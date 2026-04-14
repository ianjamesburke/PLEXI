//! Integration test — spawn the photo-viewer binary and drive it through a
//! minimal Plexi protocol handshake. Verifies that the app decodes a fixture
//! PNG, emits draw commands, and exits cleanly on shutdown.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin_path() -> PathBuf {
    // Cargo sets CARGO_BIN_EXE_<name> for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_photo-viewer"))
}

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("red.png");
    p
}

#[test]
fn spawns_and_emits_frame_done() {
    let fixture = fixture_path();
    assert!(fixture.exists(), "fixture missing at {}", fixture.display());

    let mut child = Command::new(bin_path())
        .arg(&fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn photo-viewer");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    // init → render → shutdown
    writeln!(stdin, r#"{{"type":"init","width":800.0,"height":600.0,"pixels_per_point":2.0}}"#).unwrap();
    writeln!(stdin, r#"{{"type":"render","width":800.0,"height":600.0}}"#).unwrap();
    stdin.flush().unwrap();

    // Read lines until frame_done (with deadline)
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_rect = false;
    let mut saw_frame_done = false;
    loop {
        if Instant::now() > deadline {
            panic!("timed out waiting for frame_done");
        }
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read_line");
        if n == 0 { break; }
        if line.contains(r#""type":"rect""#) {
            saw_rect = true;
        }
        if line.contains(r#""type":"frame_done""#) {
            saw_frame_done = true;
            break;
        }
    }

    writeln!(stdin, r#"{{"type":"shutdown"}}"#).unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let status = child.wait().expect("wait");
    assert!(status.success() || status.code() == Some(0), "exit: {:?}", status);
    assert!(saw_rect, "expected at least one rect draw command");
    assert!(saw_frame_done, "expected frame_done terminator");
}
