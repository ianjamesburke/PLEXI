#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    ClosePane,
    Quit,
    NavigateLeft,
    NavigateRight,
    NavigateUp,
    NavigateDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStroke {
    pub command: bool,
    pub shift: bool,
    pub key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    W,
    Q,
    H,
    J,
    K,
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPolicy {
    pub app_capture: bool,
}

impl KeyPolicy {
    pub fn allow_host_shortcut(self, action: KeyAction) -> bool {
        if !self.app_capture {
            return true;
        }
        matches!(action, KeyAction::ClosePane | KeyAction::Quit)
    }
}

pub fn map_key(stroke: KeyStroke) -> Option<KeyAction> {
    if stroke.command && !stroke.shift {
        return match stroke.key {
            Key::W => Some(KeyAction::ClosePane),
            Key::Q => Some(KeyAction::Quit),
            Key::H => Some(KeyAction::NavigateLeft),
            Key::J => Some(KeyAction::NavigateDown),
            Key::K => Some(KeyAction::NavigateUp),
            Key::L => Some(KeyAction::NavigateRight),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::input::keymap::{map_key, Key, KeyAction, KeyPolicy, KeyStroke};

    #[test]
    fn cmd_w_and_cmd_q_stay_global_under_capture() {
        let policy = KeyPolicy { app_capture: true };
        assert!(policy.allow_host_shortcut(KeyAction::ClosePane));
        assert!(policy.allow_host_shortcut(KeyAction::Quit));
        assert!(!policy.allow_host_shortcut(KeyAction::NavigateLeft));
    }

    #[test]
    fn keymap_maps_core_cmd_shortcuts() {
        let close = map_key(KeyStroke {
            command: true,
            shift: false,
            key: Key::W,
        });
        assert_eq!(close, Some(KeyAction::ClosePane));

        let quit = map_key(KeyStroke {
            command: true,
            shift: false,
            key: Key::Q,
        });
        assert_eq!(quit, Some(KeyAction::Quit));
    }
}
