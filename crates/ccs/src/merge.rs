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
    /// Enter was pressed with changes to commit, but no message.
    pub needs_message: bool,
    /// It finishes the merge of another session, whose conflicts this one fixed: it commits it,
    /// with git's message.
    pub finish: bool,
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
pub fn expand(text: &str, vars: &[(&str, String)]) -> String {
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
fn variable<'a>(text: &'a str, vars: &'a [(&str, String)]) -> Option<(&'a str, &'a str)> {
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
            ("CS_BRANCH", "worktree-ccs-1".to_owned()),
            ("CS_REPO_BRANCH", "main".to_owned()),
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
}
