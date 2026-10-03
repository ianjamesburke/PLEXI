use super::*;

fn origin() -> Origin {
    Origin {
        window: 1,
        context: 1,
        pane: 2,
        workspace: std::path::PathBuf::from("/fixture"),
    }
}

fn utterance(id: u64) -> Utterance {
    Utterance {
        id,
        generation: 1,
        origin: origin(),
        text: "Open Notes".into(),
        started: Instant::now(),
        finalized: Instant::now(),
    }
}

#[test]
fn queue_remains_live_while_previous_execution_is_pending() {
    let mut session = Session::default();
    session.start();
    session.enqueue(utterance(1)).unwrap();
    assert_eq!(session.next().unwrap().id, 1);
    for id in 2..=9 {
        session.enqueue(utterance(id)).unwrap();
    }
    assert!(session.next().is_none());
    assert!(session.enqueue(utterance(10)).is_err());
    session.complete(1, 1, "Opened".into());
    assert_eq!(session.next().unwrap().id, 2);
}

#[test]
fn stop_cancels_queue_and_late_results_cannot_cross_restart() {
    let mut session = Session::default();
    session.start();
    session.enqueue(utterance(1)).unwrap();
    session.next().unwrap();
    session.stop();
    session.start();
    assert!(!session.complete(1, 1, "Late action".into()));
    assert!(session.enqueue(utterance(2)).is_err());
    assert!(session.next().is_none());
}

#[test]
fn finalized_identity_cannot_execute_twice() {
    let mut session = Session::default();
    session.start();
    session.enqueue(utterance(1)).unwrap();
    assert!(session.enqueue(utterance(1)).is_err());
    session.next().unwrap();
    session.complete(1, 1, "Opened".into());
    assert!(session.enqueue(utterance(1)).is_err());
}

#[test]
fn silence_and_partial_frames_never_finalize_a_command() {
    let mut segmenter = Segmenter::new(700);
    for _ in 0..100 {
        assert!(segmenter.push(&[0.0; FRAME], false).is_none());
    }
    for _ in 0..20 {
        assert!(segmenter.push(&[0.1; FRAME], true).is_none());
    }
    for _ in 0..43 {
        assert!(segmenter.push(&[0.0; FRAME], false).is_none());
    }
    assert!(segmenter.push(&[0.0; FRAME], false).is_some());
    assert!(segmenter.push(&[0.0; FRAME], false).is_none());
}

#[test]
fn overload_rejects_whole_utterance_and_recovers_after_silence() {
    let mut segmenter = Segmenter::new(700);
    for _ in 0..800 {
        assert!(segmenter.push(&[0.1; FRAME], true).is_none());
    }
    assert!(segmenter.rejected);
    assert!(segmenter.samples.len() <= MAX_SAMPLES);
    for _ in 0..44 {
        assert!(segmenter.push(&[0.0; FRAME], false).is_none());
    }
    assert!(!segmenter.rejected);
    for _ in 0..20 {
        segmenter.push(&[0.1; FRAME], true);
    }
    segmenter.invalidate();
    for _ in 0..44 {
        assert!(segmenter.push(&[0.0; FRAME], false).is_none());
    }
}

#[test]
fn continuous_finalization_does_not_wait_for_a_blocked_decision() {
    use std::sync::{mpsc, Arc, Mutex};
    let mailbox = Arc::new(Mutex::new(runtime::Mailbox::default()));
    let output = mailbox.clone();
    let (release, blocked) = mpsc::sync_channel::<()>(1);
    let interpreter = std::thread::spawn(move || blocked.recv().unwrap());
    let transcription = std::thread::spawn(move || {
        let mut segmenter = Segmenter::new(700);
        for id in 1..=10 {
            for _ in 0..20 {
                segmenter.push(&[0.1; FRAME], true);
            }
            for _ in 0..44 {
                if segmenter.push(&[0.0; FRAME], false).is_some() {
                    output.lock().unwrap().finalize(utterance(id));
                }
            }
        }
    });
    transcription.join().unwrap();
    let shared = mailbox.lock().unwrap();
    assert_eq!(shared.finals.len(), QUEUE_CAPACITY);
    assert_eq!(shared.rejected, 2);
    assert!(!interpreter.is_finished());
    release.send(()).unwrap();
    interpreter.join().unwrap();
}
