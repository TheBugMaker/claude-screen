//! The finder (leader f): fuzzy search over open sessions and the past sessions of a project.

use std::path::{Path, PathBuf};

use ccs_claude::{
    history::{self, PastSession},
    session::Session,
};
use frizbee::{Config, Matcher};
use ratatui::widgets::ListState;

use crate::{app::View, git, input::Input};

/// Characters of each prompt that are searched and shown. Pasted prompts can be very long.
const PROMPT_CHARS: usize = 300;

/// What a background scan of a project's past sessions sends back: the project and its sessions.
pub type Scanned = (PathBuf, Result<Vec<OldSession>, String>);

pub struct Finder {
    /// The project whose past sessions are listed.
    pub cwd: PathBuf,
    pub query: Input,
    pub old: Vec<OldSession>,
    /// Waiting for the scan of past sessions.
    pub loading: bool,
    /// Only search sessions open in ccs (Shift-Tab).
    pub active_only: bool,
    pub items: Vec<Item>,
    /// Items matching the query, best first.
    pub hits: Vec<Hit>,
    pub list: ListState,
}

/// A past session, with the prompts you typed in it.
pub struct OldSession {
    pub past: PastSession,
    pub prompts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// An open session, by index in `App::views`.
    View(usize),
    /// A past session that is not open, by index in `Finder::old`.
    Old(usize),
}

/// One session, as the finder searches and shows it.
#[derive(Debug)]
pub struct Item {
    pub target: Target,
    pub title: String,
    pub prompts: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Hit {
    /// Index in `Finder::items`.
    pub item: usize,
    /// The prompt that matched, by index in the item's prompts, when it matched better than the
    /// title.
    pub prompt: Option<usize>,
}

impl Finder {
    /// Lists the open sessions. Past ones come later, with [`Finder::scanned`].
    pub fn new(cwd: PathBuf, views: &[View]) -> Self {
        let mut finder = Self {
            cwd,
            query: Input::default(),
            old: Vec::new(),
            loading: true,
            active_only: false,
            items: Vec::new(),
            hits: Vec::new(),
            list: ListState::default(),
        };
        finder.items = items(&open(views), &finder.old);
        finder.refresh();
        finder
    }

    pub fn scanned(&mut self, old: Vec<OldSession>, views: &[View]) {
        self.old = old;
        self.loading = false;
        self.relist(views);
    }

    /// Lists the open sessions again, as after one closed: a closed session is a past one again.
    pub fn relist(&mut self, views: &[View]) {
        self.items = items(&open(views), &self.old);
        self.refresh();
    }

    /// Matches the query again and selects the best hit.
    pub fn refresh(&mut self) {
        self.hits = rank(self.query.text(), &self.items, self.active_only);
        self.list.select((!self.hits.is_empty()).then_some(0));
    }

    pub fn selected(&self) -> Option<Target> {
        let hit = self.hits.get(self.list.selected()?)?;
        Some(self.items[hit.item].target)
    }
}

/// The past sessions of `cwd`, and of the worktrees Claude made for sessions in its repository,
/// with their prompts. Reads every transcript, so run it off the UI.
pub fn scan(cwd: &Path) -> Result<Vec<OldSession>, String> {
    let dir = history::projects_dir().map_err(|e| e.to_string())?;
    // Claude makes them in the main checkout, wherever in it, or in which worktree, it started.
    let top = git::toplevel(cwd).unwrap_or_else(|| cwd.to_owned());
    let repo = history::worktree_repo(&top).unwrap_or(&top);
    let sessions = history::list_sessions(&dir, cwd, repo).map_err(|e| e.to_string())?;
    Ok(sessions
        .into_iter()
        .map(|past| {
            let prompts = history::user_prompts(&past.path)
                .unwrap_or_default()
                .iter()
                .map(|p| clip(p))
                .collect();
            OldSession { past, prompts }
        })
        .collect())
}

fn open(views: &[View]) -> Vec<(&Session, &Path)> {
    views
        .iter()
        // Named after the project, not the worktree it works in.
        .map(|v| (&v.session, v.repo.as_path()))
        .collect()
}

/// Open sessions in session bar order, then past ones that are not open, newest first. `open`
/// are the sessions and their directories.
fn items(open: &[(&Session, &Path)], old: &[OldSession]) -> Vec<Item> {
    let find_old = |id: Option<&str>| old.iter().find(|o| Some(o.past.id.as_str()) == id);
    let mut items: Vec<Item> = open
        .iter()
        .enumerate()
        .map(|(i, &(session, cwd))| {
            // Its transcript file, found by the scan when it ran in the finder's project.
            let file = find_old(session.session_id.as_deref());
            let title = session
                .name
                .clone()
                .or_else(|| file.map(|o| past_title(&o.past)))
                .unwrap_or_else(|| {
                    cwd.file_name().map_or_else(
                        || cwd.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    )
                });
            Item {
                target: Target::View(i),
                title,
                prompts: file.map(|o| o.prompts.clone()).unwrap_or_default(),
            }
        })
        .collect();
    let open_ids: Vec<&str> = open
        .iter()
        .filter_map(|(s, _)| s.session_id.as_deref())
        .collect();
    items.extend(
        old.iter()
            .enumerate()
            .filter(|(_, o)| !open_ids.contains(&o.past.id.as_str()))
            .map(|(i, o)| Item {
                target: Target::Old(i),
                title: past_title(&o.past),
                prompts: o.prompts.clone(),
            }),
    );
    items
}

/// The name given to a past session, or else its title.
fn past_title(past: &PastSession) -> String {
    past.name.clone().unwrap_or_else(|| past.title.clone())
}

/// Items matching `query` by title or one of their prompts, best first. Equal scores
/// keep the order of `items`. An empty query matches everything.
fn rank(query: &str, items: &[Item], active_only: bool) -> Vec<Hit> {
    let shown = items
        .iter()
        .enumerate()
        .filter(|(_, item)| !active_only || matches!(item.target, Target::View(_)));
    if query.trim().is_empty() {
        return shown.map(|(item, _)| Hit { item, prompt: None }).collect();
    }
    let mut fields = Vec::new();
    let mut haystacks: Vec<&str> = Vec::new();
    for (i, item) in shown {
        fields.push((i, None));
        haystacks.push(&item.title);
        for (p, prompt) in item.prompts.iter().enumerate() {
            fields.push((i, Some(p)));
            haystacks.push(prompt);
        }
    }
    // Sorted by score, then by index: the first match of an item is its best.
    let mut hits: Vec<Hit> = Vec::new();
    for found in Matcher::from_query(query, &Config::default()).match_list(&haystacks) {
        let (item, prompt) = fields[found.index as usize];
        if !hits.iter().any(|h| h.item == item) {
            hits.push(Hit { item, prompt });
        }
    }
    hits
}

/// The start of a prompt on one line.
fn clip(text: &str) -> String {
    text.chars()
        .take(PROMPT_CHARS)
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(target: Target, title: &str, prompts: &[&str]) -> Item {
        Item {
            target,
            title: title.into(),
            prompts: prompts.iter().map(|&p| p.into()).collect(),
        }
    }

    fn sample() -> Vec<Item> {
        vec![
            item(
                Target::View(0),
                "Fix the login bug",
                &["the login form rejects valid passwords"],
            ),
            item(Target::View(1), "Bump CI", &[]),
            item(
                Target::Old(0),
                "Add rate limiting",
                &["limit requests per user", "use a token bucket"],
            ),
        ]
    }

    fn found(query: &str, active_only: bool) -> Vec<(usize, Option<usize>)> {
        rank(query, &sample(), active_only)
            .into_iter()
            .map(|h| (h.item, h.prompt))
            .collect()
    }

    #[test]
    fn empty_query_lists_everything_in_order() {
        assert_eq!(found("", false), [(0, None), (1, None), (2, None)]);
        assert_eq!(found("  ", true), [(0, None), (1, None)]);
    }

    #[test]
    fn matches_titles() {
        assert_eq!(found("login", false), [(0, None)]);
        assert_eq!(found("bump", false), [(1, None)]);
    }

    #[test]
    fn a_prompt_match_shows_the_prompt() {
        assert_eq!(found("token bucket", false), [(2, Some(1))]);
    }

    #[test]
    fn active_only_leaves_out_past_sessions() {
        assert_eq!(found("rate", false), [(2, None)]);
        assert_eq!(found("rate", true), []);
    }

    #[test]
    fn query_syntax_on_its_own_does_not_panic() {
        for query in ["!", "'", "^", "$", "!^", "^$", "'!", "! ^ $"] {
            rank(query, &sample(), false);
        }
    }

    #[test]
    fn short_queries_do_not_match_everything() {
        assert_eq!(found("xyz", false), []);
    }

    #[test]
    fn open_sessions_are_not_listed_again() {
        let past = |id: &str| OldSession {
            past: PastSession {
                id: id.into(),
                path: PathBuf::new(),
                title: format!("title {id}"),
                name: (id == "c").then(|| "named c".into()),
                cwd: None,
                modified: std::time::SystemTime::UNIX_EPOCH,
            },
            prompts: Vec::new(),
        };
        let mut resumed = Session::default();
        resumed.session_id = Some("a".into());
        let mut renamed = Session::default();
        renamed.name = Some("named here".into());
        let fresh = Session::default();
        let cwd = Path::new("/work/web");
        let mut old = [past("a"), past("b"), past("c")];
        old[0].prompts = vec!["fix the login bug".into()];
        let items = items(&[(&resumed, cwd), (&renamed, cwd), (&fresh, cwd)], &old);
        let found: Vec<(Target, &str)> =
            items.iter().map(|i| (i.target, i.title.as_str())).collect();
        // An open session takes its title from its transcript file, or else its directory. A name
        // comes first.
        assert_eq!(
            found,
            [
                (Target::View(0), "title a"),
                (Target::View(1), "named here"),
                (Target::View(2), "web"),
                (Target::Old(1), "title b"),
                (Target::Old(2), "named c"),
            ]
        );
        // Its prompts come from there too.
        assert_eq!(items[0].prompts, ["fix the login bug"]);
    }
}
