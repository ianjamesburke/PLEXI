//! IME composition state.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (editor.rs IME handling), MIT.
//! Diverges from upstream: preedit text is never inserted into the buffer —
//! it lives here and is painted as an overlay at the caret, so the buffer
//! only mutates through committed transactions.

/// In-progress IME composition. `None` when not composing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImeState {
    preedit: Option<String>,
}

impl ImeState {
    /// Replaces the current preedit text. An empty string keeps the
    /// composition session open with nothing to paint.
    pub fn set_preedit(&mut self, text: String) {
        self.preedit = Some(text);
    }

    /// Ends composition (after commit or cancel).
    pub fn clear(&mut self) {
        self.preedit = None;
    }

    /// The composition text to paint at the caret, if composing.
    #[must_use]
    pub fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }

    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.preedit.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preedit_lifecycle() {
        let mut ime = ImeState::default();
        assert!(!ime.is_composing());
        ime.set_preedit("か".into());
        assert_eq!(ime.preedit(), Some("か"));
        ime.set_preedit("かん".into());
        assert_eq!(ime.preedit(), Some("かん"));
        ime.clear();
        assert!(!ime.is_composing());
        assert_eq!(ime.preedit(), None);
    }
}
