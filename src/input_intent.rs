use egui::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputIntent {
    Key,
    Text,
    Paste,
    Ime,
    Copy,
    Cut,
}

pub(crate) fn classify(event: &Event) -> Option<InputIntent> {
    match event {
        Event::Key { .. } => Some(InputIntent::Key),
        Event::Text(_) => Some(InputIntent::Text),
        Event::Paste(_) => Some(InputIntent::Paste),
        Event::Ime(_) => Some(InputIntent::Ime),
        Event::Copy => Some(InputIntent::Copy),
        Event::Cut => Some(InputIntent::Cut),
        _ => None,
    }
}
