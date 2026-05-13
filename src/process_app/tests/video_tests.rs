//! Routing tests for the v3.4 video substrate (#345).
//!
//! Two paths under test:
//!   1. App without `video.playback` — synchronous denial response on
//!      `outbound_events`. No device dispatch.
//!   2. App with the capability — `MockVideoDecoder` opens the source,
//!      the routing layer queues `VideoOpenAck` and pumps frames.
use super::super::*;
use crate::app_protocol::{HostCommand, PlexiEvent};
use crate::video::{MockVideoDecoder, MockVideoDecoderConfig};
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
        "test_video",
        "Test Video",
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
    // App without `video.playback`: route_command must immediately queue
    // a VideoOpenError with "capability denied" and never touch the
    // decoder.
    let Some(mut app) = make_app(HashSet::new()) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let mock = Arc::new(MockVideoDecoder::new(MockVideoDecoderConfig::default()));
    app.video_device = Arc::clone(&mock) as Arc<dyn crate::video::VideoDecoder>;

    app.route_command(HostCommand::OpenVideo {
        request_id: "req-denied".to_owned(),
        source: "mock://gradient".to_owned(),
        pipe_id: "video-stream".to_owned(),
    });

    let evt = app
        .outbound_events
        .iter()
        .find(|e| matches!(e, PlexiEvent::VideoOpenError { .. }))
        .expect("expected VideoOpenError on outbound queue");
    match evt {
        PlexiEvent::VideoOpenError { request_id, error } => {
            assert_eq!(request_id, "req-denied");
            assert!(
                error.contains("capability denied"),
                "denial must say `capability denied`: {error}"
            );
            assert!(
                error.contains("video.playback"),
                "denial must name the capability: {error}"
            );
        }
        other => panic!("expected VideoOpenError, got {other:?}"),
    }

    // The denial path must not produce a VideoOpenAck.
    assert!(
        !app
            .outbound_events
            .iter()
            .any(|e| matches!(e, PlexiEvent::VideoOpenAck { .. })),
        "denied path must not produce a VideoOpenAck"
    );
    assert!(
        app.video_handles.is_empty(),
        "denied path must not register a handle"
    );
}

#[test]
fn granted_app_dispatches_open_to_decoder() {
    // App WITH `video.playback`: route_command must open the decoder,
    // queue PipeOpened then VideoOpenAck, and register a handle. Then
    // SetVideoState dispatches into the handle without panicking, and
    // CloseVideo tears it down cleanly.
    let mut caps = HashSet::new();
    caps.insert(Capability::VideoPlayback);
    let Some(mut app) = make_app(caps) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let mock = Arc::new(MockVideoDecoder::new(MockVideoDecoderConfig {
        width: 16,
        height: 8,
        fps: 30.0,
        duration_ms: 5_000,
    }));
    app.video_device = Arc::clone(&mock) as Arc<dyn crate::video::VideoDecoder>;

    app.route_command(HostCommand::OpenVideo {
        request_id: "req-1".to_owned(),
        source: "mock://gradient".to_owned(),
        pipe_id: "video-stream".to_owned(),
    });

    // PipeOpened arrives BEFORE VideoOpenAck so the app can connect the
    // unix socket before the first frame.
    let pipe_opened = app
        .outbound_events
        .iter()
        .position(|e| matches!(e, PlexiEvent::PipeOpened { .. }))
        .expect("expected PipeOpened");
    let video_ack = app
        .outbound_events
        .iter()
        .position(|e| matches!(e, PlexiEvent::VideoOpenAck { .. }))
        .expect("expected VideoOpenAck");
    assert!(
        pipe_opened < video_ack,
        "PipeOpened must precede VideoOpenAck so the app's socket connection races first"
    );

    // Pull the ack out and confirm the dimensions match the mock config.
    let ack_handle_id = match &app.outbound_events[video_ack] {
        PlexiEvent::VideoOpenAck {
            handle_id,
            width,
            height,
            fps,
            duration_ms,
            request_id,
        } => {
            assert_eq!(request_id, "req-1");
            assert_eq!(*width, 16);
            assert_eq!(*height, 8);
            assert!((*fps - 30.0).abs() < 0.01);
            assert_eq!(*duration_ms, 5_000);
            *handle_id
        }
        _ => unreachable!(),
    };
    assert!(
        app.video_handles.contains_key(&ack_handle_id),
        "open must register a handle"
    );

    // SetVideoState — pause then play, neither should panic and no error
    // event should arrive.
    app.route_command(HostCommand::SetVideoState {
        handle_id: ack_handle_id,
        state: crate::video::VideoState::Pause,
    });
    app.route_command(HostCommand::SetVideoState {
        handle_id: ack_handle_id,
        state: crate::video::VideoState::Play,
    });
    app.route_command(HostCommand::SetVideoState {
        handle_id: ack_handle_id,
        state: crate::video::VideoState::Seek { position_ms: 1_000 },
    });
    // No additional VideoOpenError must have been queued.
    let errors = app
        .outbound_events
        .iter()
        .filter(|e| matches!(e, PlexiEvent::VideoOpenError { .. }))
        .count();
    assert_eq!(errors, 0, "set_state must not produce VideoOpenError");

    // CloseVideo tears down the handle and unregisters the pipe id map.
    app.route_command(HostCommand::CloseVideo {
        handle_id: ack_handle_id,
    });
    assert!(
        !app.video_handles.contains_key(&ack_handle_id),
        "close must drop the handle"
    );
    assert!(
        !app.video_pipe_ids.contains_key(&ack_handle_id),
        "close must unregister the pipe id mapping"
    );
}
