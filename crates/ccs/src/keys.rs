//! The keys of the config (`ccs_core::keys`), as crossterm reports them.

use ccs_core::keys::{self, Key};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Whether `pressed` is `key`. Shift is in the character's case: crossterm sends Alt+Shift+N as `n`
/// with Shift under the kitty protocol, and as `N` otherwise.
pub fn matches(key: Key, pressed: &KeyEvent) -> bool {
    let mut code = pressed.code;
    let mut modifiers = pressed.modifiers;
    if let KeyCode::Char(c) = code
        && modifiers.contains(KeyModifiers::SHIFT)
    {
        code = KeyCode::Char(c.to_ascii_uppercase());
        modifiers.remove(KeyModifiers::SHIFT);
    }
    let wanted = event(key);
    code == wanted.code && modifiers == wanted.modifiers
}

/// The key press itself, to pass it on to a program.
pub fn event(key: Key) -> KeyEvent {
    let code = match key.code {
        keys::KeyCode::Char(c) => KeyCode::Char(c),
        keys::KeyCode::Enter => KeyCode::Enter,
    };
    let mut modifiers = KeyModifiers::NONE;
    modifiers.set(KeyModifiers::CONTROL, key.ctrl);
    modifiers.set(KeyModifiers::ALT, key.alt);
    KeyEvent::new(code, modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn matches_keys_as_terminals_send_them() {
        let key = |text| Key::parse(text).unwrap();
        let ctrl_space = press(KeyCode::Char(' '), KeyModifiers::CONTROL);
        assert!(matches(key("<C-Space>"), &ctrl_space));
        assert!(!matches(
            key("<C-Space>"),
            &press(KeyCode::Char(' '), KeyModifiers::NONE)
        ));
        assert_eq!(event(key("<C-Space>")), ctrl_space);

        assert!(matches(
            key("<C-n>"),
            &press(KeyCode::Char('n'), KeyModifiers::CONTROL)
        ));
        assert!(matches(
            key("<CR>"),
            &press(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert!(matches(
            key("<A-1>"),
            &press(KeyCode::Char('1'), KeyModifiers::ALT)
        ));
        assert!(!matches(
            key("<A-1>"),
            &press(KeyCode::Char('1'), KeyModifiers::CONTROL)
        ));

        // Shift: as the character's case, or as a modifier.
        let shift = KeyModifiers::SHIFT;
        assert!(matches(key("N"), &press(KeyCode::Char('N'), shift)));
        assert!(matches(
            key("N"),
            &press(KeyCode::Char('N'), KeyModifiers::NONE)
        ));
        assert!(!matches(key("n"), &press(KeyCode::Char('N'), shift)));
        assert!(!matches(
            key("N"),
            &press(KeyCode::Char('n'), KeyModifiers::NONE)
        ));
        assert!(matches(
            key("<A-N>"),
            &press(KeyCode::Char('n'), KeyModifiers::ALT | shift)
        ));
        assert!(matches(
            key("<A-N>"),
            &press(KeyCode::Char('N'), KeyModifiers::ALT | shift)
        ));
        assert!(!matches(
            key("<A-n>"),
            &press(KeyCode::Char('n'), KeyModifiers::ALT | shift)
        ));
    }
}
