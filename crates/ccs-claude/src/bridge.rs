//! What the ccs-bridge mod reports from inside an interactive `claude`.
//! The mod lives in `mods/ccs-bridge`; ccs applies its reports with `Session::bridge`.

use std::path::PathBuf;

use serde::Deserialize;

/// One report, as the mod sends it: `{"ev": "ready", "session_id": "…", "root": "…"}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum Bridge {
    /// The session started or resumed, and waits for a prompt. `root` is where it works: the
    /// worktree Claude made for it, if it did.
    Ready { session_id: String, root: PathBuf },
    /// You submitted a prompt. Slash commands like `/context` are not prompts.
    Submitted,
    /// A turn started.
    Working,
    /// A permission dialog (or a question from Claude) is shown.
    Permission,
    /// The dialog was answered and the tool runs.
    Approved,
    /// The dialog was answered no.
    Denied,
    /// The turn is ending, but work Claude started in the background, or a scheduled wakeup,
    /// will start its next one. `Idle` follows.
    Asleep,
    /// The turn ended, answered or interrupted.
    Idle,
    /// The session was named with `/rename`.
    Title { title: String },
    /// Claude moved to another directory, as into or out of a worktree.
    Root { root: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_reports_of_the_mod() {
        let parse = |line: &str| serde_json::from_str::<Bridge>(line).unwrap();
        assert_eq!(
            parse(r#"{"ev":"ready","session_id":"5c1c","root":"/work"}"#),
            Bridge::Ready {
                session_id: "5c1c".into(),
                root: "/work".into(),
            }
        );
        assert_eq!(parse(r#"{"ev":"approved"}"#), Bridge::Approved);
        assert_eq!(parse(r#"{"ev":"asleep"}"#), Bridge::Asleep);
        assert_eq!(
            parse(r#"{"ev":"title","title":"probe"}"#),
            Bridge::Title {
                title: "probe".into()
            }
        );
        assert_eq!(
            parse(r#"{"ev":"root","root":"/work/.claude/worktrees/a"}"#),
            Bridge::Root {
                root: "/work/.claude/worktrees/a".into()
            }
        );
        assert!(serde_json::from_str::<Bridge>(r#"{"ev":"later"}"#).is_err());
    }
}
