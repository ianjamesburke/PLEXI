use crate::host::effect::HostEffect;

pub trait EventSink: Send {
    fn emit(&mut self, effect: &HostEffect);
}

/// No-op sink — used in production until the event bus is fully wired.
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&mut self, _effect: &HostEffect) {}
}

/// Accumulates all emitted effects into a vec. Used in tests.
pub struct VecEventSink {
    pub events: Vec<HostEffect>,
}

impl VecEventSink {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl Default for VecEventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for VecEventSink {
    fn emit(&mut self, effect: &HostEffect) {
        self.events.push(effect.clone());
    }
}

pub struct HostServices {
    pub event_sink: Box<dyn EventSink>,
}

impl HostServices {
    pub fn new() -> Self {
        Self { event_sink: Box::new(NoopEventSink) }
    }
}

impl Default for HostServices {
    fn default() -> Self {
        Self::new()
    }
}
