//! The focus stack: the one ordered record of which interactive surface owns
//! input (stint 1391). Its top decides modal key dispatch, which surface draws
//! and takes pointer hits, egui focus projection, and whether a pane or tab
//! action is admitted. Each entry names its surface and this opening of it,
//! so a delayed action can tell whether the opening it belongs to still
//! stands. It also records which surface owns the pointer press in progress
//! and which surface the last pass drew, so a pane admits a pointer action
//! only for a press no surface owned. No egui types.

/// One opening of a surface. A surface that closes and opens again is a new
/// opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OpeningId(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Opening<K> {
    pub kind: K,
    pub id: OpeningId,
    /// The request a request-backed surface (an approval) stands for.
    pub request: Option<String>,
}

/// Surfaces back to front; the last entry is the top.
#[derive(Debug)]
pub(crate) struct FocusStack<K> {
    entries: Vec<Opening<K>>,
    next: u64,
    /// The surface that owned the press of the pointer gesture in progress.
    press_owner: Option<K>,
    /// The top surface a presented pass drew, and that pass.
    drawn: Option<(K, u64)>,
}

impl<K> Default for FocusStack<K> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next: 0,
            press_owner: None,
            drawn: None,
        }
    }
}

impl<K: Copy + Eq> FocusStack<K> {
    /// The surface that owns input.
    pub(crate) fn top(&self) -> Option<K> {
        self.entries.last().map(|entry| entry.kind)
    }

    pub(crate) fn top_opening(&self) -> Option<&Opening<K>> {
        self.entries.last()
    }

    pub(crate) fn contains(&self, kind: &K) -> bool {
        self.entries.iter().any(|entry| entry.kind == *kind)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The open surfaces, back to front.
    #[cfg(test)]
    pub(crate) fn kinds(&self) -> impl Iterator<Item = K> + '_ {
        self.entries.iter().map(|entry| entry.kind)
    }

    /// The opening of `kind`, if it is open.
    pub(crate) fn opening(&self, kind: K) -> Option<&Opening<K>> {
        self.entries.iter().find(|entry| entry.kind == kind)
    }

    /// Open `kind` on top, covering what is below. Already on top: the same
    /// opening stands. Open further down: that opening ends and a new one
    /// covers the rest.
    pub(crate) fn open(&mut self, kind: K, request: Option<String>) -> OpeningId {
        if let Some(top) = self.entries.last_mut().filter(|top| top.kind == kind) {
            top.request = request;
            return top.id;
        }
        self.entries.retain(|entry| entry.kind != kind);
        self.next += 1;
        let id = OpeningId(self.next);
        self.entries.push(Opening { kind, id, request });
        id
    }

    /// Bring an open `kind` back to the top, keeping its opening. Returns
    /// whether it was open.
    pub(crate) fn promote(&mut self, kind: K) -> bool {
        let Some(at) = self.entries.iter().position(|entry| entry.kind == kind) else {
            return false;
        };
        let entry = self.entries.remove(at);
        self.entries.push(entry);
        true
    }

    /// `kind` owns the pointer press now under way, until every button is up.
    pub(crate) fn own_press(&mut self, kind: K) {
        self.press_owner = Some(kind);
    }

    /// Every button is up: no surface owns a press.
    pub(crate) fn release_presses(&mut self) {
        self.press_owner = None;
    }

    pub(crate) fn press_owner(&self) -> Option<K> {
        self.press_owner
    }

    /// The pass `pass` drew `kind` and will be presented.
    pub(crate) fn note_drawn(&mut self, kind: K, pass: u64) {
        self.drawn = Some((kind, pass));
    }

    /// Whether `kind` was drawn in `pass`: egui hit tests a pass's pointer
    /// input against the layout of the pass before it.
    pub(crate) fn drawn_in(&self, kind: K, pass: u64) -> bool {
        self.drawn == Some((kind, pass))
    }

    /// End the opening of `kind`, wherever it is.
    pub(crate) fn dismiss(&mut self, kind: K) -> Option<Opening<K>> {
        let at = self.entries.iter().position(|entry| entry.kind == kind)?;
        Some(self.entries.remove(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum K {
        Queue,
        Palette,
        Banner,
    }

    #[test]
    fn openings_cover_promote_and_dismiss() {
        let mut stack = FocusStack::default();
        let queue = stack.open(K::Queue, None);
        let palette = stack.open(K::Palette, None);
        assert_eq!(stack.top(), Some(K::Palette));
        assert_eq!(stack.open(K::Palette, None), palette, "already on top");
        assert!(stack.promote(K::Queue));
        assert_eq!(stack.top(), Some(K::Queue));
        assert_eq!(stack.opening(K::Queue).map(|o| o.id), Some(queue));
        let reopened = stack.open(K::Palette, None);
        assert_ne!(
            reopened, palette,
            "an opening beneath ends when reopened on top"
        );
        assert_eq!(stack.kinds().collect::<Vec<_>>(), [K::Queue, K::Palette]);
        stack.open(K::Banner, Some("req-1".into()));
        assert_eq!(
            stack.top_opening().and_then(|o| o.request.as_deref()),
            Some("req-1")
        );
        assert_eq!(stack.dismiss(K::Palette).map(|o| o.id), Some(reopened));
        assert_eq!(stack.kinds().collect::<Vec<_>>(), [K::Queue, K::Banner]);
        assert!(stack.dismiss(K::Palette).is_none());
    }
}
