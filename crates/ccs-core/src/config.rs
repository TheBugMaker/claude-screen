//! `~/.config/ccs/config.toml`: the leader key, the theme, the slide between sessions, worktrees
//! and the `[[tab]]` definitions.

use std::path::Path;

use serde::Deserialize;

use crate::Error;

/// Keys after the leader that `App::on_leader_key` takes for itself, besides the digits: they
/// can't be a tab's key.
const LEADER_KEYS: &[char] = &['[', ']', ',', '.', 'c', 'f', 'q'];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Key that starts a ccs shortcut, e.g. `ctrl-space`. Parsed by the app.
    pub leader: String,
    /// Name of the theme: a file in `~/.config/ccs/themes`, or a built-in one. Read by the app.
    pub theme: Option<String>,
    /// How long switching sessions slides the new one in, in milliseconds; 0 switches at once.
    pub slide_ms: u64,
    /// Each new session works in its own git worktree, which Claude makes (`claude --worktree`),
    /// and its tabs run there. Sessions outside git, or before the first commit, get none.
    pub worktree: bool,
    /// Peer tabs every session gets next to its Chat tab.
    #[serde(rename = "tab")]
    pub tabs: Vec<TabDef>,
}

/// One `[[tab]]`: a command run in a pseudo-terminal.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabDef {
    pub name: String,
    /// Goes to the tab after the leader.
    pub key: Option<char>,
    #[serde(default)]
    pub mode: Mode,
    /// Run with `sh -c` in the session's directory, with the `CS_*` variables set.
    pub command: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Run every time the tab is opened.
    Eager,
    /// Run the first time the tab is opened.
    #[default]
    Lazy,
    /// Run when the session starts, before the tab is opened.
    Regular,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            leader: "ctrl-space".into(),
            theme: None,
            slide_ms: 150,
            worktree: true,
            tabs: vec![
                TabDef {
                    name: "review".into(),
                    key: Some('r'),
                    mode: Mode::Eager,
                    command: "git diff".into(),
                },
                TabDef {
                    name: "shell".into(),
                    key: Some('s'),
                    mode: Mode::Regular,
                    command: "$SHELL".into(),
                },
            ],
        }
    }
}

impl Config {
    /// Reads the config at `path`; a missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(Error::Read(err)),
        }
    }

    pub fn parse(text: &str) -> Result<Self, Error> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), Error> {
        for (i, tab) in self.tabs.iter().enumerate() {
            let Some(key) = tab.key else { continue };
            if key.is_ascii_digit() || LEADER_KEYS.contains(&key) {
                return Err(Error::Invalid(format!(
                    "tab {:?}: key {key:?} is taken by a ccs shortcut",
                    tab.name
                )));
            }
            if self.tabs[..i].iter().any(|other| other.key == Some(key)) {
                return Err(Error::Invalid(format!(
                    "tab {:?}: key {key:?} is used by another tab",
                    tab.name
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let config = Config::parse(include_str!("../../../config/example.toml")).unwrap();
        let names: Vec<&str> = config.tabs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["review", "shell", "git"]);
        assert_eq!(config.slide_ms, 150);
        assert!(config.worktree);
    }

    #[test]
    fn missing_file_and_missing_tabs_give_the_defaults() {
        let config = Config::load(Path::new("/nonexistent/ccs/config.toml")).unwrap();
        assert_eq!(config, Config::default());
        let config = Config::parse("leader = \"ctrl-a\"").unwrap();
        assert_eq!(config.leader, "ctrl-a");
        assert_eq!(config.slide_ms, 150);
        assert_eq!(config.tabs, Config::default().tabs);
        // 0 turns the slide off.
        assert_eq!(Config::parse("slide_ms = 0").unwrap().slide_ms, 0);
        assert!(config.worktree);
        assert!(!Config::parse("worktree = false").unwrap().worktree);
    }

    #[test]
    fn tabs_replace_the_defaults() {
        let config = Config::parse("[[tab]]\nname = \"top\"\ncommand = \"htop\"").unwrap();
        assert_eq!(config.leader, "ctrl-space");
        assert_eq!(
            config.tabs,
            vec![TabDef {
                name: "top".into(),
                key: None,
                mode: Mode::Lazy,
                command: "htop".into(),
            }]
        );
    }

    #[test]
    fn parses_every_mode() {
        for (name, mode) in [
            ("eager", Mode::Eager),
            ("lazy", Mode::Lazy),
            ("regular", Mode::Regular),
        ] {
            let text = format!("[[tab]]\nname = \"a\"\ncommand = \"x\"\nmode = \"{name}\"");
            assert_eq!(Config::parse(&text).unwrap().tabs[0].mode, mode);
        }
    }

    #[test]
    fn rejects_bad_configs() {
        for text in [
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkind = \"claude\"",
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nmode = \"on_change\"",
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"3\"",
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"f\"",
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"c\"",
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"d\"\n\
             [[tab]]\nname = \"b\"\ncommand = \"y\"\nkey = \"d\"",
        ] {
            assert!(Config::parse(text).is_err(), "accepted: {text}");
        }
    }
}
