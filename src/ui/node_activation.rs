//! Only controls painted and enabled in the current pass can be activated.
pub(crate) struct NodeActivations<A>(egui::IdMap<A>);

impl<A> Default for NodeActivations<A> {
    fn default() -> Self { Self(egui::IdMap::default()) }
}

impl<A: Clone> NodeActivations<A> {
    pub(crate) fn clear(&mut self) { self.0.clear(); }
    pub(crate) fn record(&mut self, response: &egui::Response, action: A) {
        if response.enabled() { self.0.insert(response.id, action); }
    }
    pub(crate) fn record_id(&mut self, id: egui::Id, action: A) { self.0.insert(id, action); }
    pub(crate) fn get(&self, id: egui::Id) -> Option<A> { self.0.get(&id).cloned() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeActivation { Activated, NotActivatable }
