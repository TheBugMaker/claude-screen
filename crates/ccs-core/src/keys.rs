//! Keys in the config, written as vim writes them: `<leader>f` after the leader, `<C-f>` and
//! `<A-1>` on their own, `<C-Space>` for the leader itself.

use std::fmt;

use serde::Deserialize;

/// How a key is written, for errors.
const NOTATION: &str =
    "write keys as vim does: f, N, <C-f>, <A-1>, <C-Space>, <leader>f, <leader><CR>";

/// A key pressed: a character or Enter, with Ctrl and Alt. Shift is in the character's case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// Space is `' '`.
    Char(char),
    Enter,
}

impl Key {
    pub const ENTER: Self = Self {
        code: KeyCode::Enter,
        ctrl: false,
        alt: false,
    };

    #[must_use]
    pub const fn char(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            ctrl: false,
            alt: false,
        }
    }

    /// Parses one key: a character, or `<…>` with `C-` and `A-` (or `M-`) before a character or
    /// one of `Space`, `CR` (`Enter`), `lt`. As in vim, `<C-N>` is `<C-n>`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let unknown = || format!("unknown key {text:?}: {NOTATION}");
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Ok(Self::char(c));
        }
        let mut rest = text
            .strip_prefix('<')
            .and_then(|t| t.strip_suffix('>'))
            .ok_or_else(unknown)?;
        let mut key = Self::char(' ');
        while rest.len() > 2 && rest.as_bytes()[1] == b'-' {
            match rest.as_bytes()[0].to_ascii_lowercase() {
                b'c' => key.ctrl = true,
                b'a' | b'm' => key.alt = true,
                _ => return Err(unknown()),
            }
            rest = &rest[2..];
        }
        let mut chars = rest.chars();
        key.code = match (chars.next(), chars.next()) {
            (Some(c), None) => KeyCode::Char(c),
            _ => match rest.to_ascii_lowercase().as_str() {
                "space" => KeyCode::Char(' '),
                "cr" | "enter" => KeyCode::Enter,
                "lt" => KeyCode::Char('<'),
                _ => return Err(unknown()),
            },
        };
        if let (true, KeyCode::Char(c)) = (key.ctrl, key.code) {
            key.code = KeyCode::Char(c.to_ascii_lowercase());
        }
        Ok(key)
    }

    /// How the key is shown on screen: `f`, `Enter`, `C-Space`, `A-1`.
    #[must_use]
    pub fn label(self) -> String {
        let name = match self.code {
            KeyCode::Char(' ') => "Space".to_owned(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".to_owned(),
        };
        let ctrl = if self.ctrl { "C-" } else { "" };
        let alt = if self.alt { "A-" } else { "" };
        format!("{ctrl}{alt}{name}")
    }
}

impl TryFrom<String> for Key {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        Self::parse(&text)
    }
}

/// As the config writes it: `f`, `<CR>`, `<C-Space>`.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let name = match self.code {
            KeyCode::Char(' ') => "Space".to_owned(),
            KeyCode::Char('<') => "lt".to_owned(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "CR".to_owned(),
        };
        match self {
            Self {
                code: KeyCode::Char(c),
                ctrl: false,
                alt: false,
            } if *c != ' ' && *c != '<' => write!(f, "{c}"),
            _ => {
                let ctrl = if self.ctrl { "C-" } else { "" };
                let alt = if self.alt { "A-" } else { "" };
                write!(f, "<{ctrl}{alt}{name}>")
            }
        }
    }
}

/// A key that does something: after the leader (`<leader>f`), or on its own (`<C-f>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Binding {
    pub leader: bool,
    pub key: Key,
}

impl Binding {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.get(..8) {
            Some(prefix) if prefix.eq_ignore_ascii_case("<leader>") => Ok(Self {
                leader: true,
                key: Key::parse(&text[8..])?,
            }),
            _ => Ok(Self {
                leader: false,
                key: Key::parse(text)?,
            }),
        }
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.leader {
            write!(f, "<leader>")?;
        }
        write!(f, "{}", self.key)
    }
}

/// The keys that do one thing: `"<leader>f"`, or a list like `["<leader>f", "<C-f>"]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "OneOrMany")]
pub struct Bindings(pub Vec<Binding>);

impl Bindings {
    #[must_use]
    pub fn after_leader(key: Key) -> Self {
        Self(vec![Binding { leader: true, key }])
    }
}

/// Strings, parsed after: a key that doesn't parse then says why, rather than that no form fits.
#[derive(Deserialize)]
#[serde(untagged, expecting = "a key, like \"<leader>f\", or a list of them")]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl TryFrom<OneOrMany> for Bindings {
    type Error = String;

    fn try_from(value: OneOrMany) -> Result<Self, String> {
        let texts = match value {
            OneOrMany::One(text) => vec![text],
            OneOrMany::Many(texts) => texts,
        };
        texts
            .iter()
            .map(|t| Binding::parse(t))
            .collect::<Result<_, _>>()
            .map(Self)
    }
}

/// What a key can do, besides going to a tab: the names in the config's `[keys]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "String")]
pub enum Action {
    Session1,
    Session2,
    Session3,
    Session4,
    Session5,
    Session6,
    Session7,
    Session8,
    Session9,
    PreviousSession,
    NextSession,
    NewSession,
    NewSessionNoWorktree,
    Find,
    Merge,
    Chat,
    ChatSplit,
    OtherPane,
    PreviousTab,
    NextTab,
    RerunTab,
    SubmitAndNewSession,
    SubmitAndStay,
    Quit,
}

impl Action {
    /// Every action, in the order the leader panel lists them; the sessions first.
    pub const ALL: [Self; 24] = [
        Self::Session1,
        Self::Session2,
        Self::Session3,
        Self::Session4,
        Self::Session5,
        Self::Session6,
        Self::Session7,
        Self::Session8,
        Self::Session9,
        Self::PreviousSession,
        Self::NextSession,
        Self::NewSession,
        Self::NewSessionNoWorktree,
        Self::Find,
        Self::Merge,
        Self::Chat,
        Self::ChatSplit,
        Self::OtherPane,
        Self::PreviousTab,
        Self::NextTab,
        Self::RerunTab,
        Self::SubmitAndNewSession,
        Self::SubmitAndStay,
        Self::Quit,
    ];

    /// Its name in the config, and the key after the leader that does it by default.
    fn spec(self) -> (&'static str, Key) {
        match self {
            Self::Session1 => ("session_1", Key::char('1')),
            Self::Session2 => ("session_2", Key::char('2')),
            Self::Session3 => ("session_3", Key::char('3')),
            Self::Session4 => ("session_4", Key::char('4')),
            Self::Session5 => ("session_5", Key::char('5')),
            Self::Session6 => ("session_6", Key::char('6')),
            Self::Session7 => ("session_7", Key::char('7')),
            Self::Session8 => ("session_8", Key::char('8')),
            Self::Session9 => ("session_9", Key::char('9')),
            Self::PreviousSession => ("previous_session", Key::char(',')),
            Self::NextSession => ("next_session", Key::char('.')),
            Self::NewSession => ("new_session", Key::char('n')),
            Self::NewSessionNoWorktree => ("new_session_no_worktree", Key::char('N')),
            Self::Find => ("find", Key::char('f')),
            Self::Merge => ("merge", Key::char('m')),
            Self::Chat => ("chat", Key::char('c')),
            Self::ChatSplit => ("chat_split", Key::char('C')),
            Self::OtherPane => ("other_pane", Key::char('o')),
            Self::PreviousTab => ("previous_tab", Key::char('[')),
            Self::NextTab => ("next_tab", Key::char(']')),
            Self::RerunTab => ("rerun_tab", Key::ENTER),
            Self::SubmitAndNewSession => ("submit_and_new_session", Key::ENTER),
            Self::SubmitAndStay => ("submit_and_stay", Key::char('w')),
            Self::Quit => ("quit", Key::char('q')),
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        self.spec().0
    }

    #[must_use]
    pub fn default_bindings(self) -> Bindings {
        Bindings::after_leader(self.spec().1)
    }

    /// The session it goes to, from 0: `session_1` … `session_9`.
    #[must_use]
    pub fn session(self) -> Option<usize> {
        Self::ALL[..9].iter().position(|&a| a == self)
    }

    /// Whether it does anything with a terminal tab on screen (`peer`), or with Chat.
    #[must_use]
    pub fn works_in(self, peer: bool) -> bool {
        match self {
            Self::SubmitAndNewSession | Self::SubmitAndStay => !peer,
            Self::RerunTab => peer,
            _ => true,
        }
    }
}

impl TryFrom<String> for Action {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|a| a.name() == name)
            .ok_or_else(|| format!("unknown action {name:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, ctrl: bool, alt: bool) -> Key {
        Key { code, ctrl, alt }
    }

    #[test]
    fn parses_keys_as_vim_writes_them() {
        assert_eq!(Key::parse("f"), Ok(Key::char('f')));
        assert_eq!(Key::parse("N"), Ok(Key::char('N')));
        assert_eq!(Key::parse("<"), Ok(Key::char('<')));
        assert_eq!(Key::parse("<lt>"), Ok(Key::char('<')));
        assert_eq!(Key::parse("<CR>"), Ok(Key::ENTER));
        assert_eq!(Key::parse("<enter>"), Ok(Key::ENTER));
        let ctrl_space = key(KeyCode::Char(' '), true, false);
        assert_eq!(Key::parse("<C-Space>"), Ok(ctrl_space));
        assert_eq!(Key::parse("<c-space>"), Ok(ctrl_space));
        // Ctrl ignores case; Alt doesn't.
        assert_eq!(
            Key::parse("<C-N>"),
            Ok(key(KeyCode::Char('n'), true, false))
        );
        assert_eq!(
            Key::parse("<A-N>"),
            Ok(key(KeyCode::Char('N'), false, true))
        );
        assert_eq!(
            Key::parse("<M-1>"),
            Ok(key(KeyCode::Char('1'), false, true))
        );
        assert_eq!(
            Key::parse("<C-A-x>"),
            Ok(key(KeyCode::Char('x'), true, true))
        );
        assert_eq!(
            Key::parse("<C-->"),
            Ok(key(KeyCode::Char('-'), true, false))
        );
        for text in [
            "",
            "ctrl-space",
            "<C-Tab>",
            "<S-n>",
            "<C->",
            "<C-ab>",
            "<Space",
        ] {
            assert!(Key::parse(text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn writes_keys_back_as_they_parse() {
        for text in [
            "f",
            "N",
            "<lt>",
            "<CR>",
            "<C-Space>",
            "<A-1>",
            "<C-A-x>",
            "<Space>",
        ] {
            assert_eq!(Key::parse(text).unwrap().to_string(), text);
        }
        assert_eq!(Key::parse("<C-Space>").unwrap().label(), "C-Space");
        assert_eq!(Key::ENTER.label(), "Enter");
        let binding = Binding::parse("<Leader><CR>").unwrap();
        assert_eq!(
            binding,
            Binding {
                leader: true,
                key: Key::ENTER
            }
        );
        assert_eq!(binding.to_string(), "<leader><CR>");
        assert!(!Binding::parse("<A-f>").unwrap().leader);
    }

    #[test]
    fn actions_have_names_and_default_keys() {
        assert_eq!(
            Action::try_from("new_session_no_worktree".to_owned()),
            Ok(Action::NewSessionNoWorktree)
        );
        assert!(Action::try_from("open".to_owned()).is_err());
        assert_eq!(Action::Session9.session(), Some(8));
        assert_eq!(Action::Quit.session(), None);
        assert_eq!(
            Action::Session3.default_bindings(),
            Bindings::after_leader(Key::char('3'))
        );
    }
}
