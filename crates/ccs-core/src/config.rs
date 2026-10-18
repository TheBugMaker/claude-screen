//! `~/.config/ccs/config.toml`: the leader key, the theme, the slide between sessions, worktrees,
//! the `[[tab]]` definitions and the `[[merge]]` actions.

use std::path::Path;

use serde::Deserialize;

use crate::Error;

/// Keys after the leader that `App::on_leader_key` takes for itself, besides the digits: they
/// can't be a tab's key.
const LEADER_KEYS: &[char] = &['[', ']', ',', '.', 'c', 'f', 'm', 'n', 'N', 'q', 'w'];

/// What the default `[[merge]]` commits with.
pub const MERGE_COMMIT: &str = r#"git add -A && git commit -m "$CS_MESSAGE""#;

/// The default `[[merge]]`: merges the session's branch into the branch checked out in the main
/// checkout, there. A session that works on that branch itself only had to commit.
pub const MERGE_COMMAND: &str = r#"set -e
[ -n "$CS_REPO_BRANCH" ] || { echo "$CS_REPO is on no branch"; exit 1; }
[ "$CS_BRANCH" != "$CS_REPO_BRANCH" ] || { echo "committed on $CS_BRANCH: nothing to merge"; exit 0; }
cd "$CS_REPO"
git diff --quiet HEAD || { echo "$CS_REPO has uncommitted changes, or an unfinished merge"; exit 1; }
git merge --no-edit "$CS_BRANCH""#;

/// What the default `[[merge]]` asks the Claude that fixes its conflicts to do.
pub const MERGE_ON_CONFLICT: &str = "Merging branch `$CS_BRANCH` into `$CS_REPO_BRANCH` stopped \
on conflicts in this checkout. Resolve each conflict, keeping what both sides meant, and run the \
project's checks if it has them. Then finish the merge with `git commit --no-edit`. If a conflict \
can't be resolved safely, stop and explain it, and leave the merge unfinished.";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Key that starts a ccs shortcut, e.g. `ctrl-space`. Parsed by the app.
    pub leader: String,
    /// Name of the theme: a file in `~/.config/ccs/themes`, or a built-in one. Read by the app.
    pub theme: Option<String>,
    /// How long switching sessions slides the new one in over 100 columns, in milliseconds: a
    /// wider window takes longer, at the same speed. 0 switches at once.
    pub slide_ms: u64,
    /// Each new session works in its own git worktree, which Claude makes (`claude --worktree`),
    /// and its tabs run there. Sessions outside git, or before the first commit, get none.
    pub worktree: bool,
    /// Peer tabs every session gets next to its Chat tab.
    #[serde(rename = "tab")]
    pub tabs: Vec<TabDef>,
    /// The ways leader m can get a session's work into its target, picked in its panel.
    #[serde(rename = "merge")]
    pub merges: Vec<MergeDef>,
}

/// One `[[merge]]`: a way to get a session's work into its target, such as merging it there,
/// pushing it, or opening a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeDef {
    pub name: String,
    /// Commits the changes in the session's directory, when it has any, before `command`. The
    /// message you typed is in `CS_MESSAGE`. If it fails, nothing more runs.
    #[serde(default = "merge_commit")]
    pub commit: String,
    /// Run with `sh -c` in the session's directory, with the `CS_*` variables set. Exiting 0 means
    /// the work is in, and the session closes.
    pub command: String,
    /// What to ask Claude when `command` stops on conflicts: `$CS_*` names are filled in. The
    /// session's own Claude gets it for conflicts in its directory, a new session in the main
    /// checkout for conflicts there. Without it, a conflict only gets a notice.
    pub on_conflict: Option<String>,
}

fn merge_commit() -> String {
    MERGE_COMMIT.to_owned()
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
            merges: vec![MergeDef {
                name: "merge".into(),
                commit: MERGE_COMMIT.into(),
                command: MERGE_COMMAND.into(),
                on_conflict: Some(MERGE_ON_CONFLICT.into()),
            }],
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
        // It would do nothing and succeed, which closes the session.
        if let Some(merge) = self.merges.iter().find(|m| m.command.trim().is_empty()) {
            return Err(Error::Invalid(format!(
                "merge {:?}: its command is empty",
                merge.name
            )));
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
        // It shows the default action as it is.
        assert_eq!(config.merges, Config::default().merges);
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
    fn merge_actions_replace_the_default() {
        assert_eq!(Config::parse("").unwrap().merges.len(), 1);
        let config = Config::parse(
            "[[merge]]\nname = \"pr\"\ncommand = \"gh pr create --fill\"\n\
             [[merge]]\nname = \"push\"\ncommit = \"git commit -am \\\"$CS_MESSAGE\\\"\"\n\
             command = \"git push\"\non_conflict = \"fix it\"",
        )
        .unwrap();
        assert_eq!(
            config.merges,
            vec![
                MergeDef {
                    name: "pr".into(),
                    commit: MERGE_COMMIT.into(),
                    command: "gh pr create --fill".into(),
                    on_conflict: None,
                },
                MergeDef {
                    name: "push".into(),
                    commit: "git commit -am \"$CS_MESSAGE\"".into(),
                    command: "git push".into(),
                    on_conflict: Some("fix it".into()),
                },
            ]
        );
        // None at all: leader m says so.
        assert_eq!(Config::parse("merge = []").unwrap().merges, []);
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
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"n\"",
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"m\"",
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"d\"\n\
             [[tab]]\nname = \"b\"\ncommand = \"y\"\nkey = \"d\"",
            "[[merge]]\nname = \"a\"\ncommand = \" \"",
            "[[merge]]\nname = \"a\"",
            "[[merge]]\nname = \"a\"\ncommand = \"x\"\nthen = \"keep\"",
        ] {
            assert!(Config::parse(text).is_err(), "accepted: {text}");
        }
    }
}
