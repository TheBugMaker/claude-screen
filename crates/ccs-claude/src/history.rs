//! Past sessions that Claude Code keeps in `~/.claude/projects/<encoded cwd>/<id>.jsonl`: their
//! titles, directories and the prompts you typed, for the finder.

use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::Deserialize;
use serde_json::Value;

use crate::Error;

/// How much of the end of a file is scanned for its title.
const TAIL_BYTES: u64 = 64 * 1024;
/// How many lines from the start are scanned for the first prompt and cwd.
const HEAD_LINES: usize = 50;

#[derive(Debug, Clone)]
pub struct PastSession {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    /// The name given with `/rename`.
    pub name: Option<String>,
    /// Directory the session ran in, from the transcript itself.
    pub cwd: Option<PathBuf>,
    pub modified: SystemTime,
}

/// `$CLAUDE_CONFIG_DIR/projects`, or `~/.claude/projects`.
pub fn projects_dir() -> Result<PathBuf, Error> {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => etcetera::home_dir()?.join(".claude"),
    };
    Ok(base.join("projects"))
}

/// The directory name Claude Code uses for `cwd`: every non-alphanumeric character becomes `-`.
#[must_use]
pub fn encode_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The main checkout a worktree Claude made belongs to: the part of `path` before
/// `.claude/worktrees/<name>`. `None` when `path` is in no such worktree.
#[must_use]
pub fn worktree_repo(path: &Path) -> Option<&Path> {
    path.ancestors()
        // The worktree's name comes after `worktrees`.
        .skip(1)
        .find(|dir| dir.ends_with(".claude/worktrees"))
        .and_then(|dir| dir.parent()?.parent())
}

/// Sessions that ran in `cwd`, or in a worktree Claude made in the main checkout `repo`, most
/// recent first. Claude keeps a worktree's sessions apart, as for any other directory.
pub fn list_sessions(projects_dir: &Path, cwd: &Path, repo: &Path) -> io::Result<Vec<PastSession>> {
    let entries = match std::fs::read_dir(projects_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let own = encode_cwd(cwd);
    // `<repo>/.claude/worktrees/<name>`, encoded.
    let worktrees = format!("{}--claude-worktrees-", encode_cwd(repo));
    let mut sessions = Vec::new();
    for entry in entries {
        let dir = entry?;
        let name = dir.file_name();
        let name = name.to_string_lossy();
        if name != own && !name.starts_with(&worktrees) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        sessions.extend(files.filter_map(|file| read_session(&file.ok()?.path())));
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
    Ok(sessions)
}

/// Finds a session by id in any project.
pub fn find_session(projects_dir: &Path, id: &str) -> io::Result<Option<PastSession>> {
    for entry in std::fs::read_dir(projects_dir)? {
        let path = entry?.path().join(format!("{id}.jsonl"));
        if path.is_file() {
            return Ok(read_session(&path));
        }
    }
    Ok(None)
}

/// The prompts and slash commands you typed in a past session, oldest first.
pub fn user_prompts(path: &Path) -> io::Result<Vec<String>> {
    let mut prompts = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if !line.contains(r#""type":"user""#) {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Line>(&line) else {
            continue;
        };
        if line.is_sidechain || line.is_meta || line.is_compact_summary {
            continue;
        }
        if let Some(prompt) = line.message.as_ref().and_then(HistoryMessage::prompt) {
            prompts.push(prompt);
        }
    }
    Ok(prompts)
}

/// A transcript line. Only `user`/`assistant` lines have a message.
#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, rename = "isSidechain")]
    is_sidechain: bool,
    #[serde(default, rename = "isMeta")]
    is_meta: bool,
    #[serde(default, rename = "isCompactSummary")]
    is_compact_summary: bool,
    message: Option<HistoryMessage>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
    #[serde(rename = "lastPrompt")]
    last_prompt: Option<String>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
    cwd: Option<PathBuf>,
}

/// Text, or a list of content blocks: tool results in a user message.
#[derive(Deserialize)]
struct HistoryMessage {
    content: Value,
}

impl HistoryMessage {
    /// The prompt or slash command you typed, if this message is one. Command output and other
    /// tagged text are Claude Code's own.
    fn prompt(&self) -> Option<String> {
        let text = self.content.as_str()?;
        if let Some(name) = between(text, "<command-name>", "</command-name>") {
            let args = between(text, "<command-args>", "</command-args>").unwrap_or_default();
            return Some(format!("{name} {args}").trim_end().to_owned());
        }
        (!text.starts_with('<')).then(|| text.to_owned())
    }
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let len = text[start..].find(close)?;
    Some(&text[start..start + len])
}

/// Reads id, title and cwd without loading the whole file. `None` if it holds no conversation.
fn read_session(path: &Path) -> Option<PastSession> {
    if path.extension()? != "jsonl" {
        return None;
    }
    let id = path.file_stem()?.to_str()?.to_owned();
    let modified = path.metadata().ok()?.modified().ok()?;

    let mut first_prompt = None;
    let mut cwd = None;
    let file = BufReader::new(File::open(path).ok()?);
    for line in file.lines().take(HEAD_LINES) {
        let Ok(line) = serde_json::from_str::<Line>(&line.ok()?) else {
            continue;
        };
        if line.kind == "user" && !line.is_meta && !line.is_compact_summary {
            cwd = cwd.or(line.cwd);
            if let Some(prompt) = line.message.as_ref().and_then(HistoryMessage::prompt) {
                first_prompt = Some(prompt);
                break;
            }
        }
    }

    let titles = tail_titles(path).unwrap_or_default();
    let title = titles.ai_title.or(first_prompt).or(titles.last_prompt)?;
    Some(PastSession {
        id,
        path: path.to_owned(),
        title: title.lines().next().unwrap_or_default().to_owned(),
        name: titles.custom_title,
        cwd,
        modified,
    })
}

#[derive(Default)]
struct Titles {
    ai_title: Option<String>,
    last_prompt: Option<String>,
    custom_title: Option<String>,
}

/// The last AI title in a transcript, if it has one yet.
#[must_use]
pub fn ai_title(path: &Path) -> Option<String> {
    let title = tail_titles(path).ok()?.ai_title?;
    Some(title.lines().next().unwrap_or_default().to_owned())
}

/// The last `ai-title`, `last-prompt` and `custom-title` near the end of the file (all are
/// rewritten often).
fn tail_titles(path: &Path) -> io::Result<Titles> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;

    let mut titles = Titles::default();
    for line in String::from_utf8_lossy(&tail).lines() {
        if !line.contains(r#""type":"ai-title""#)
            && !line.contains(r#""type":"last-prompt""#)
            && !line.contains(r#""type":"custom-title""#)
        {
            continue;
        }
        if let Ok(line) = serde_json::from_str::<Line>(line) {
            titles.ai_title = line.ai_title.or(titles.ai_title);
            titles.last_prompt = line.last_prompt.or(titles.last_prompt);
            titles.custom_title = line.custom_title.or(titles.custom_title);
        }
    }
    Ok(titles)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/history.jsonl");

    #[test]
    fn encodes_cwd_like_claude_code() {
        assert_eq!(
            encode_cwd(Path::new("/home/me/my_repo.v2")),
            "-home-me-my-repo-v2"
        );
    }

    #[test]
    fn finds_the_main_checkout_of_a_worktree() {
        let repo = |path| worktree_repo(Path::new(path));
        assert_eq!(
            repo("/work/.claude/worktrees/ccs-1"),
            Some(Path::new("/work"))
        );
        assert_eq!(
            repo("/work/.claude/worktrees/ccs-1/src"),
            Some(Path::new("/work"))
        );
        // A repository kept in `~/.claude` has its worktrees in `~/.claude/.claude`.
        assert_eq!(
            repo("/home/me/.claude/.claude/worktrees/a"),
            Some(Path::new("/home/me/.claude"))
        );
        assert_eq!(repo("/work"), None);
        assert_eq!(repo("/work/.claude/worktrees"), None);
        assert_eq!(repo("/work/.claude"), None);
    }

    #[test]
    fn lists_the_sessions_of_a_directory_and_of_its_worktrees() {
        let projects = std::env::temp_dir().join(format!("ccs-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&projects);
        for (dir, id) in [
            ("-work", "main"),
            ("-work--claude-worktrees-ccs-1", "tree"),
            ("-work-sub", "sub"),
            ("-other", "other"),
        ] {
            std::fs::create_dir_all(projects.join(dir)).unwrap();
            std::fs::copy(FIXTURE, projects.join(dir).join(format!("{id}.jsonl"))).unwrap();
        }
        let ids = |cwd: &str, repo: &str| {
            let mut ids: Vec<String> = list_sessions(&projects, Path::new(cwd), Path::new(repo))
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(ids("/work", "/work"), ["main", "tree"]);
        // Started in a subdirectory, Claude's worktree is still the main checkout's.
        assert_eq!(ids("/work/sub", "/work"), ["sub", "tree"]);
        assert_eq!(ids("/nowhere", "/nowhere"), Vec::<String>::new());
        std::fs::remove_dir_all(&projects).unwrap();
        assert!(
            list_sessions(&projects, Path::new("/work"), Path::new("/work"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn lists_your_prompts() {
        assert_eq!(
            user_prompts(Path::new(FIXTURE)).unwrap(),
            ["the login form rejects valid passwords", "/cost"]
        );
    }

    #[test]
    fn reads_title_and_cwd() {
        let session = read_session(Path::new(FIXTURE)).unwrap();
        assert_eq!(session.id, "history");
        assert_eq!(session.title, "Fix the login bug");
        assert_eq!(session.name.as_deref(), Some("login-bug"));
        assert_eq!(session.cwd.as_deref(), Some(Path::new("/work")));
    }

    #[test]
    fn reads_the_last_ai_title() {
        assert_eq!(
            ai_title(Path::new(FIXTURE)).as_deref(),
            Some("Fix the login bug")
        );
    }
}
