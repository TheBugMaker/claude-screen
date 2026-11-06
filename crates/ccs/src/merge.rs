//! Leader m: getting a session's work into its target with a `[[merge]]` action, and telling
//! from what the action left behind how it went.

use ccs_core::config::MergeDef;

use crate::input::Input;

/// What leader m runs in a merge session once its conflicts are fixed: the commit that finishes
/// the merge, with what you changed in tracked files while reviewing it. Git's own message keeps
/// the `Merge branch '…'` the finder tells merged sessions by.
pub const FINISH: &str = "git commit -a --no-edit";

/// The panel leader m opens: what the merge takes, the commit message, and the action.
pub struct MergePanel {
    /// The session it merges, by key.
    pub view: u64,
    /// The branch checked out in the session's directory.
    pub branch: String,
    /// The branch checked out in the session's `repo`, which the work goes into. `None` on a
    /// detached HEAD there.
    pub target: Option<String>,
    /// The changes not committed yet, as `git status --short` lists them. They are committed
    /// first.
    pub changes: Vec<String>,
    /// Commits the session's branch has that `target` doesn't.
    pub ahead: usize,
    /// Other sessions working in the main checkout, whose files a merge there changes.
    pub others: usize,
    pub message: Input,
    /// The `[[merge]]` to run, by its place in the config.
    pub action: usize,
    /// What its `params` ask for.
    pub params: Vec<Param>,
    /// The field that gets your keys: the message, or `params[i]`.
    pub focus: Option<usize>,
    /// Enter was pressed with changes to commit, but no message.
    pub needs_message: bool,
    /// It finishes the merge of another session, whose conflicts this one fixed: it commits it,
    /// with git's message.
    pub finish: bool,
}

/// What asks you before a session opens to fix the conflicts a merge stopped on in the main
/// checkout: Enter opens it, Esc gives the merge up.
pub struct ConflictPanel {
    /// The session merged, by key.
    pub view: u64,
    /// The branch it merged.
    pub branch: String,
    pub target: String,
    pub repo: std::path::PathBuf,
    /// The files in conflict.
    pub files: Vec<String>,
    /// What the session that opens starts with.
    pub prompt: String,
}

/// A value the action picked in the panel asks for: one of its `params`.
pub struct Param {
    pub name: String,
    /// What the panel shows before it.
    pub label: String,
    pub input: Input,
}

impl MergePanel {
    /// The session works on the target branch itself, without a worktree: there is only
    /// committing to do.
    pub fn plain(&self) -> bool {
        self.target.as_deref() == Some(self.branch.as_str())
    }

    /// The commit message, when there is something to commit: what was typed, trimmed. None when
    /// it finishes a merge.
    pub fn commit_message(&self) -> Option<&str> {
        (!self.finish && !self.changes.is_empty()).then(|| self.message.text().trim())
    }

    /// Asks for what `def` takes, its defaults filled in from `vars`. A param the action before
    /// had too keeps what was typed in it. The field that had your keys keeps them if it is still
    /// there.
    pub fn set_params(&mut self, def: &MergeDef, vars: &[(String, String)]) {
        let old = std::mem::take(&mut self.params);
        self.params = def
            .params
            .iter()
            .map(|param| {
                let mut input = Input::default();
                match old.iter().find(|o| o.name == param.name) {
                    Some(old) => input.insert(old.input.text()),
                    None => input.insert(&expand(param.default.as_deref().unwrap_or(""), vars)),
                }
                Param {
                    name: param.name.clone(),
                    label: param.prompt.clone().unwrap_or_else(|| param.name.clone()),
                    input,
                }
            })
            .collect();
        let gone = match self.focus {
            Some(i) => i >= self.params.len(),
            None => self.commit_message().is_none(),
        };
        if gone {
            self.focus = (self.commit_message().is_none() && !self.params.is_empty()).then_some(0);
        }
    }

    /// Gives your keys to the next field, or the one before: the message when there is one,
    /// then the params.
    pub fn move_focus(&mut self, back: bool) {
        let message = usize::from(self.commit_message().is_some());
        let fields = message + self.params.len();
        if fields == 0 {
            return;
        }
        let at = self.focus.map_or(0, |i| message + i);
        let next = (if back { at + fields - 1 } else { at + 1 }) % fields;
        self.focus = next.checked_sub(message);
    }

    /// `CS_<NAME>` for each param, set to what was typed, trimmed.
    pub fn param_vars(&self) -> Vec<(String, String)> {
        self.params
            .iter()
            .map(|p| {
                let name = format!("CS_{}", p.name.to_ascii_uppercase());
                (name, p.input.text().trim().to_owned())
            })
            .collect()
    }
}

/// Whether the session's directory and its main checkout have files in conflict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unmerged {
    pub cwd: bool,
    pub repo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The work is in its target.
    Merged,
    /// The action stopped on conflicts there.
    Conflict(Place),
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The session's own directory, which only its own Claude can work in when it is a worktree.
    Cwd,
    /// The main checkout.
    Repo,
}

/// How an action whose tab exited with `status` went. `before` and `after`: the conflicts before
/// it ran and after. Only new ones count, wherever the action stopped on them, even if it exited
/// 0 after. Those in the session's directory come first, as its own Claude can fix them.
pub fn outcome(status: &str, before: Unmerged, after: Unmerged) -> Outcome {
    if after.cwd && !before.cwd {
        Outcome::Conflict(Place::Cwd)
    } else if after.repo && !before.repo {
        Outcome::Conflict(Place::Repo)
    } else if status == "exit 0" {
        Outcome::Merged
    } else {
        Outcome::Failed
    }
}

/// What the merge tab runs: `def`'s commit, if there is something to commit, then its command.
/// The command runs only if the commit worked, and isn't in a subshell, so `set -e` works there.
pub fn script(def: &MergeDef, commit: bool) -> String {
    if commit {
        format!("(\n{}\n) || exit\n{}", def.commit, def.command)
    } else {
        def.command.clone()
    }
}

/// `text` with each `$NAME` or `${NAME}` of `vars` replaced by its value. Any other `$` stays as
/// it is: the text is a prompt, never run.
pub fn expand(text: &str, vars: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        if let Some((value, after)) = variable(rest, vars) {
            out.push_str(value);
            rest = after;
        } else {
            out.push('$');
        }
    }
    out.push_str(rest);
    out
}

/// The value of the variable of `vars` that `text` starts with, as `NAME` or `{NAME}`, and the
/// text after it.
fn variable<'a>(text: &'a str, vars: &'a [(String, String)]) -> Option<(&'a str, &'a str)> {
    let (name, after) = if let Some(braced) = text.strip_prefix('{') {
        braced.split_once('}')?
    } else {
        let end = text
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(text.len());
        text.split_at(end)
    };
    let (_, value) = vars.iter().find(|(var, _)| *var == name)?;
    Some((value, after))
}

#[cfg(test)]
mod tests {
    use ccs_core::config::{After, ParamDef};

    use super::*;

    #[test]
    fn new_conflicts_tell_where_the_action_stopped() {
        let none = Unmerged::default();
        let cwd = Unmerged {
            cwd: true,
            repo: false,
        };
        let repo = Unmerged {
            cwd: false,
            repo: true,
        };
        let both = Unmerged {
            cwd: true,
            repo: true,
        };
        assert_eq!(outcome("exit 0", none, none), Outcome::Merged);
        assert_eq!(outcome("exit 1", none, none), Outcome::Failed);
        assert_eq!(outcome("SIGHUP", none, none), Outcome::Failed);
        assert_eq!(
            outcome("exit 1", none, repo),
            Outcome::Conflict(Place::Repo)
        );
        // An action that carries on after a conflict hasn't merged.
        assert_eq!(outcome("exit 0", none, cwd), Outcome::Conflict(Place::Cwd));
        // The session's own Claude goes first.
        assert_eq!(outcome("exit 1", none, both), Outcome::Conflict(Place::Cwd));
        // Conflicts that were there before are not the action's.
        assert_eq!(outcome("exit 1", repo, repo), Outcome::Failed);
        assert_eq!(outcome("exit 1", cwd, both), Outcome::Conflict(Place::Repo));
    }

    #[test]
    fn the_commit_runs_first_and_stops_the_command_if_it_fails() {
        let def = MergeDef {
            name: "merge".into(),
            commit: "git commit -am \"$CS_MESSAGE\"".into(),
            command: "set -e\ngit merge x".into(),
            on_conflict: None,
            params: Vec::new(),
            after: After::Remove,
        };
        assert_eq!(script(&def, false), "set -e\ngit merge x");
        let both = script(&def, true);
        assert_eq!(
            both,
            "(\ngit commit -am \"$CS_MESSAGE\"\n) || exit\nset -e\ngit merge x"
        );
        let status = |script: &str| {
            std::process::Command::new("sh")
                .args(["-c", script])
                .status()
                .unwrap()
                .code()
        };
        let def = |commit: &str, command: &str| MergeDef {
            commit: commit.into(),
            command: command.into(),
            ..def.clone()
        };
        assert_eq!(status(&script(&def("exit 3", "exit 0"), true)), Some(3));
        assert_eq!(status(&script(&def("true", "exit 4"), true)), Some(4));
        // `set -e` in the command still stops it.
        assert_eq!(
            status(&script(&def("true", "set -e\nfalse\nexit 0"), true)),
            Some(1)
        );
    }

    #[test]
    fn expands_only_the_names_it_has() {
        let vars = [
            ("CS_BRANCH".to_owned(), "worktree-ccs-1".to_owned()),
            ("CS_REPO_BRANCH".to_owned(), "main".to_owned()),
        ];
        assert_eq!(
            expand(
                "Merge `$CS_BRANCH` into ${CS_REPO_BRANCH}: $CS_BRANCH.",
                &vars
            ),
            "Merge `worktree-ccs-1` into main: worktree-ccs-1."
        );
        // Unknown names, a lone `$`, an open brace and shell syntax stay as they are.
        assert_eq!(
            expand("$CS_OTHER costs $5, ${CS_BRANCH $(rm -rf) $", &vars),
            "$CS_OTHER costs $5, ${CS_BRANCH $(rm -rf) $"
        );
        // A longer name isn't a shorter one.
        assert_eq!(expand("$CS_BRANCHES", &vars), "$CS_BRANCHES");
    }

    fn panel(changes: &[&str]) -> MergePanel {
        MergePanel {
            view: 0,
            branch: "worktree-ccs-1".into(),
            target: Some("main".into()),
            changes: changes.iter().map(|&c| c.to_owned()).collect(),
            ahead: 1,
            others: 0,
            message: Input::default(),
            action: 0,
            params: Vec::new(),
            focus: None,
            needs_message: false,
            finish: false,
        }
    }

    fn action(params: &[(&str, Option<&str>)]) -> MergeDef {
        MergeDef {
            name: "mr".into(),
            commit: String::new(),
            command: "x".into(),
            on_conflict: None,
            params: params
                .iter()
                .map(|&(name, default)| ParamDef {
                    name: name.into(),
                    prompt: (name == "ticket").then(|| "ADO ticket".into()),
                    default: default.map(str::to_owned),
                })
                .collect(),
            after: After::Remove,
        }
    }

    #[test]
    fn params_start_from_their_defaults_and_keep_what_was_typed() {
        let vars = [("CS_TITLE".to_owned(), "Fix login".to_owned())];
        let mut panel = panel(&[]);
        panel.set_params(
            &action(&[("ticket", None), ("title", Some("$CS_TITLE!"))]),
            &vars,
        );
        let labels: Vec<&str> = panel.params.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["ADO ticket", "title"]);
        assert_eq!(panel.params[1].input.text(), "Fix login!");
        // Nothing to commit: the keys go to the first param.
        assert_eq!(panel.focus, Some(0));
        panel.params[0].input.insert(" 123 ");
        assert_eq!(
            panel.param_vars(),
            [
                ("CS_TICKET".to_owned(), "123".to_owned()),
                ("CS_TITLE".to_owned(), "Fix login!".to_owned()),
            ]
        );
        // Another action with a ticket keeps it; the focus was on a field that is gone.
        panel.focus = Some(1);
        panel.set_params(&action(&[("type", Some("feat")), ("ticket", None)]), &vars);
        assert_eq!(panel.params[0].input.text(), "feat");
        assert_eq!(panel.params[1].input.text(), " 123 ");
        assert_eq!(panel.focus, Some(1));
        panel.set_params(&action(&[]), &vars);
        assert_eq!(panel.focus, None);
    }

    #[test]
    fn the_focus_goes_round_the_message_and_the_params() {
        let mut panel = panel(&[" M a.rs"]);
        panel.set_params(&action(&[("a", None), ("b", None)]), &[]);
        assert_eq!(panel.focus, None);
        panel.move_focus(false);
        assert_eq!(panel.focus, Some(0));
        panel.move_focus(false);
        assert_eq!(panel.focus, Some(1));
        panel.move_focus(false);
        assert_eq!(panel.focus, None);
        panel.move_focus(true);
        assert_eq!(panel.focus, Some(1));
        // Without a message, only the params.
        let mut panel = self::panel(&[]);
        panel.set_params(&action(&[("a", None), ("b", None)]), &[]);
        panel.move_focus(true);
        assert_eq!(panel.focus, Some(1));
        panel.move_focus(false);
        assert_eq!(panel.focus, Some(0));
        // No field at all.
        let mut panel = self::panel(&[]);
        panel.move_focus(false);
        assert_eq!(panel.focus, None);
    }
}
