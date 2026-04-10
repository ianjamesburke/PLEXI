use std::path::PathBuf;
use std::sync::mpsc;

pub(crate) enum AudioMsg {
    Play(PathBuf),
    Pause,
    Resume,
    Stop,
}

/// Audio thread — owns rodio OutputStream and Sink (not Send).
pub(crate) fn audio_thread(rx: mpsc::Receiver<AudioMsg>) {
    let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
        log::error!("FileBrowser audio: failed to open output stream");
        return;
    };
    let mut sink: Option<rodio::Sink> = None;

    loop {
        let msg = match rx.recv() {
            Ok(m) => m,
            Err(_) => break,
        };
        match msg {
            AudioMsg::Play(path) => {
                if let Some(s) = sink.take() {
                    s.stop();
                }
                let Ok(file) = std::fs::File::open(&path) else { continue };
                let Ok(source) = rodio::Decoder::new(std::io::BufReader::new(file)) else { continue };
                let Ok(new_sink) = rodio::Sink::try_new(&handle) else { continue };
                new_sink.append(source);
                sink = Some(new_sink);
            }
            AudioMsg::Pause => {
                if let Some(s) = &sink { s.pause(); }
            }
            AudioMsg::Resume => {
                if let Some(s) = &sink { s.play(); }
            }
            AudioMsg::Stop => {
                if let Some(s) = sink.take() { s.stop(); }
            }
        }
    }
}
