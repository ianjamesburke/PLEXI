//! Routing tests for the v3.4 CoreMIDI capability (#320).
//!
//! Two paths under test:
//!   1. App without `midi.in` / `midi.out` — synchronous denial response
//!      lands on `outbound_events`. No device dispatch occurs.
//!   2. App with the capability — `MockMidiDevice` records the open and
//!      the routing layer queues `MidiInputOpened` on success.
use super::super::*;
use crate::app_protocol::{HostCommand, PlexiEvent};
use crate::midi::MockMidiDevice;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

fn make_app(capabilities: HashSet<Capability>) -> Option<ProcessApp> {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(PathBuf::from)?;
    let workspace_root = std::env::temp_dir();
    ProcessApp::launch(
        "test_midi",
        "Test MIDI",
        &sh,
        &workspace_root,
        &["-c".to_string(), "sleep 1".to_string()],
        workspace_root.clone(),
        capabilities,
        false,
        None,
    )
    .ok()
}

#[test]
fn denied_app_gets_capability_denied_response() {
    // App without `midi.in`: route_command must immediately queue
    // a MidiInputError with "capability denied" and never touch the
    // device. The send path mirrors the same contract for `midi.out`.
    let Some(mut app) = make_app(HashSet::new()) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let mock = Arc::new(MockMidiDevice::new());
    app.midi_device = Arc::clone(&mock) as Arc<dyn crate::midi::MidiDevice>;

    app.route_command(HostCommand::OpenMidiInput {
        port_id: "mock-input-1".to_owned(),
        pipe_id: "midi-in-pipe".to_owned(),
    });

    let evt = app
        .outbound_events
        .iter()
        .find(|e| matches!(e, PlexiEvent::MidiInputError { .. }))
        .expect("expected MidiInputError on outbound queue");
    match evt {
        PlexiEvent::MidiInputError { pipe_id, error } => {
            assert_eq!(pipe_id, "midi-in-pipe");
            assert!(
                error.contains("capability denied"),
                "denial must say `capability denied`: {error}"
            );
            assert!(
                error.contains("midi.in"),
                "denial must name the capability: {error}"
            );
        }
        other => panic!("expected MidiInputError, got {other:?}"),
    }

    // The mock must NOT have an active session — the denied path
    // short-circuits before open_input is called.
    assert!(
        mock.injected_sinks
            .lock()
            .expect("mock midi sinks poisoned")
            .is_empty(),
        "denied path must not open the MIDI input"
    );

    // SendMidi without `midi.out` is the same shape.
    app.route_command(HostCommand::SendMidi {
        port_id: "mock-output-1".to_owned(),
        bytes: vec![0x90, 0x3C, 0x64],
    });
    let evt = app
        .outbound_events
        .iter()
        .find(|e| matches!(e, PlexiEvent::MidiSendError { .. }))
        .expect("expected MidiSendError on outbound queue");
    match evt {
        PlexiEvent::MidiSendError { port_id, error } => {
            assert_eq!(port_id, "mock-output-1");
            assert!(
                error.contains("capability denied"),
                "denial must say `capability denied`: {error}"
            );
            assert!(
                error.contains("midi.out"),
                "denial must name the capability: {error}"
            );
        }
        other => panic!("expected MidiSendError, got {other:?}"),
    }
}

#[test]
fn granted_app_dispatches_open_input_to_device() {
    // App WITH `midi.in` granted: route_command must open the input on
    // the device, register a sink, and queue MidiInputOpened on
    // outbound_events. With `midi.out` granted, SendMidi must dispatch
    // the bytes to the mock's `sent` log.
    let mut caps = HashSet::new();
    caps.insert(Capability::MidiIn);
    caps.insert(Capability::MidiOut);
    let Some(mut app) = make_app(caps) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let mock = Arc::new(MockMidiDevice::new());
    app.midi_device = Arc::clone(&mock) as Arc<dyn crate::midi::MidiDevice>;

    app.route_command(HostCommand::OpenMidiInput {
        port_id: "mock-input-1".to_owned(),
        pipe_id: "midi-in-pipe".to_owned(),
    });

    // PipeOpened arrives BEFORE MidiInputOpened so the app can connect
    // the unix socket before the first byte arrives.
    let pipe_opened = app
        .outbound_events
        .iter()
        .position(|e| matches!(e, PlexiEvent::PipeOpened { .. }))
        .expect("expected PipeOpened");
    let midi_opened = app
        .outbound_events
        .iter()
        .position(|e| matches!(e, PlexiEvent::MidiInputOpened { .. }))
        .expect("expected MidiInputOpened");
    assert!(
        pipe_opened < midi_opened,
        "PipeOpened must precede MidiInputOpened so the app's socket connection races first"
    );

    // The device must have a registered sink for the port.
    assert!(
        mock.injected_sinks
            .lock()
            .expect("mock midi sinks poisoned")
            .contains_key("mock-input-1"),
        "open_input must have registered a sink"
    );

    // SendMidi path: dispatches one note-on to the mock output log.
    app.route_command(HostCommand::SendMidi {
        port_id: "mock-output-1".to_owned(),
        bytes: vec![0x90, 0x3C, 0x64],
    });
    let log = mock.sent.lock().expect("mock sent poisoned").clone();
    let entries = log
        .get("mock-output-1")
        .expect("mock-output-1 entries must exist after SendMidi");
    assert_eq!(entries.len(), 1, "exactly one send dispatched");
    assert_eq!(entries[0], vec![0x90u8, 0x3C, 0x64]);
}
