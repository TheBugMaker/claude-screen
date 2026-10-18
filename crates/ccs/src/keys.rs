//! Keys named in the config, like the leader `ctrl-space`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyMatch {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyMatch {
    /// Parses `ctrl-space`, `ctrl-a`, `alt-x`: modifiers (`ctrl`, `alt`, `shift`) joined to a
    /// key (`space` or one character) by `-`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (modifier_names, key) = text.rsplit_once('-').unwrap_or(("", text));
        let mut modifiers = KeyModifiers::NONE;
        for name in modifier_names.split('-').filter(|n| !n.is_empty()) {
            modifiers |= match name {
                "ctrl" => KeyModifiers::CONTROL,
                "alt" => KeyModifiers::ALT,
                "shift" => KeyModifiers::SHIFT,
                other => return Err(format!("unknown modifier {other:?} in {text:?}")),
            };
        }
        let mut chars = key.chars();
        let code = match (key, chars.next(), chars.next()) {
            ("space", _, _) => KeyCode::Char(' '),
            (_, Some(c), None) => KeyCode::Char(c.to_ascii_lowercase()),
            _ => return Err(format!("unknown key {key:?} in {text:?}")),
        };
        Ok(Self { code, modifiers })
    }

    pub fn matches(&self, key: &KeyEvent) -> bool {
        let code = match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            code => code,
        };
        code == self.code && key.modifiers == self.modifiers
    }

    /// The key press itself, to pass it on to a program.
    pub fn event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_matches_keys() {
        let leader = KeyMatch::parse("ctrl-space").unwrap();
        assert!(leader.matches(&KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL)));
        assert!(!leader.matches(&KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)));

        let leader = KeyMatch::parse("ctrl-a").unwrap();
        assert!(leader.matches(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
        assert_eq!(leader.event().code, KeyCode::Char('a'));

        assert!(KeyMatch::parse("alt-x").is_ok());
        assert!(KeyMatch::parse("hyper-x").is_err());
        assert!(KeyMatch::parse("ctrl-escape").is_err());
        assert!(KeyMatch::parse("").is_err());
    }
}
