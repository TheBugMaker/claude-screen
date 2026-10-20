//! `~/.config/ccs/config.toml`: the leader key and the `[keys]` of each action, the theme, the
//! slide between sessions, worktrees, plan mode, the `[[tab]]` definitions and the `[[merge]]`
//! actions. A project's `.ccs/config.toml` changes any of them for the sessions there.

use std::{collections::BTreeMap, path::Path};

use serde::Deserialize;

use crate::{
    Error,
    keys::{Action, Binding, Bindings, Key, KeyCode},
};

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
on conflicts in this checkout. Resolve each conflict, keeping what both sides meant, mark each \
resolved file with `git add`, and run the project's checks if it has them. Don't commit the merge \
and don't abort it: I review it and commit it myself. If a conflict can't be resolved safely, \
stop and explain it.";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Key that starts a ccs shortcut, e.g. `<C-Space>`.
    pub leader: Key,
    /// The keys of every action: the config's, else the defaults.
    pub keys: BTreeMap<Action, Bindings>,
    /// Name of the theme: a file in `~/.config/ccs/themes`, or a built-in one. Read by the app.
    pub theme: Option<String>,
    /// How long switching sessions slides the new one in over 100 columns, in milliseconds: a
    /// wider window takes longer, at the same speed. 0 switches at once.
    pub slide_ms: u64,
    /// Each new session works in its own git worktree, which Claude makes (`claude --worktree`),
    /// and its tabs run there. Sessions outside git, or before the first commit, get none.
    pub worktree: bool,
    /// A new session starts in Claude's plan mode (`claude --permission-mode plan`). Resumed ones,
    /// and one opened to fix a merge's conflicts, start in your own default mode.
    pub plan: bool,
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
    /// session's own Claude gets it for conflicts in its directory, a new merge session in the
    /// main checkout for conflicts there, where leader m then commits the merge. Without it, a
    /// conflict only gets a notice.
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
    /// Go to the tab.
    #[serde(default)]
    pub key: Bindings,
    /// Show the tab beside the one on screen, or take it off the screen if it is there.
    #[serde(default)]
    pub split_key: Bindings,
    #[serde(default)]
    pub mode: Mode,
    /// Run with `sh -c` in the session's directory, with the `CS_*` variables set.
    pub command: String,
}

/// A project's `.ccs/config.toml`: only what it changes of your config. What it sets replaces
/// yours, and a `[[tab]]` or `[[merge]]` with the name of one of yours changes only the fields it
/// lists.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ProjectConfig {
    leader: Option<Key>,
    keys: BTreeMap<Action, Bindings>,
    theme: Option<String>,
    slide_ms: Option<u64>,
    worktree: Option<bool>,
    plan: Option<bool>,
    /// Your tabs it takes away, by name. Names you don't have are skipped.
    disabled_tabs: Vec<String>,
    #[serde(rename = "tab")]
    tabs: Vec<TabPatch>,
    #[serde(rename = "merge")]
    merges: Vec<MergePatch>,
}

/// A `[[tab]]` of a project: changes to your tab of that name, else a new tab.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TabPatch {
    name: String,
    key: Option<Bindings>,
    split_key: Option<Bindings>,
    mode: Option<Mode>,
    command: Option<String>,
}

/// A `[[merge]]` of a project: changes to your action of that name, else a new action.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MergePatch {
    name: String,
    commit: Option<String>,
    command: Option<String>,
    on_conflict: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Run every time the tab comes on screen.
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
            leader: Key {
                code: KeyCode::Char(' '),
                ctrl: true,
                alt: false,
            },
            keys: Action::ALL
                .into_iter()
                .map(|action| (action, action.default_bindings()))
                .collect(),
            theme: None,
            slide_ms: 150,
            worktree: true,
            plan: true,
            tabs: vec![
                TabDef {
                    name: "review".into(),
                    key: Bindings::after_leader(Key::char('r')),
                    split_key: Bindings::after_leader(Key::char('R')),
                    mode: Mode::Eager,
                    command: "git diff".into(),
                },
                TabDef {
                    name: "shell".into(),
                    key: Bindings::after_leader(Key::char('s')),
                    split_key: Bindings::after_leader(Key::char('S')),
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
        let mut config: Self = toml::from_str(text)?;
        // `[keys]` lists only the actions it changes.
        for action in Action::ALL {
            config
                .keys
                .entry(action)
                .or_insert_with(|| action.default_bindings());
        }
        config.validate()?;
        Ok(config)
    }

    /// Applies the project config at `path` over this one; a missing file changes nothing.
    pub fn load_project(self, path: &Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(text) => self.apply(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(self),
            Err(err) => Err(Error::Read(err)),
        }
    }

    /// Applies a project config over this one. Disabled tabs go after the others are changed, so
    /// disabling one wins; the result is checked as a whole, so a disabled tab's keys are free.
    pub fn apply(mut self, text: &str) -> Result<Self, Error> {
        let project: ProjectConfig = toml::from_str(text)?;
        self.leader = project.leader.unwrap_or(self.leader);
        self.keys.extend(project.keys);
        self.theme = project.theme.or(self.theme);
        self.slide_ms = project.slide_ms.unwrap_or(self.slide_ms);
        self.worktree = project.worktree.unwrap_or(self.worktree);
        self.plan = project.plan.unwrap_or(self.plan);
        for patch in project.tabs {
            let Some(tab) = self.tabs.iter_mut().find(|t| t.name == patch.name) else {
                let command = patch.command.ok_or_else(|| {
                    Error::Invalid(format!("tab {:?}: a new tab needs a command", patch.name))
                })?;
                self.tabs.push(TabDef {
                    name: patch.name,
                    key: patch.key.unwrap_or_default(),
                    split_key: patch.split_key.unwrap_or_default(),
                    mode: patch.mode.unwrap_or_default(),
                    command,
                });
                continue;
            };
            if let Some(key) = patch.key {
                tab.key = key;
            }
            if let Some(split_key) = patch.split_key {
                tab.split_key = split_key;
            }
            if let Some(mode) = patch.mode {
                tab.mode = mode;
            }
            if let Some(command) = patch.command {
                tab.command = command;
            }
        }
        for patch in project.merges {
            let Some(merge) = self.merges.iter_mut().find(|m| m.name == patch.name) else {
                let command = patch.command.ok_or_else(|| {
                    Error::Invalid(format!(
                        "merge {:?}: a new action needs a command",
                        patch.name
                    ))
                })?;
                self.merges.push(MergeDef {
                    name: patch.name,
                    commit: patch.commit.unwrap_or_else(merge_commit),
                    command,
                    on_conflict: patch.on_conflict,
                });
                continue;
            };
            if let Some(commit) = patch.commit {
                merge.commit = commit;
            }
            if let Some(command) = patch.command {
                merge.command = command;
            }
            if patch.on_conflict.is_some() {
                merge.on_conflict = patch.on_conflict;
            }
        }
        self.tabs
            .retain(|tab| !project.disabled_tabs.contains(&tab.name));
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), Error> {
        if !self.leader.ctrl && !self.leader.alt {
            return Err(Error::Invalid(format!(
                "leader: {:?} needs ctrl or alt, like <C-Space>: it would take that key from what \
                 you type",
                self.leader.to_string()
            )));
        }
        // What each key does, and the action it is, if it isn't a tab.
        let owners: Vec<(String, &Bindings, Option<Action>)> = self
            .keys
            .iter()
            .map(|(&action, keys)| (format!("keys.{}", action.name()), keys, Some(action)))
            .chain(self.tabs.iter().flat_map(|tab| {
                [
                    (format!("tab {:?}", tab.name), &tab.key, None),
                    (
                        format!("tab {:?} split_key", tab.name),
                        &tab.split_key,
                        None,
                    ),
                ]
            }))
            .collect();
        for (i, (owner, keys, action)) in owners.iter().enumerate() {
            for &binding in &keys.0 {
                let Binding { leader, key } = binding;
                if !leader && !key.ctrl && !key.alt {
                    let after = Binding { leader: true, key };
                    let alt = Key { alt: true, ..key };
                    return Err(Error::Invalid(format!(
                        "{owner}: {:?} needs <leader> ({after}), or ctrl or alt ({alt}): on its \
                         own it would take that key from what you type",
                        binding.to_string()
                    )));
                }
                if key == self.leader {
                    return Err(Error::Invalid(format!(
                        "{owner}: {:?} is the leader's: twice sends it to the program in the tab",
                        binding.to_string()
                    )));
                }
                // Two can share a key only if they never work in the same tab.
                let works = |action: &Option<Action>, peer| action.is_none_or(|a| a.works_in(peer));
                let shared = owners[..i].iter().find(|(_, other_keys, other)| {
                    other_keys.0.contains(&binding)
                        && [false, true]
                            .into_iter()
                            .any(|peer| works(action, peer) && works(other, peer))
                });
                if let Some((other, _, _)) = shared {
                    return Err(Error::Invalid(format!(
                        "{other} and {owner} both have {:?}",
                        binding.to_string()
                    )));
                }
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
        // As the default tabs have them.
        assert_eq!(config.tabs[..2], Config::default().tabs);
        assert_eq!(config.slide_ms, 150);
        assert!(config.worktree);
        assert!(config.plan);
        // It shows the default action and keys as they are.
        assert_eq!(config.merges, Config::default().merges);
        assert_eq!(config.keys, Config::default().keys);
    }

    #[test]
    fn missing_file_and_missing_tabs_give_the_defaults() {
        let config = Config::load(Path::new("/nonexistent/ccs/config.toml")).unwrap();
        assert_eq!(config, Config::default());
        let config = Config::parse("leader = \"<C-a>\"").unwrap();
        assert_eq!(config.leader, Key::parse("<C-a>").unwrap());
        assert_eq!(config.keys, Config::default().keys);
        assert_eq!(config.slide_ms, 150);
        assert_eq!(config.tabs, Config::default().tabs);
        // 0 turns the slide off.
        assert_eq!(Config::parse("slide_ms = 0").unwrap().slide_ms, 0);
        assert!(config.worktree);
        assert!(!Config::parse("worktree = false").unwrap().worktree);
        assert!(config.plan);
        assert!(!Config::parse("plan = false").unwrap().plan);
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
        assert_eq!(config.leader, Config::default().leader);
        assert_eq!(
            config.tabs,
            vec![TabDef {
                name: "top".into(),
                key: Bindings::default(),
                split_key: Bindings::default(),
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
    fn keys_take_lists_and_keep_the_defaults_of_the_others() {
        let config = Config::parse(
            "[keys]\nfind = [\"<leader>f\", \"<C-f>\"]\nsession_1 = \"<A-1>\"\nquit = []\n\
             [[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = [\"<leader>r\", \"<A-r>\"]\n\
             split_key = [\"<leader>R\", \"<A-R>\"]",
        )
        .unwrap();
        let keys =
            |texts: &[&str]| Bindings(texts.iter().map(|t| Binding::parse(t).unwrap()).collect());
        assert_eq!(config.keys[&Action::Find], keys(&["<leader>f", "<C-f>"]));
        assert_eq!(config.keys[&Action::Session1], keys(&["<A-1>"]));
        assert_eq!(config.keys[&Action::Quit], keys(&[]));
        assert_eq!(config.keys[&Action::Merge], keys(&["<leader>m"]));
        assert_eq!(config.tabs[0].key, keys(&["<leader>r", "<A-r>"]));
        assert_eq!(config.tabs[0].split_key, keys(&["<leader>R", "<A-R>"]));
        assert_eq!(config.keys[&Action::ChatSplit], keys(&["<leader>C"]));
        assert_eq!(config.keys[&Action::OtherPane], keys(&["<leader>o"]));
        // Enter does one thing in Chat, another in a terminal tab: so can any key.
        assert!(
            Config::parse("[keys]\nsubmit_and_stay = \"<A-x>\"\nrerun_tab = \"<A-x>\"").is_ok()
        );
        // The leader can go after the leader with another key.
        assert!(Config::parse("[keys]\nquit = \"<leader><C-q>\"").is_ok());
    }

    #[test]
    fn example_project_config_applies() {
        let config = Config::default()
            .apply(include_str!("../../../config/example-project.toml"))
            .unwrap();
        let names: Vec<&str> = config.tabs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["review", "tests"]);
        assert!(!config.plan);
        assert_eq!(config.tabs[1].key, Bindings::after_leader(Key::char('t')));
    }

    #[test]
    fn a_project_changes_only_what_it_sets() {
        let yours = Config::parse("slide_ms = 0\n[keys]\nquit = \"<A-q>\"").unwrap();
        let config = yours
            .clone()
            .apply("plan = false\n[keys]\nfind = \"<A-f>\"")
            .unwrap();
        assert_eq!(config.slide_ms, 0);
        assert!(!config.plan);
        assert!(config.worktree);
        assert_eq!(config.leader, yours.leader);
        assert_eq!(
            config.keys[&Action::Find],
            Bindings(vec![Binding::parse("<A-f>").unwrap()])
        );
        assert_eq!(config.keys[&Action::Quit], yours.keys[&Action::Quit]);
        assert_eq!(config.tabs, yours.tabs);
        assert_eq!(config.merges, yours.merges);
        // Nothing at all, or no file: your config as it is.
        assert_eq!(yours.clone().apply("").unwrap(), yours);
        let missing = Path::new("/nonexistent/project/.ccs/config.toml");
        assert_eq!(yours.clone().load_project(missing).unwrap(), yours);
    }

    #[test]
    fn a_project_tab_changes_yours_of_that_name_or_is_added() {
        let config = Config::default()
            .apply(
                "[[tab]]\nname = \"review\"\ncommand = \"git diff --stat\"\n\
                 [[tab]]\nname = \"tests\"\ncommand = \"cargo test\"\nmode = \"regular\"",
            )
            .unwrap();
        let [review, shell, tests] = &config.tabs[..] else {
            panic!("{:?}", config.tabs);
        };
        assert_eq!(
            *review,
            TabDef {
                command: "git diff --stat".into(),
                ..Config::default().tabs[0].clone()
            }
        );
        assert_eq!(*shell, Config::default().tabs[1]);
        assert_eq!(
            *tests,
            TabDef {
                name: "tests".into(),
                key: Bindings::default(),
                split_key: Bindings::default(),
                mode: Mode::Regular,
                command: "cargo test".into(),
            }
        );
        // A new one has nothing to start from.
        assert!(
            Config::default()
                .apply("[[tab]]\nname = \"tests\"")
                .is_err()
        );
    }

    #[test]
    fn a_project_merge_changes_yours_of_that_name_or_is_added() {
        let config = Config::default()
            .apply(
                "[[merge]]\nname = \"merge\"\ncommand = \"git push\"\n\
                 [[merge]]\nname = \"pr\"\ncommand = \"gh pr create --fill\"",
            )
            .unwrap();
        assert_eq!(
            config.merges,
            vec![
                MergeDef {
                    command: "git push".into(),
                    ..Config::default().merges[0].clone()
                },
                MergeDef {
                    name: "pr".into(),
                    commit: MERGE_COMMIT.into(),
                    command: "gh pr create --fill".into(),
                    on_conflict: None,
                },
            ]
        );
        assert!(Config::default().apply("[[merge]]\nname = \"pr\"").is_err());
        assert!(
            Config::default()
                .apply("[[merge]]\nname = \"merge\"\ncommand = \" \"")
                .is_err()
        );
    }

    #[test]
    fn a_project_disables_tabs_and_frees_their_keys() {
        let names = |config: &Config| -> Vec<String> {
            config.tabs.iter().map(|t| t.name.clone()).collect()
        };
        let config = Config::default()
            .apply("disabled_tabs = [\"shell\", \"lazygit\"]")
            .unwrap();
        assert_eq!(names(&config), ["review"]);
        // Disabling wins over changing it.
        let config = Config::default()
            .apply("disabled_tabs = [\"shell\"]\n[[tab]]\nname = \"shell\"\ncommand = \"zsh\"")
            .unwrap();
        assert_eq!(names(&config), ["review"]);
        // <leader>r is the review tab's, until it goes.
        let tab = "[[tab]]\nname = \"run\"\nkey = \"<leader>r\"\ncommand = \"cargo run\"";
        assert!(Config::default().apply(tab).is_err());
        let config = Config::default()
            .apply(&format!("disabled_tabs = [\"review\"]\n{tab}"))
            .unwrap();
        assert_eq!(names(&config), ["shell", "run"]);
    }

    #[test]
    fn rejects_bad_project_configs() {
        for text in [
            "tabs_off = 1",
            "disabled_tabs = \"shell\"",
            "[[tab]]\nname = \"review\"\nkind = \"claude\"",
            "[[tab]]\ncommand = \"x\"",
            "[[tab]]\nname = \"review\"\nkey = \"r\"",
            "[[merge]]\nname = \"merge\"\nthen = \"keep\"",
            "leader = \"a\"",
        ] {
            assert!(Config::default().apply(text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn a_bad_key_says_where_it_is() {
        let text = "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"ctrl-r\"";
        let Err(Error::Parse(err)) = Config::parse(text) else {
            panic!("accepted: {text}");
        };
        let err = err.to_string();
        assert!(err.contains("line 4") && err.contains("<C-f>"), "{err}");
    }

    #[test]
    fn rejects_bad_configs() {
        let tab = |key: &str| format!("[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = {key}");
        for text in [
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkind = \"claude\"".to_owned(),
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nmode = \"on_change\"".to_owned(),
            // A key of its own, or taken.
            tab("\"r\""),
            tab("\"<leader>3\""),
            tab("\"<leader>f\""),
            tab("\"<leader>c\""),
            tab("\"<leader>N\""),
            tab("\"<leader><CR>\""),
            tab("[\"<A-r>\", \"<leader>m\"]"),
            tab("\"<leader><C-Space>\""),
            tab("\"<C-Space>\""),
            tab("3"),
            // A split key of its own, or taken: by an action, by the tab's own key.
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nsplit_key = \"R\"".to_owned(),
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nsplit_key = \"<leader>C\"".to_owned(),
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nsplit_key = \"<leader>o\"".to_owned(),
            "[[tab]]\nname = \"a\"\ncommand = \"x\"\nkey = \"<A-a>\"\nsplit_key = \"<A-a>\""
                .to_owned(),
            format!("{}\n{}", tab("\"<leader>d\""), tab("\"<leader>d\"")),
            "[keys]\nfind = \"f\"".to_owned(),
            "[keys]\nfind = \"<leader>q\"".to_owned(),
            "[keys]\nfind = \"<A-x>\"\nmerge = \"<A-x>\"".to_owned(),
            "[keys]\nsubmit_and_stay = \"<leader>x\"\nsubmit_and_new_session = \"<leader>x\""
                .to_owned(),
            "[keys]\nopen = \"<leader>o\"".to_owned(),
            // The old way, and a leader without ctrl or alt.
            "leader = \"ctrl-space\"".to_owned(),
            "leader = \"a\"".to_owned(),
            "[[merge]]\nname = \"a\"\ncommand = \" \"".to_owned(),
            "[[merge]]\nname = \"a\"".to_owned(),
            "[[merge]]\nname = \"a\"\ncommand = \"x\"\nthen = \"keep\"".to_owned(),
        ] {
            assert!(Config::parse(&text).is_err(), "accepted: {text}");
        }
    }
}
