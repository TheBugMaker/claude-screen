use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use ccs_claude::{
    bridge::Bridge,
    history::{self, PastSession},
    session::{Session, State},
};
use ccs_core::{
    config::{Config, Mode, TabDef},
    keys::{Action, Bindings},
};
use ccs_term::{Pty, PtyEvent};
use color_eyre::{
    Section, SectionExt,
    eyre::{OptionExt, WrapErr, eyre},
};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use futures::StreamExt;
use ratatui::{
    DefaultTerminal,
    layout::{Position, Rect},
};
use tokio::{sync::mpsc, time::Instant};

use crate::{
    finder::{self, Finder, OldSession, Scanned, Target},
    git,
    input::Input,
    keys,
    merge::{self, MergePanel, Outcome, Place, Unmerged},
    theme::ClaudeTheme,
    ui,
};

/// The Claude Code executable.
const CLAUDE: &str = "claude";
/// The mods every Claude runs with: the bridge, which reports what it does, and the others in
/// `mods/`. They run from the source tree.
const MODS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../mods");
/// How soon after your last key a submitted prompt or an approval moves you on. Later, it came
/// from something other than you, and you stay.
const TYPED_WITHIN: Duration = Duration::from_secs(1);
/// Lines moved by one mouse-wheel notch.
const WHEEL_STEP: u16 = 3;
/// A sideways scroll this soon after the last one is the same swipe, which goes one session.
const SWIPE_GAP: Duration = Duration::from_millis(200);
/// How long the leader waits for the next key before showing what it can be, as which-key does.
const LEADER_PANEL_DELAY: Duration = Duration::from_millis(200);
/// How often a slide between sessions is drawn: about 60 times a second.
const FRAME: Duration = Duration::from_millis(16);
/// How soon leader q has to come again to quit while a merge runs.
const QUIT_AGAIN: Duration = Duration::from_secs(3);
/// How often the transcript of a session in a turn is read for the title Claude gives it.
const TITLE_EVERY: Duration = Duration::from_secs(1);

pub struct App {
    pub views: Vec<View>,
    pub active: usize,
    /// A Claude started ahead of time in a directory, to be the next new session there. Not on
    /// screen until then.
    spare: Option<View>,
    /// Worktrees of closed sessions being removed, if nothing in them would be lost.
    cleanups: Vec<std::thread::JoinHandle<()>>,
    pub modal: Option<Modal>,
    /// Where scans of past sessions for the finder report back.
    scans: mpsc::UnboundedSender<Scanned>,
    ptys: ccs_term::EventSender,
    pub config: Config,
    /// When the leader was pressed; the next key is a ccs shortcut.
    pub leader_pending: Option<Instant>,
    /// The keys after the leader are listed on screen.
    pub leader_panel: bool,
    /// Where each session's chip was drawn last, for clicks.
    pub chip_areas: Vec<Rect>,
    /// Where each tab's title was drawn last (0 is Chat), for clicks.
    pub tab_areas: Vec<Rect>,
    /// The whole window, which sets the size of every terminal tab.
    window: Rect,
    /// Keys of sessions and terminals, never reused.
    next_key: u64,
    /// The socket the bridge mods report to.
    bridge: PathBuf,
    /// The theme every Claude is drawn in; Claude's own without one.
    theme: Option<ClaudeTheme>,
    /// What went wrong last, shown until the next key.
    pub notice: Option<String>,
    /// `notice` is good news: a merge went in.
    pub notice_ok: bool,
    /// The session on screen slides in over the one before it.
    slide: Option<Slide>,
    /// When the transcripts of sessions in a turn are read next.
    titles_due: Instant,
    quit: bool,
    /// When leader q was refused because a merge runs: again soon after, it quits.
    quit_refused: Option<Instant>,
    /// When the last sideways scroll came, to take a swipe's many as one.
    last_swipe: Option<Instant>,
    /// How the last Claude failed, when that is why ccs quits.
    failure: Option<color_eyre::Report>,
}

/// One Claude session on screen.
pub struct View {
    /// The key of the session's reports, and of its Claude's terminal.
    key: u64,
    /// Where Claude was started: the main checkout when it works in a worktree. New sessions
    /// there take the spare, and the finder lists its past sessions.
    pub repo: PathBuf,
    /// Where the session's files are and its tabs run: its worktree once Claude reports it,
    /// else `repo`. Follows Claude when it moves.
    pub cwd: PathBuf,
    /// The worktree ccs asked Claude to make for the session.
    worktree: Option<Worktree>,
    /// Claude is still making the session's worktree, so its tabs have nowhere to run yet.
    pub waiting: bool,
    pub session: Session,
    /// Claude, in the Chat tab.
    pub claude: Tab,
    /// When the last key went to `claude`, and what a prompt it submits does next.
    last_key: Option<(Instant, Then)>,
    /// Peer tabs, from the config, then the merge tab while there is one.
    pub tabs: Vec<Tab>,
    /// The tab that gets your keys: 0 is Chat, then `tabs[tab - 1]`.
    pub tab: usize,
    /// The two tabs side by side when the screen is split; `tab` is one of them.
    pub split: Option<Split>,
    /// The last merge of the session's work (leader m).
    merge: Option<MergeRun>,
}

/// Two tabs side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split {
    /// On the left: Chat whenever it is one of the two, else the tab that was on screen alone.
    pub main: usize,
    /// On the right: the tab put beside it, whose place a split key takes.
    pub side: usize,
}

/// A merge of a session's work: its `[[merge]]` running in a tab, and what came of it.
struct MergeRun {
    /// The `[[merge]]`, by its place in the config. None for the commit that finishes the merge of
    /// another session, whose conflicts this one fixed.
    action: Option<usize>,
    /// The key of the merge tab's terminal.
    tab: u64,
    /// The variables it ran with, which its conflict prompt is filled in from too.
    env: Vec<(&'static str, String)>,
    /// The conflicts there were before it ran.
    before: Unmerged,
    /// The branch the work goes into, checked out in the session's `repo`.
    target: String,
    /// The session fixing the conflicts it stopped on in the main checkout.
    fixer: Option<Fixer>,
    /// The session's own Claude is fixing the conflicts it stopped on in its directory, for you
    /// to review before leader m again.
    fixing: bool,
}

/// What a session is to a merge that stopped on conflicts, for where Enter takes you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Plain,
    /// Its Claude fixes the conflicts, for you to review: it goes first once it waits for you.
    Fixing,
    /// Another session fixes the conflicts of its merge: there is nothing to do in it meanwhile.
    Merged,
}

/// A session started in the main checkout to fix the conflicts of a merge there.
struct Fixer {
    key: u64,
    /// The commit merged, which the target has once the merge is done.
    tip: String,
}

impl View {
    /// Where the merge tab is in `tabs`, if there is one.
    fn merge_tab(&self) -> Option<usize> {
        let key = self.merge.as_ref()?.tab;
        self.tabs.iter().position(|t| t.key == key)
    }

    /// The tab on screen is the merge tab.
    pub fn on_merge_tab(&self) -> bool {
        self.tab
            .checked_sub(1)
            .is_some_and(|index| Some(index) == self.merge_tab())
    }

    /// Its merge tab is running.
    fn merge_running(&self) -> bool {
        self.merge_tab()
            .is_some_and(|index| !self.tabs[index].finished())
    }

    /// Its merge is under way: running, or waiting for a session to fix its conflicts.
    fn merging(&self) -> bool {
        self.merge_running() || self.merge.as_ref().is_some_and(|m| m.fixer.is_some())
    }

    /// Tab `tab`: 0 is Chat, then `tabs[tab - 1]`.
    pub fn tab_at(&self, tab: usize) -> &Tab {
        match tab.checked_sub(1) {
            Some(index) => &self.tabs[index],
            None => &self.claude,
        }
    }

    fn tab_at_mut(&mut self, tab: usize) -> &mut Tab {
        match tab.checked_sub(1) {
            Some(index) => &mut self.tabs[index],
            None => &mut self.claude,
        }
    }

    /// The terminal that gets your keys: Claude's in the Chat tab, or a peer tab's.
    pub fn terminal(&self) -> &Tab {
        self.tab_at(self.tab)
    }

    fn terminal_mut(&mut self) -> &mut Tab {
        self.tab_at_mut(self.tab)
    }

    /// The tabs on screen, from the left: the two of the split, else the one with your keys.
    pub fn shown(&self) -> Vec<usize> {
        match self.split {
            Some(split) => vec![split.main, split.side],
            None => vec![self.tab],
        }
    }

    /// The size (rows, columns) of tab `tab` in a window of `window`: its side's when the screen
    /// is split, else all of it, as when it is out of sight.
    fn size(&self, tab: usize, window: Rect) -> (u16, u16) {
        let shown = self.shown();
        match shown.iter().position(|&t| t == tab) {
            Some(pane) => ui::size(ui::pane_areas(window, shown.len())[pane]),
            None => ui::terminal_size(window),
        }
    }

    /// Sizes the terminal of every tab to where it is drawn in a window of `window`.
    fn fit(&mut self, window: Rect) {
        for tab in 0..=self.tabs.len() {
            let size = self.size(tab, window);
            if let Some(pty) = &mut self.tab_at_mut(tab).pty {
                pty.resize(size);
            }
        }
    }

    /// Gives Chat your keys: beside the tab it is split with, else on its own.
    fn focus_chat(&mut self, window: Rect) {
        self.split.take_if(|split| split.main != 0);
        self.tab = 0;
        self.fit(window);
    }

    /// The variables of the session's tabs. Each is set, empty when unknown, so that none comes
    /// from a ccs this one runs in.
    fn tab_env(&self) -> Vec<(&'static str, String)> {
        let path = |path: &Path| path.to_string_lossy().into_owned();
        vec![
            ("CS_CWD", path(&self.cwd)),
            ("CS_REPO", path(&self.repo)),
            ("CS_BRANCH", git::branch(&self.cwd).unwrap_or_default()),
            (
                "CS_REPO_BRANCH",
                git::branch(&self.repo).unwrap_or_default(),
            ),
            (
                "CS_GIT_BASE",
                self.worktree
                    .as_ref()
                    .map(|w| w.base.clone())
                    .unwrap_or_default(),
            ),
            (
                "CS_SESSION_ID",
                self.session.session_id.clone().unwrap_or_default(),
            ),
        ]
    }

    /// Runs the command of `tabs[index]`, again if it ran before, at its size in a window of
    /// `window`. Its events come with `key`.
    fn start_tab(&mut self, index: usize, key: u64, window: Rect, ptys: ccs_term::EventSender) {
        let env = self.tab_env();
        let size = self.size(index + 1, window);
        self.tabs[index].start(&self.cwd, &env, key, size, ptys);
    }

    /// Runs the regular tabs that haven't run yet, and the tabs on screen if they never ran. Their
    /// keys come from `keys`.
    fn start_tabs(&mut self, keys: &mut u64, window: Rect, ptys: &ccs_term::EventSender) {
        let shown = self.shown();
        for index in 0..self.tabs.len() {
            let tab = &self.tabs[index];
            let never_ran = tab.pty.is_none() && tab.error.is_none();
            if never_ran && (tab.def.mode == Mode::Regular || shown.contains(&(index + 1))) {
                self.start_tab(index, *keys, window, ptys.clone());
                *keys += 1;
            }
        }
    }

    /// Applies a report of the bridge mod: what the session does, and where it works. Once
    /// Claude has made the session's worktree, the tabs that waited for it start there. Whether
    /// Claude moved away from tabs that still run.
    fn bridge(
        &mut self,
        event: Bridge,
        keys: &mut u64,
        window: Rect,
        ptys: &ccs_term::EventSender,
    ) -> bool {
        let ready = matches!(event, Bridge::Ready { .. });
        let root = match &event {
            Bridge::Ready { root, .. } | Bridge::Root { root } => Some(root.clone()),
            _ => None,
        };
        // First, so the tabs get the session's id.
        self.session.bridge(event);
        let Some(root) = root else {
            return false;
        };
        if !ready {
            self.cwd = root;
            return self.tabs.iter().any(|t| t.pty.is_some() && !t.finished());
        }
        // Else it stays where it started: the launch directory, or a resumed worktree.
        if self.waiting {
            self.waiting = false;
            self.cwd = root;
            self.start_tabs(keys, window, ptys);
        }
        false
    }
}

/// What a prompt you submit, or a tool you allow, does next.
#[derive(Clone, Copy)]
enum Then {
    /// Goes to the session that needs you most, or a new one: Enter.
    Advance,
    /// Stays on the session: `submit_and_stay`, leader w.
    Stay,
    /// Opens a new session where this one started, without a worktree, even if another one
    /// waits: `submit_and_new_session`, leader Enter.
    OpenPlain,
}

/// What a key of the config does.
#[derive(Clone, Copy)]
enum Command {
    Action(Action),
    /// Shows a tab on its own: 0 is Chat.
    Tab(usize),
    /// Shows a tab beside the one on screen, or takes it off the screen.
    Split(usize),
}

/// A worktree ccs asked Claude to make: `claude --worktree <name>`.
struct Worktree {
    name: String,
    /// The commit it starts from: the one checked out in the session's `repo` when Claude started.
    base: String,
}

/// A tab: a command in a terminal.
pub struct Tab {
    pub def: TabDef,
    /// Not started yet, or failed to start.
    pub pty: Option<Pty>,
    /// The key of `pty`'s events.
    pub key: u64,
    /// How the command ended.
    pub exited: Option<String>,
    /// Why the command could not start.
    pub error: Option<String>,
}

impl Tab {
    /// Its command ran and is over, or never ran; Enter starts it again.
    pub fn finished(&self) -> bool {
        self.exited.is_some() || self.error.is_some()
    }

    /// Runs its command in `cwd` with `env` added, again if it ran before. Its events come with
    /// `key`.
    fn start(
        &mut self,
        cwd: &Path,
        env: &[(&str, String)],
        key: u64,
        size: (u16, u16),
        ptys: ccs_term::EventSender,
    ) {
        self.pty = None;
        self.exited = None;
        self.error = None;
        self.key = key;
        let env: Vec<(&str, &str)> = env
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        match Pty::spawn(&self.def.command, cwd, &env, size, key, ptys) {
            Ok(pty) => self.pty = Some(pty),
            Err(err) => self.error = Some(err.to_string()),
        }
    }
}

pub enum Modal {
    Find(Finder),
    Merge(MergePanel),
}

/// A switch between sessions, drawn as the new one pushing the old one off the screen.
struct Slide {
    /// The key of the session sliding off.
    from: u64,
    /// The new session comes in from the right: its chip is after the old one's.
    forward: bool,
    started: Instant,
    /// How long it takes: `slide_ms` for every 100 columns, so any width slides at one speed.
    duration: Duration,
}

impl App {
    pub fn new(
        config: Config,
        scans: mpsc::UnboundedSender<Scanned>,
        ptys: ccs_term::EventSender,
        bridge: PathBuf,
        theme: Option<ClaudeTheme>,
    ) -> Self {
        // Sessions open before the terminal is taken over, so ask for its size now.
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        Self {
            views: Vec::new(),
            active: 0,
            spare: None,
            cleanups: Vec::new(),
            modal: None,
            scans,
            ptys,
            config,
            leader_pending: None,
            leader_panel: false,
            chip_areas: Vec::new(),
            tab_areas: Vec::new(),
            window: Rect::new(0, 0, width, height),
            next_key: 0,
            bridge,
            theme,
            notice: None,
            notice_ok: false,
            slide: None,
            titles_due: Instant::now(),
            quit: false,
            quit_refused: None,
            last_swipe: None,
            failure: None,
        }
    }

    /// Starts a session in `repo`, or resumes `past` from there, and shows it. A new session gets
    /// a worktree only if `worktree` too. It is the spare Claude when that is in `repo`, fits
    /// `worktree`, and `repo` hasn't moved on to another commit since.
    pub fn open(
        &mut self,
        repo: &Path,
        past: Option<&PastSession>,
        worktree: bool,
    ) -> color_eyre::Result<()> {
        let repo = repo
            .canonicalize()
            .wrap_err_with(|| format!("{} is not a directory", repo.display()))?;
        let spare = match self.spare.take_if(|spare| {
            past.is_none() && spare.repo == repo && (worktree || spare.worktree.is_none())
        }) {
            // Its worktree started from a commit `repo` has moved on from since.
            Some(spare)
                if spare
                    .worktree
                    .as_ref()
                    .is_some_and(|w| git::head(&repo).as_ref() != Some(&w.base)) =>
            {
                tracing::info!(
                    key = spare.key,
                    "the spare claude started from an older commit"
                );
                self.discard(spare);
                None
            }
            spare => spare,
        };
        let view = match spare {
            Some(spare) => {
                tracing::info!(key = spare.key, "the spare claude is the new session");
                spare
            }
            None => self.spawn_view(repo.clone(), past, worktree, None)?,
        };
        let ready = view.session.state() != State::Starting;
        self.views.push(view);
        self.active = self.views.len() - 1;
        // A spare that is up already: start the next one now. Else once this one is (`on_bridge`).
        if ready {
            self.refill(&repo);
        }
        Ok(())
    }

    /// Starts a spare Claude in `repo`, unless it has one already. One elsewhere is dropped.
    fn refill(&mut self, repo: &Path) {
        if self.spare.as_ref().is_some_and(|spare| spare.repo == repo) {
            return;
        }
        match self.spawn_view(repo.to_owned(), None, true, None) {
            Ok(spare) => {
                if let Some(old) = self.spare.replace(spare) {
                    self.discard(old);
                }
            }
            // Not on screen, so not a notice.
            Err(err) => tracing::warn!("starting a spare claude failed: {err:#}"),
        }
    }

    /// Starts Claude in `repo`, resuming `past`, as a session not on screen yet. A new session
    /// gets a worktree of its own, if `worktree`, the config wants one and `repo` has a commit to
    /// start from. Claude starts on `prompt`, if given.
    fn spawn_view(
        &mut self,
        repo: PathBuf,
        past: Option<&PastSession>,
        worktree: bool,
        prompt: Option<&str>,
    ) -> color_eyre::Result<View> {
        let mut session = Session::default();
        // Known before Claude reports it, so the finder doesn't offer it again.
        if let Some(past) = past {
            session.session_id = Some(past.id.clone());
            session.name.clone_from(&past.name);
            // Not where its directory says: Claude moves the transcript out of a worktree it
            // removes.
            session.ai_title = history::ai_title(&past.path);
        }
        let worktree = if worktree && self.config.worktree && past.is_none() {
            git::head(&repo).map(|base| Worktree {
                name: format!("ccs-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                base,
            })
        } else {
            None
        };
        if worktree.is_some()
            && let Err(err) = git::exclude_worktrees(&repo)
        {
            tracing::warn!("{err}");
        }
        let key = self.next_key;
        let claude = self.spawn_claude(
            key,
            &repo,
            past.map(|p| p.id.as_str()),
            worktree.as_ref().map(|w| w.name.as_str()),
            prompt,
        )?;
        self.next_key += 1;
        // A resumed session goes back to where it was, if that is still there: Claude goes back
        // into its worktree itself.
        let cwd = past
            .and_then(|past| past.cwd.clone())
            .filter(|cwd| cwd.is_dir())
            .unwrap_or_else(|| repo.clone());
        let mut view = View {
            key,
            repo,
            cwd,
            waiting: worktree.is_some(),
            worktree,
            session,
            claude,
            last_key: None,
            tabs: Vec::new(),
            tab: 0,
            split: None,
            merge: None,
        };
        self.peer_tabs(&mut view);
        Ok(view)
    }

    /// Gives `view` the peer tabs of the config. Regular ones start now, unless Claude is still
    /// making the session's worktree; the others when opened.
    fn peer_tabs(&mut self, view: &mut View) {
        view.tabs = self
            .config
            .tabs
            .iter()
            .map(|def| Tab {
                def: def.clone(),
                pty: None,
                key: 0,
                exited: None,
                error: None,
            })
            .collect();
        if !view.waiting {
            view.start_tabs(&mut self.next_key, self.window, &self.ptys);
        }
    }

    /// Runs the command of `views[view].tabs[index]`, again if it ran before. Not while Claude
    /// is still making the worktree it runs in, and never the merge tab: leader m runs a merge.
    fn start_tab(&mut self, view: usize, index: usize) {
        if self.views[view].waiting || self.views[view].merge_tab() == Some(index) {
            return;
        }
        let key = self.next_key;
        self.next_key += 1;
        self.views[view].start_tab(index, key, self.window, self.ptys.clone());
    }

    /// Shows tab `tab` of the session on screen on its own (0 is Chat), with your keys.
    fn select_tab(&mut self, tab: usize) {
        let view = &mut self.views[self.active];
        if tab > view.tabs.len() {
            return;
        }
        let before = view.shown();
        view.tab = tab;
        view.split = None;
        self.came_on_screen(&before);
    }

    /// Shows tab `tab` of the session on screen beside the one there, with your keys. With two on
    /// screen, it takes the place of the one on the right, or goes if it is one of them. On screen
    /// alone, Chat comes beside it instead. Chat is always on the left.
    fn split_tab(&mut self, tab: usize) {
        let view = &mut self.views[self.active];
        if tab > view.tabs.len() {
            return;
        }
        let before = view.shown();
        // The tab that stays, and the one that comes and gets your keys.
        let (stays, comes) = match view.split {
            // It goes, and the other one has the screen.
            Some(split) if before.contains(&tab) => {
                view.tab = if tab == split.main {
                    split.side
                } else {
                    split.main
                };
                view.split = None;
                return self.came_on_screen(&before);
            }
            Some(split) => (split.main, tab),
            None if tab != view.tab => (view.tab, tab),
            None if tab != 0 => (tab, 0),
            // Chat beside itself.
            None => return,
        };
        view.tab = comes;
        view.split = Some(if comes == 0 {
            Split {
                main: comes,
                side: stays,
            }
        } else {
            Split {
                main: stays,
                side: comes,
            }
        });
        self.came_on_screen(&before);
    }

    /// Gives your keys to the other tab on screen, when there are two.
    fn other_pane(&mut self) {
        let view = &mut self.views[self.active];
        if let Some(split) = view.split {
            view.tab = if view.tab == split.main {
                split.side
            } else {
                split.main
            };
        }
    }

    /// The tabs of the session on screen have changed from `before` (`View::shown`). Runs those on
    /// screen the first time, and every time one comes on screen if it is eager. Sizes each to
    /// where it is now.
    fn came_on_screen(&mut self, before: &[usize]) {
        for tab in self.views[self.active].shown() {
            let Some(index) = tab.checked_sub(1) else {
                continue;
            };
            let peer = &self.views[self.active].tabs[index];
            let never_ran = peer.pty.is_none() && peer.error.is_none();
            if never_ran || (!before.contains(&tab) && peer.def.mode == Mode::Eager) {
                self.start_tab(self.active, index);
            }
        }
        self.views[self.active].fit(self.window);
    }

    /// Resumes a past session by id, from the directory it ran in.
    pub fn resume(&mut self, id: &str) -> color_eyre::Result<()> {
        let past = history::find_session(&history::projects_dir()?, id)?
            .ok_or_eyre(format!("no Claude Code session with id {id}"))?;
        let cwd = past
            .cwd
            .clone()
            .ok_or_eyre("the session has no directory")?;
        self.open(resume_from(&cwd), Some(&past), true)
    }

    /// Starts a new session where the one on screen started, in a worktree of its own only if
    /// `worktree`.
    fn open_new(&mut self, worktree: bool) {
        let repo = self.views[self.active].repo.clone();
        if let Err(err) = self.open(&repo, None, worktree) {
            self.notify(format!("starting a session failed: {err:#}"));
        }
    }

    /// Starts Claude in `cwd`, resuming `resume`, or in a new worktree `worktree`, with the
    /// bridge mod reporting as session `key`, on `prompt` if given. Its terminal's events come
    /// with `key` too. A new session without a prompt starts in plan mode, if the config wants it.
    fn spawn_claude(
        &self,
        key: u64,
        cwd: &Path,
        resume: Option<&str>,
        worktree: Option<&str>,
        prompt: Option<&str>,
    ) -> color_eyre::Result<Tab> {
        let mut args = vec![CLAUDE.to_owned(), "--plugin-dir".into(), quoted(MODS)];
        let mut settings = serde_json::Map::new();
        if let Some(theme) = &self.theme {
            let dir = theme
                .dir()
                .to_str()
                .ok_or_eyre("the theme path is not UTF-8")?;
            args.extend(["--plugin-dir".into(), quoted(dir)]);
            settings.insert("theme".into(), theme.setting().into());
        }
        if let Some(name) = worktree {
            args.extend(["--worktree".into(), quoted(name)]);
            // From the commit checked out in `cwd`, the files you have, not the remote's.
            settings.insert("worktree".into(), serde_json::json!({ "baseRef": "head" }));
        }
        if !settings.is_empty() {
            let settings = serde_json::Value::Object(settings).to_string();
            args.extend(["--settings".into(), quoted(&settings)]);
        }
        // A new session starts by planning. Not a resumed one, nor one Claude starts on a prompt:
        // that is a merge's conflicts, for it to fix on its own.
        if self.config.plan && resume.is_none() && prompt.is_none() {
            args.extend(["--permission-mode".into(), "plan".into()]);
        }
        if let Some(id) = resume {
            args.extend(["--resume".into(), quoted(id)]);
        }
        if let Some(prompt) = prompt {
            args.push(quoted(prompt));
        }
        let command = args.join(" ");
        let exe = std::env::current_exe().wrap_err("finding the ccs executable")?;
        let session = key.to_string();
        let env = [
            (
                "CCS_EXE",
                exe.to_str().ok_or_eyre("the ccs path is not UTF-8")?,
            ),
            (
                "CCS_SOCKET",
                self.bridge
                    .to_str()
                    .ok_or_eyre("the socket path is not UTF-8")?,
            ),
            ("CCS_SESSION", session.as_str()),
        ];
        let size = ui::terminal_size(self.window);
        let pty = Pty::spawn(&command, cwd, &env, size, key, self.ptys.clone())?;
        Ok(Tab {
            // Shown in the tab bar like a peer tab's. The command run is `command`.
            def: TabDef {
                name: "Chat".to_owned(),
                key: self.config.keys[&Action::Chat].clone(),
                split_key: Bindings::default(),
                mode: Mode::Lazy,
                command: CLAUDE.to_owned(),
            },
            pty: Some(pty),
            key,
            exited: None,
            error: None,
        })
    }

    /// Shows `text` until the next key, and logs it.
    fn notify(&mut self, text: String) {
        tracing::warn!("{text}");
        self.notice = Some(text);
        self.notice_ok = false;
    }

    /// Shows good news `text` until the next key, and logs it.
    fn inform(&mut self, text: String) {
        tracing::info!("{text}");
        self.notice = Some(text);
        self.notice_ok = true;
    }

    /// Where the session whose merge session `key` fixes the conflicts is in `views`.
    fn fixed_by(&self, key: u64) -> Option<usize> {
        self.views.iter().position(|v| {
            v.merge
                .as_ref()
                .and_then(|m| m.fixer.as_ref())
                .is_some_and(|f| f.key == key)
        })
    }

    /// The branch the merge that session `index` fixes goes into, while it is a merge session.
    pub fn merge_session_target(&self, index: usize) -> Option<&str> {
        let merged = self.fixed_by(self.views[index].key)?;
        self.views[merged].merge.as_ref().map(|m| m.target.as_str())
    }

    /// What `view` is to a merge that stopped on conflicts.
    fn role(&self, view: &View) -> Role {
        if self.fixed_by(view.key).is_some() {
            return Role::Fixing;
        }
        match &view.merge {
            Some(run) if run.fixer.is_some() => Role::Merged,
            Some(run) if run.fixing => Role::Fixing,
            _ => Role::Plain,
        }
    }

    /// Goes to the session that needs you most, or opens a new one if none is waiting.
    fn advance(&mut self) {
        let sessions: Vec<(State, Role)> = self
            .views
            .iter()
            .map(|v| (v.session.state(), self.role(v)))
            .collect();
        match next_waiting(&sessions, self.active) {
            // You go there to answer it, so to its Chat tab.
            Some(index) => {
                self.active = index;
                self.views[index].focus_chat(self.window);
            }
            None => self.open_new(true),
        }
    }

    /// Slides the session on screen in over `shown`, the key of the one on screen before, if
    /// that is another session and still open. A closed one, or none, gives way at once.
    fn slide_from(&mut self, shown: u64) {
        let duration =
            Duration::from_millis(self.config.slide_ms * u64::from(self.window.width) / 100);
        if duration.is_zero() || self.views.get(self.active).is_none_or(|v| v.key == shown) {
            return;
        }
        if let Some(from) = self.views.iter().position(|v| v.key == shown) {
            self.slide = Some(Slide {
                from: shown,
                forward: self.active > from,
                started: Instant::now(),
                duration,
            });
        }
    }

    /// While a session slides in: the one sliding off, how many of `width` columns the new one
    /// has come in, and whether it comes in from the right.
    pub fn slide(&self, width: u16) -> Option<(&View, u16, bool)> {
        let slide = self.slide.as_ref()?;
        let from = self.views.iter().find(|v| v.key == slide.from)?;
        let total = slide.duration.as_millis();
        let left = total.saturating_sub(slide.started.elapsed().as_millis());
        // Fast at first, slowing as it lands: what is still to come goes with the time left, cubed.
        let to_come = u128::from(width) * left.pow(3) / total.pow(3);
        let moved = width - u16::try_from(to_come).unwrap_or(width);
        Some((from, moved, slide.forward))
    }

    pub async fn run(
        mut self,
        terminal: &mut DefaultTerminal,
        mut scans: mpsc::UnboundedReceiver<Scanned>,
        mut pty_events: mpsc::UnboundedReceiver<(u64, PtyEvent)>,
        mut bridge_reports: mpsc::UnboundedReceiver<(u64, Bridge)>,
    ) -> color_eyre::Result<()> {
        let mut terminal_events = EventStream::new();
        while !self.quit {
            // Over: the session on screen is all there.
            self.slide.take_if(|s| s.started.elapsed() >= s.duration);
            terminal.draw(|frame| ui::render(frame, &mut self))?;
            let shown = self.views[self.active].key;
            tokio::select! {
                event = terminal_events.next() => match event {
                    Some(event) => self.on_terminal(event?),
                    None => break,
                },
                Some((cwd, result)) = scans.recv() => self.on_scan(&cwd, result),
                Some((key, event)) = pty_events.recv() => self.on_pty(key, event),
                Some((key, event)) = bridge_reports.recv() => self.on_bridge(key, event),
                // The leader has waited long enough for the next key: list them.
                () = tokio::time::sleep_until(
                    self.leader_pending.unwrap_or_else(Instant::now) + LEADER_PANEL_DELAY
                ), if self.leader_pending.is_some() && !self.leader_panel => {
                    self.leader_panel = true;
                }
                // Claude's title for a session can come long before its turn ends.
                () = tokio::time::sleep_until(self.titles_due),
                    if self.views.iter().any(|v| in_turn(&v.session)) => {
                    self.titles_due = Instant::now() + TITLE_EVERY;
                    for view in self.views.iter_mut().filter(|v| in_turn(&v.session)) {
                        refresh_ai_title(&mut view.session, &view.cwd);
                    }
                }
                // The next frame of the slide. Output from a program can keep this from firing,
                // but it redraws then anyway.
                () = tokio::time::sleep(FRAME), if self.slide.is_some() => {}
            }
            self.slide_from(shown);
        }
        // Dropping the terminals ends their programs, Claude's too.
        for view in std::mem::take(&mut self.views)
            .into_iter()
            .chain(self.spare.take())
        {
            self.discard(view);
        }
        for cleanup in self.cleanups.drain(..) {
            let _ = cleanup.join();
        }
        match self.failure {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    fn on_pty(&mut self, key: u64, event: PtyEvent) {
        match event {
            // Drawn on the way round the loop.
            PtyEvent::Output => {}
            PtyEvent::Exited(status) => {
                // Not started again here: a Claude that can't start would start over and over.
                // The next session to be ready starts one.
                if let Some(spare) = self.spare.take_if(|s| s.claude.key == key) {
                    tracing::warn!(key, status, "the spare claude exited");
                    self.discard(spare);
                    return;
                }
                if let Some(index) = self.views.iter().position(|v| v.claude.key == key) {
                    if self.views[index].waiting {
                        self.without_worktree(index);
                    } else {
                        self.close(index, &status);
                    }
                    return;
                }
                // The spare's too, as its regular tabs run. Not found when the tab was started
                // again since.
                if let Some(tab) = self
                    .views
                    .iter_mut()
                    .chain(self.spare.as_mut())
                    .flat_map(|v| &mut v.tabs)
                    .find(|t| t.key == key && t.pty.is_some())
                {
                    tab.exited = Some(status.clone());
                    self.merge_ended(key, &status);
                }
            }
        }
    }

    /// Starts session `index` again without a worktree, as its Claude exited before it made one:
    /// it does that only in a directory you trust, and asks for that only without one.
    fn without_worktree(&mut self, index: usize) {
        let view = &mut self.views[index];
        let screen = view
            .claude
            .pty
            .as_ref()
            .map(Pty::contents)
            .unwrap_or_default();
        let why = screen
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .to_owned();
        let window = self.window;
        let worktree = view.worktree.take();
        view.waiting = false;
        let key = view.key;
        let repo = view.repo.clone();
        // In case it got as far as making it.
        if let Some(worktree) = worktree {
            self.remove_worktree(repo.clone(), worktree.name);
        }
        match self.spawn_claude(key, &repo, None, None, None) {
            Ok(claude) => {
                let view = &mut self.views[index];
                view.claude = claude;
                // Started at the size of the whole window, but Chat may be beside a tab.
                view.fit(window);
                view.start_tabs(&mut self.next_key, window, &self.ptys);
                self.notify(format!("no worktree for this session: {why}"));
            }
            Err(err) => self.close(index, &format!("starting it again failed: {err:#}")),
        }
    }

    /// Removes the worktree of a session that is over, on a thread, if ccs asked for it and
    /// nothing in it would be lost. A session named with `/rename` keeps it, as Claude does.
    /// Dropping `view` ends the programs in its terminals.
    fn discard(&mut self, view: View) {
        let View {
            repo,
            worktree,
            session,
            ..
        } = view;
        if let Some(worktree) = worktree.filter(|_| session.name.is_none()) {
            self.remove_worktree(repo, worktree.name);
        }
    }

    /// Removes the worktree `name` of `repo` on a thread, if nothing in it would be lost.
    fn remove_worktree(&mut self, repo: PathBuf, name: String) {
        self.cleanups.push(std::thread::spawn(move || {
            match git::remove_if_unused(&repo, &name) {
                Ok(true) => tracing::info!(name, "removed the unused worktree"),
                Ok(false) => tracing::info!(name, "the worktree holds work, or is gone"),
                Err(err) => tracing::warn!(name, "removing the worktree failed: {err}"),
            }
        }));
    }

    /// Closes session `index`, whose Claude exited with `status`, and its tabs with it. Quits
    /// after the last one.
    fn close(&mut self, index: usize, status: &str) {
        // Dropped on the way out, which ends the programs in its tabs.
        let view = self.views.remove(index);
        let failed = status != "exit 0";
        if self.views.is_empty() {
            // `run` stops before drawing again, and drawing needs a session.
            self.quit = true;
            if failed {
                let screen = view
                    .claude
                    .pty
                    .as_ref()
                    .map(|pty| pty.contents().trim().to_owned())
                    .unwrap_or_default();
                let failure = eyre!("claude exited ({status})");
                self.failure = Some(if screen.is_empty() {
                    failure
                } else {
                    failure.section(screen.header("Claude's screen:"))
                });
            }
            self.discard(view);
            return;
        }
        self.active = active_after_close(self.active, index, self.views.len());
        // The finder lists open sessions by their place, and those after it moved up.
        if let Some(Modal::Find(finder)) = &mut self.modal {
            finder.relist(&self.views);
        }
        if failed {
            self.notify(format!(
                "{}: claude exited ({status})",
                ui::session_name(&view)
            ));
        }
        let key = view.key;
        // After /exit Claude has removed its worktree, or asked; this is for a Claude that crashed.
        self.discard(view);
        self.check_fixer(key, true);
    }

    /// A report of the bridge mod in session `key`'s Claude.
    fn on_bridge(&mut self, key: u64, event: Bridge) {
        let window = self.window;
        // Kept, so the spare shows up ready with its id, its tabs in its worktree.
        if let Some(spare) = self.spare.as_mut().filter(|s| s.key == key) {
            spare.bridge(event, &mut self.next_key, window, &self.ptys);
            return;
        }
        let Some(index) = self.views.iter().position(|v| v.key == key) else {
            return;
        };
        // You sent a prompt, or allowed a tool: Claude carries on without you.
        let moves_on = matches!(event, Bridge::Submitted | Bridge::Approved);
        let ready = matches!(event, Bridge::Ready { .. });
        let idle = matches!(event, Bridge::Idle);
        let view = &mut self.views[index];
        if view.bridge(event, &mut self.next_key, window, &self.ptys) {
            let cwd = view.cwd.display().to_string();
            let rerun = ui::key_hint(&self.config, Action::RerunTab);
            self.notify(format!(
                "claude moved to {cwd}; tabs still running stay put, {rerun} runs one there"
            ));
            return;
        }
        if idle {
            refresh_ai_title(&mut view.session, &view.cwd);
        }
        // Only now, so the spare doesn't slow down the session you are waiting for.
        if ready {
            let repo = view.repo.clone();
            self.refill(&repo);
            // Being ready never moves you on.
            return;
        }
        // Only right after your own key there: never switch away from a session in the background.
        let typed = view.last_key.filter(|(at, _)| at.elapsed() < TYPED_WITHIN);
        if moves_on
            && let Some((_, then)) = typed
            && index == self.active
            && view.tab == 0
            && self.modal.is_none()
        {
            view.last_key = None;
            match then {
                Then::Advance => self.advance(),
                Then::Stay => {}
                Then::OpenPlain => self.open_new(false),
            }
        }
        if idle {
            self.check_fixer(key, false);
        }
    }

    fn on_scan(&mut self, cwd: &Path, result: Result<Vec<OldSession>, String>) {
        // The finder was closed, or opened again elsewhere, since this scan started.
        let Some(Modal::Find(finder)) = &mut self.modal else {
            return;
        };
        if finder.cwd != cwd {
            return;
        }
        match result {
            Ok(old) => finder.scanned(old, &self.views),
            Err(err) => {
                finder.scanned(Vec::new(), &self.views);
                self.notify(format!("listing past sessions failed: {err}"));
            }
        }
    }

    fn on_terminal(&mut self, event: Event) {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.notice = None;
                self.on_key(key);
            }
            Event::Paste(text) => {
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                match &mut self.modal {
                    Some(Modal::Find(finder)) => {
                        finder.query.insert(&text.replace('\n', " "));
                        finder.refresh();
                    }
                    Some(Modal::Merge(panel)) => {
                        if panel.commit_message().is_some() {
                            panel.message.insert(&text);
                        }
                    }
                    None => {
                        if let Some(pty) = &mut self.views[self.active].terminal_mut().pty {
                            pty.paste(&text);
                        }
                    }
                }
            }
            // The popups cover the screen, so clicks and the wheel leave it alone.
            Event::Mouse(mouse) if self.modal.is_none() => {
                let at = Position::new(mouse.column, mouse.row);
                // A click on a session's chip or a tab's title goes there, as the leader keys do.
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    if let Some(index) = self.chip_areas.iter().position(|a| a.contains(at)) {
                        self.active = index;
                        return;
                    }
                    if let Some(tab) = self.tab_areas.iter().position(|a| a.contains(at)) {
                        self.select_tab(tab);
                        return;
                    }
                }
                // Scrolling sideways, as a touchpad swipe does, goes to the session beside this one.
                // So does Shift+scroll, as Ghostty on Linux keeps sideways scrolling for its tabs.
                let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
                let next = match mouse.kind {
                    MouseEventKind::ScrollRight => Some(true),
                    MouseEventKind::ScrollDown if shift => Some(true),
                    MouseEventKind::ScrollLeft => Some(false),
                    MouseEventKind::ScrollUp if shift => Some(false),
                    _ => None,
                };
                if let Some(next) = next {
                    let same_swipe = self.last_swipe.is_some_and(|t| t.elapsed() < SWIPE_GAP);
                    self.last_swipe = Some(Instant::now());
                    if !same_swipe {
                        let last = self.views.len() - 1;
                        self.active = if next {
                            (self.active + 1).min(last)
                        } else {
                            self.active.saturating_sub(1)
                        };
                    }
                    return;
                }
                let view = &mut self.views[self.active];
                let shown = view.shown();
                let panes = ui::pane_areas(self.window, shown.len());
                // The tab under the pointer. Off the tabs, the wheel scrolls the one with your keys.
                let pane = panes.iter().position(|a| a.contains(at));
                let tab = pane.map_or(view.tab, |pane| shown[pane]);
                let click = matches!(mouse.kind, MouseEventKind::Down(_));
                // A click there gives it your keys too.
                if pane.is_some() && click {
                    view.tab = tab;
                }
                let Some(pty) = &mut view.tab_at_mut(tab).pty else {
                    return;
                };
                // To the program if it asked for the mouse, else the wheel scrolls back.
                if let Some(pane) = pane {
                    let area = panes[pane];
                    let at_screen = (at.x - area.x, at.y - area.y);
                    if pty.mouse(mouse.kind, mouse.modifiers, at_screen) {
                        // A click can answer Claude's permission dialog.
                        if tab == 0 && click {
                            view.last_key = Some((Instant::now(), Then::Advance));
                        }
                        return;
                    }
                }
                match mouse.kind {
                    MouseEventKind::ScrollUp => pty.scroll_up(WHEEL_STEP.into()),
                    MouseEventKind::ScrollDown => pty.scroll_down(WHEEL_STEP.into()),
                    _ => {}
                }
            }
            // Hidden tabs and the spare too, so a program never draws for the wrong size.
            Event::Resize(width, height) => {
                self.window = Rect::new(0, 0, width, height);
                for view in self.views.iter_mut().chain(self.spare.as_mut()) {
                    view.fit(self.window);
                }
            }
            _ => {}
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if self.modal.is_some() {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                self.quit = true;
            } else {
                self.on_modal_key(key);
            }
            return;
        }
        if self.leader_pending.take().is_some() {
            self.leader_panel = false;
            self.on_leader_key(key);
            return;
        }
        if keys::matches(self.config.leader, &key) {
            self.leader_pending = Some(Instant::now());
            return;
        }
        // A key of ccs's own, without the leader: it doesn't reach the terminal.
        if let Some(command) = self.command_for(&key, false) {
            self.run_command(command);
            return;
        }
        self.key_to_terminal(key);
    }

    /// Gives `key` to the terminal on screen: every key but the leader and ccs's own, Ctrl+C
    /// included.
    fn key_to_terminal(&mut self, key: KeyEvent) {
        let view = &mut self.views[self.active];
        if let Some(index) = view.tab.checked_sub(1) {
            let tab = &mut view.tabs[index];
            if tab.finished() {
                if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                    self.start_tab(self.active, index);
                }
            } else if let Some(pty) = &mut tab.pty {
                pty.key(key);
            }
            return;
        }
        if let Some(pty) = &mut view.claude.pty {
            pty.key(key);
            view.last_key = Some((Instant::now(), Then::Advance));
        }
    }

    /// Presses Enter in the Claude on screen, so that the prompt it submits, or the tool it
    /// allows, does `then` next.
    fn submit(&mut self, then: Then) {
        let view = &mut self.views[self.active];
        if let Some(pty) = &mut view.claude.pty {
            pty.key(KeyEvent::from(KeyCode::Enter));
            view.last_key = Some((Instant::now(), then));
        }
    }

    /// Opens the finder, and scans the past sessions of the project on screen for it.
    fn open_finder(&mut self) {
        let cwd = self.views[self.active].repo.clone();
        self.modal = Some(Modal::Find(Finder::new(cwd.clone(), &self.views)));
        let scans = self.scans.clone();
        tokio::task::spawn_blocking(move || {
            let result = finder::scan(&cwd);
            let _ = scans.send((cwd, result));
        });
    }

    /// Quits, unless a merge is running: killing it could leave the merge half done. Then it
    /// takes leader q again, soon after.
    fn request_quit(&mut self) {
        let again = self
            .quit_refused
            .is_some_and(|at| at.elapsed() < QUIT_AGAIN);
        match self.views.iter().find(|v| v.merge_running()) {
            Some(view) if !again => {
                let name = ui::session_name(view);
                let quit = ui::key_hint(&self.config, Action::Quit);
                self.quit_refused = Some(Instant::now());
                self.notify(format!("{name} is merging: {quit} again to quit anyway"));
            }
            _ => self.quit = true,
        }
    }

    /// Opens the merge panel for the session on screen (leader m), or says why there is nothing
    /// it could do now.
    fn open_merge(&mut self) {
        match self.merge_panel() {
            Ok(panel) => self.modal = Some(Modal::Merge(panel)),
            Err(why) => self.notify(why),
        }
    }

    /// What leader m would take from the session on screen, or why it can't now.
    fn merge_panel(&self) -> Result<MergePanel, String> {
        if self.config.merges.is_empty() {
            return Err("no [[merge]] action in the config".into());
        }
        let view = &self.views[self.active];
        match view.session.state() {
            State::Idle => {}
            State::Starting => return Err("Claude is still starting: merge once it's ready".into()),
            State::Working | State::WaitingPermission => {
                return Err("Claude is working: merge when it's done".into());
            }
            // The merge closes the session, which would stop its background work.
            State::Asleep => {
                return Err(
                    "Claude is waiting on background work or a wakeup: stop it before merging"
                        .into(),
                );
            }
        }
        // A merge session only finishes the merge it fixed, which is the one under way there.
        if let Some(merged) = self.fixed_by(view.key) {
            return finish_panel(view, &self.views[merged]);
        }
        // They would get in each other's way in the main checkout.
        if let Some(other) = self
            .views
            .iter()
            .find(|v| v.repo == view.repo && v.merging())
        {
            return Err(format!(
                "{} is merging there already: one merge at a time",
                ui::session_name(other)
            ));
        }
        let branch = git::branch(&view.cwd).ok_or("this session is on no branch")?;
        let target = git::branch(&view.repo);
        let changes = git::changes(&view.cwd)?;
        let ahead = match &target {
            Some(target) => git::ahead(&view.cwd, target)?,
            None => 0,
        };
        // Doing nothing would still close the session.
        if changes.is_empty()
            && let Some(target) = &target
        {
            if *target == branch {
                return Err(format!("nothing to commit on {branch}"));
            }
            if ahead == 0 {
                return Err(format!("nothing to merge: {branch} is in {target} already"));
            }
        }
        let plain = target.as_ref() == Some(&branch);
        let others = self
            .views
            .iter()
            .filter(|v| !plain && v.key != view.key && v.cwd == view.repo)
            .count();
        let mut message = Input::default();
        let title = view
            .session
            .name
            .as_ref()
            .or(view.session.ai_title.as_ref());
        if !changes.is_empty()
            && let Some(title) = title
        {
            message.insert(title);
        }
        Ok(MergePanel {
            view: view.key,
            branch,
            target,
            changes,
            ahead,
            others,
            message,
            action: 0,
            needs_message: false,
            finish: false,
        })
    }

    /// Runs the `[[merge]]` picked in `panel` in a merge tab of its session, and shows the tab. In
    /// a merge session, the commit that finishes the merge instead.
    fn run_merge(&mut self, panel: MergePanel) {
        // Closed since.
        let Some(index) = self.views.iter().position(|v| v.key == panel.view) else {
            return;
        };
        let message = panel.commit_message().map(str::to_owned);
        let (name, command, action) = if panel.finish {
            ("finish merge".to_owned(), merge::FINISH.to_owned(), None)
        } else {
            let def = &self.config.merges[panel.action];
            let command = merge::script(def, message.is_some());
            (def.name.clone(), command, Some(panel.action))
        };
        let tab_def = TabDef {
            name,
            key: Bindings::default(),
            split_key: Bindings::default(),
            mode: Mode::Lazy,
            command,
        };
        let key = self.next_key;
        self.next_key += 1;
        let size = ui::terminal_size(self.window);
        let view = &mut self.views[index];
        // The one before is over: leader m runs only then.
        if let Some(old) = view.merge_tab() {
            view.tabs.remove(old);
        }
        let before = Unmerged {
            cwd: git::unmerged(&view.cwd),
            repo: git::unmerged(&view.repo),
        };
        let mut env = view.tab_env();
        env.push(("CS_MESSAGE", message.unwrap_or_default()));
        let mut tab = Tab {
            def: tab_def,
            pty: None,
            key,
            exited: None,
            error: None,
        };
        tab.start(&view.cwd, &env, key, size, self.ptys.clone());
        view.tabs.push(tab);
        // On its own.
        view.tab = view.tabs.len();
        view.split = None;
        view.fit(self.window);
        view.merge = Some(MergeRun {
            action,
            tab: key,
            env,
            before,
            target: panel.target.unwrap_or_default(),
            fixer: None,
            fixing: false,
        });
    }

    /// The merge tab with terminal `key` exited with `status`. Closes its session if the work is
    /// in, or hands the conflicts it stopped on to the Claude that can work where they are.
    fn merge_ended(&mut self, key: u64, status: &str) {
        let Some(index) = self
            .views
            .iter()
            .position(|v| v.merge.as_ref().is_some_and(|m| m.tab == key))
        else {
            return;
        };
        let view = &self.views[index];
        let Some(run) = &view.merge else {
            return;
        };
        // The commit that finishes the merge whose conflicts this session fixed: once it is in,
        // both sessions close.
        let Some(action) = run.action else {
            let key = view.key;
            if status != "exit 0" {
                self.notify(format!(
                    "finishing the merge failed ({status}): see its tab"
                ));
            }
            return self.check_fixer(key, false);
        };
        let after = Unmerged {
            cwd: git::unmerged(&view.cwd),
            repo: git::unmerged(&view.repo),
        };
        let def = &self.config.merges[action];
        let name = def.name.clone();
        let prompt = def.on_conflict.as_ref().map(|p| merge::expand(p, &run.env));
        match (merge::outcome(status, run.before, after), prompt) {
            (Outcome::Merged, _) => self.merged(index),
            (Outcome::Conflict(Place::Cwd), Some(prompt)) => self.prompt_claude(index, &prompt),
            (Outcome::Conflict(Place::Repo), Some(prompt)) => self.open_fixer(index, &prompt),
            (Outcome::Conflict(place), None) => {
                let dir = match place {
                    Place::Cwd => &view.cwd,
                    Place::Repo => &view.repo,
                };
                let dir = dir.display();
                let merge = ui::key_hint(&self.config, Action::Merge);
                self.notify(format!(
                    "{name} stopped on conflicts in {dir}: fix them, then {merge} again"
                ));
            }
            (Outcome::Failed, _) => self.notify(format!("{name} failed ({status}): see its tab")),
        }
    }

    /// The work of session `index` is in its target. The session closes, and its worktree goes
    /// if nothing in it would be lost, even if the session is named. If you were on it, you go on
    /// as after a prompt you submit, to a new session if none waits. Not while Claude is busy,
    /// which closing would stop.
    fn merged(&mut self, index: usize) {
        let view = &mut self.views[index];
        let name = ui::session_name(view);
        let target = match &mut view.merge {
            Some(run) => {
                run.fixer = None;
                run.target.clone()
            }
            None => String::new(),
        };
        if view.session.state() != State::Idle {
            self.inform(format!(
                "merged {name} into {target}; Claude is busy, so the session stays open"
            ));
            return;
        }
        let view = self.drop_view(index);
        let repo = view.repo.clone();
        let worktree = match &view.worktree {
            Some(worktree) => Some(worktree.name.clone()),
            // Resumed, or moved there.
            None => worktree_name(&view.cwd),
        };
        // Ends its programs, Claude's too, before its worktree goes.
        drop(view);
        if let Some(worktree) = worktree {
            self.remove_worktree(repo.clone(), worktree);
        }
        if self.views.is_empty()
            && let Err(err) = self.open(&repo, None, true)
        {
            // `run` stops before drawing again, and drawing needs a session.
            self.quit = true;
            self.failure = Some(err.wrap_err(format!("merged {name}, then opening a session")));
            return;
        }
        self.inform(format!("merged {name} into {target}"));
    }

    /// Takes session `index` away, as a merge closed it. If it was on screen, you go on as after a
    /// prompt you submit: to the session that needs you most, or a new one. Dropping it ends its
    /// programs.
    fn drop_view(&mut self, index: usize) -> View {
        // While it is still there: the session after it is one to go to.
        if self.active == index {
            self.advance();
        }
        let view = self.views.remove(index);
        if !self.views.is_empty() {
            self.active = active_after_close(self.active, index, self.views.len());
        }
        // The finder lists open sessions by their place, and those after it moved up.
        if let Some(Modal::Find(finder)) = &mut self.modal {
            finder.relist(&self.views);
        }
        view
    }

    /// Starts a merge session in the main checkout of session `index`, without a worktree, on
    /// `prompt`, to fix the conflicts its merge stopped on there, for you to review and finish.
    /// Both are busy until then: if you are on `index`, you go on to a session that needs you.
    fn open_fixer(&mut self, index: usize, prompt: &str) {
        let repo = self.views[index].repo.clone();
        let tip = git::head(&self.views[index].cwd).unwrap_or_default();
        match self.spawn_view(repo.clone(), None, false, Some(prompt)) {
            Ok(fixer) => {
                if let Some(run) = &mut self.views[index].merge {
                    run.fixer = Some(Fixer {
                        key: fixer.key,
                        tip,
                    });
                }
                let name = ui::session_name(&self.views[index]);
                self.views.push(fixer);
                let fixer = self.views.len();
                self.notify(format!(
                    "merging {name} stopped on conflicts: session {fixer} is fixing them"
                ));
                if self.active == index {
                    self.advance();
                }
            }
            Err(err) => self.notify(format!(
                "starting a session to fix the conflicts failed: {err:#}; fix them in {}, or \
                 git merge --abort",
                repo.display()
            )),
        }
    }

    /// Types `text`, the prompt to fix the conflicts its merge stopped on in its directory, into
    /// the Claude of session `index`, and submits it. It is busy until it waits for you to review
    /// them: if you are on it, you go on to a session that needs you.
    fn prompt_claude(&mut self, index: usize, text: &str) {
        let view = &mut self.views[index];
        let Some(pty) = &mut view.claude.pty else {
            return;
        };
        pty.paste(text);
        pty.key(KeyEvent::from(KeyCode::Enter));
        // So its `submitted` report doesn't move you on again.
        view.last_key = Some((Instant::now(), Then::Stay));
        if let Some(run) = &mut view.merge {
            run.fixing = true;
        }
        let name = ui::session_name(view);
        self.notify(format!(
            "merging {name} stopped on conflicts: its Claude is fixing them"
        ));
        if self.active == index {
            self.advance();
        }
    }

    /// Session `key`, which may be fixing the conflicts of a merge in its main checkout, went
    /// idle, or `closed`. Once the merge is in, it and the session merged close. A merge that was
    /// aborted, or is unfinished as the session closes, is let go, and you are told.
    fn check_fixer(&mut self, key: u64, closed: bool) {
        let Some(index) = self.fixed_by(key) else {
            return;
        };
        let view = &mut self.views[index];
        let Some(run) = &mut view.merge else {
            return;
        };
        let Some(fixer) = &run.fixer else {
            return;
        };
        let stopped = git::unmerged(&view.repo) || git::merging(&view.repo);
        let target = run.target.clone();
        if !stopped && git::contains(&view.repo, &fixer.tip, &target) {
            let merged = view.key;
            if !closed && let Some(fixer) = self.views.iter().position(|v| v.key == key) {
                drop(self.drop_view(fixer));
            }
            if let Some(index) = self.views.iter().position(|v| v.key == merged) {
                self.merged(index);
            }
        } else if !stopped {
            run.fixer = None;
            self.notify(format!("the merge into {target} was aborted"));
        } else if closed {
            run.fixer = None;
            let repo = view.repo.display().to_string();
            self.notify(format!(
                "the merge into {target} is unfinished in {repo}: fix it, or git merge --abort"
            ));
        }
    }

    /// The key after the leader: a session or tab to go to, or a shortcut.
    fn on_leader_key(&mut self, key: KeyEvent) {
        // Twice: the program in the tab gets it.
        if keys::matches(self.config.leader, &key) {
            let leader = keys::event(self.config.leader);
            if let Some(pty) = &mut self.views[self.active].terminal_mut().pty {
                pty.key(leader);
            }
            return;
        }
        if let Some(command) = self.command_for(&key, true) {
            self.run_command(command);
        }
    }

    /// What `key` does in the tab with your keys, after the leader or on its own: go to a peer tab
    /// whose key it is, or put it beside, or an action that works there. The config keeps them off
    /// each other's.
    fn command_for(&self, key: &KeyEvent, after_leader: bool) -> Option<Command> {
        let view = &self.views[self.active];
        let is = |bindings: &Bindings| {
            bindings
                .0
                .iter()
                .any(|b| b.leader == after_leader && keys::matches(b.key, key))
        };
        if let Some(index) = view.tabs.iter().position(|t| is(&t.def.key)) {
            return Some(Command::Tab(index + 1));
        }
        if let Some(index) = view.tabs.iter().position(|t| is(&t.def.split_key)) {
            return Some(Command::Split(index + 1));
        }
        let peer = view.tab > 0;
        self.config
            .keys
            .iter()
            .find(|&(action, bindings)| action.works_in(peer) && is(bindings))
            .map(|(&action, _)| Command::Action(action))
    }

    fn run_command(&mut self, command: Command) {
        let action = match command {
            Command::Tab(tab) => return self.select_tab(tab),
            Command::Split(tab) => return self.split_tab(tab),
            Command::Action(action) => action,
        };
        let count = self.views.len();
        let view = &self.views[self.active];
        let (tab, tabs) = (view.tab, view.tabs.len() + 1);
        if let Some(index) = action.session() {
            if index < count {
                self.active = index;
            }
            return;
        }
        match action {
            // Stops the tab's command first if it is still running.
            Action::RerunTab => {
                if let Some(index) = tab.checked_sub(1) {
                    self.start_tab(self.active, index);
                }
            }
            Action::SubmitAndNewSession => self.submit(Then::OpenPlain),
            Action::SubmitAndStay => self.submit(Then::Stay),
            Action::NewSession => self.open_new(true),
            Action::NewSessionNoWorktree => self.open_new(false),
            Action::PreviousSession => self.active = (self.active + count - 1) % count,
            Action::NextSession => self.active = (self.active + 1) % count,
            Action::Find => self.open_finder(),
            Action::Merge => self.open_merge(),
            Action::Quit => self.request_quit(),
            Action::Chat => self.select_tab(0),
            Action::ChatSplit => self.split_tab(0),
            Action::OtherPane => self.other_pane(),
            Action::PreviousTab => self.select_tab((tab + tabs - 1) % tabs),
            Action::NextTab => self.select_tab((tab + 1) % tabs),
            // Above.
            Action::Session1
            | Action::Session2
            | Action::Session3
            | Action::Session4
            | Action::Session5
            | Action::Session6
            | Action::Session7
            | Action::Session8
            | Action::Session9 => {}
        }
    }

    fn on_modal_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.modal = None;
            return;
        }
        if let Some(Modal::Merge(panel)) = &mut self.modal {
            let count = self.config.merges.len();
            let newline = key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
            match key.code {
                // Finishing a merge has no action to pick.
                KeyCode::Tab | KeyCode::BackTab if panel.finish => {}
                KeyCode::Tab => panel.action = (panel.action + 1) % count,
                KeyCode::BackTab => panel.action = (panel.action + count - 1) % count,
                KeyCode::Enter if !newline => {
                    if panel.commit_message() == Some("") {
                        panel.needs_message = true;
                    } else if let Some(Modal::Merge(panel)) = self.modal.take() {
                        self.run_merge(panel);
                    }
                }
                // Only a message to type: nothing else in the panel takes keys.
                _ => {
                    if panel.commit_message().is_some() && panel.message.handle_key(key) {
                        panel.needs_message = false;
                    }
                }
            }
            return;
        }
        let Some(Modal::Find(finder)) = &mut self.modal else {
            return;
        };
        match key.code {
            KeyCode::Up => finder.list.select_previous(),
            KeyCode::Down => finder.list.select_next(),
            KeyCode::BackTab => {
                finder.active_only = !finder.active_only;
                finder.refresh();
            }
            KeyCode::Enter => match finder.selected() {
                Some(Target::View(index)) => {
                    self.modal = None;
                    self.active = index;
                }
                Some(Target::Old(index)) => {
                    let past = finder.old[index].past.clone();
                    let cwd = past.cwd.clone().unwrap_or_else(|| finder.cwd.clone());
                    self.modal = None;
                    if let Err(err) = self.open(resume_from(&cwd), Some(&past), true) {
                        self.notify(format!("resuming {} failed: {err:#}", past.id));
                    }
                }
                None => {}
            },
            // The query is one line.
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            _ => {
                if finder.query.handle_key(key) {
                    finder.refresh();
                }
            }
        }
    }
}

/// What leader m would take from merge session `view`, which fixed the conflicts of the merge of
/// `merged` into its main checkout: the commit that finishes it. Or why it can't now.
fn finish_panel(view: &View, merged: &View) -> Result<MergePanel, String> {
    if !git::merging(&view.cwd) {
        return Err("no merge to finish here: it was committed or aborted".into());
    }
    // `git commit -a` would take them, conflict markers and all.
    if git::unmerged(&view.cwd) {
        return Err("files are still in conflict: resolve them and git add them first".into());
    }
    // Untracked files stay out of it.
    let changes = git::changes(&view.cwd)?
        .into_iter()
        .filter(|change| !change.starts_with("??"))
        .collect();
    Ok(MergePanel {
        view: view.key,
        branch: git::branch(&merged.cwd).unwrap_or_default(),
        target: merged.merge.as_ref().map(|m| m.target.clone()),
        changes,
        ahead: 0,
        others: 0,
        message: Input::default(),
        action: 0,
        needs_message: false,
        finish: true,
    })
}

/// The name of the worktree at `cwd`, if it is one Claude made: `.claude/worktrees/<name>`.
pub(crate) fn worktree_name(cwd: &Path) -> Option<String> {
    if !cwd.parent()?.ends_with(".claude/worktrees") {
        return None;
    }
    cwd.file_name()?.to_str().map(str::to_owned)
}

/// Where to start Claude to resume a session that ran in `cwd`. From the main checkout for a
/// worktree it made: it goes back into the worktree itself, or says the worktree is gone.
fn resume_from(cwd: &Path) -> &Path {
    history::worktree_repo(cwd).unwrap_or(cwd)
}

/// Reads the title Claude gave the session's conversation, so its chip follows it. Does nothing
/// before Claude has reported the session's id, or before it has written a title.
fn refresh_ai_title(session: &mut Session, cwd: &Path) {
    let Some(id) = session.session_id.as_deref() else {
        return;
    };
    let Ok(dir) = history::projects_dir() else {
        return;
    };
    let transcript = dir
        .join(history::encode_cwd(cwd))
        .join(format!("{id}.jsonl"));
    if let Some(title) = history::ai_title(&transcript) {
        session.ai_title = Some(title);
    }
}

/// A turn is running. Claude titles a conversation a few seconds into a turn and retitles it when
/// you approve a plan, often minutes before the turn ends.
fn in_turn(session: &Session) -> bool {
    matches!(session.state(), State::Working | State::WaitingPermission)
}

/// `text` quoted for `sh`.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The session after `from` that needs you most: one that fixed a merge's conflicts, then a
/// permission prompt, then one waiting for a prompt, then one still starting. Among equals, the
/// nearest after `from`, wrapping around.
fn next_waiting(sessions: &[(State, Role)], from: usize) -> Option<usize> {
    (1..sessions.len())
        .map(|offset| (from + offset) % sessions.len())
        .filter_map(|i| urgency(sessions[i]).map(|rank| (rank, i)))
        .min_by_key(|&(rank, _)| rank)
        .map(|(_, i)| i)
}

/// Lower needs you sooner; `None` when the session doesn't need you.
fn urgency((state, role): (State, Role)) -> Option<u8> {
    match (role, state) {
        (Role::Fixing, State::WaitingPermission | State::Idle) => Some(0),
        // A Claude asking is never left waiting, even in the session merged.
        (Role::Plain | Role::Merged, State::WaitingPermission) => Some(1),
        (Role::Plain, State::Idle) => Some(2),
        (Role::Plain, State::Starting) => Some(3),
        // A merge session starting starts on the conflicts.
        (Role::Fixing | Role::Merged, _) | (Role::Plain, State::Working | State::Asleep) => None,
    }
}

/// Where the session on screen, `active`, is after session `closed` closed, with `left` sessions
/// left. If it was the one closed, the next takes its place, or the previous one if it was last.
fn active_after_close(active: usize, closed: usize, left: usize) -> usize {
    if closed < active {
        active - 1
    } else {
        active.min(left - 1)
    }
}

#[cfg(test)]
mod tests {
    use ccs_claude::session::State::{Asleep, Idle, Starting, WaitingPermission, Working};
    use ccs_core::{
        config::{MERGE_COMMIT, MergeDef},
        keys::Key,
    };
    use crossterm::event::MouseEvent;
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    /// Where Enter goes from session `from` of sessions in `states`, none of them in a merge.
    fn next_plain(states: &[State], from: usize) -> Option<usize> {
        let sessions: Vec<(State, Role)> = states.iter().map(|&s| (s, Role::Plain)).collect();
        next_waiting(&sessions, from)
    }

    #[test]
    fn advances_to_the_session_that_needs_you_most() {
        // A permission prompt beats a nearer idle session.
        assert_eq!(next_plain(&[Working, Idle, WaitingPermission], 0), Some(2));
        // Among equals, the nearest after the current session, wrapping around.
        assert_eq!(next_plain(&[Idle, Working, Idle], 1), Some(2));
        assert_eq!(next_plain(&[Idle, Idle, Working], 2), Some(0));
        // A session still starting comes last.
        assert_eq!(next_plain(&[Working, Starting, Idle], 0), Some(2));
    }

    #[test]
    fn busy_and_current_sessions_are_not_waiting() {
        assert_eq!(next_plain(&[Working, Working, Working], 0), None);
        assert_eq!(next_plain(&[Idle], 0), None);
        // Its background work or a scheduled wakeup goes on without you.
        assert_eq!(next_plain(&[Working, Asleep, Idle], 2), None);
    }

    #[test]
    fn a_session_that_fixed_conflicts_comes_first_and_the_merged_one_waits() {
        use Role::{Fixing, Merged, Plain};
        // Done fixing, it beats a permission prompt nearer by, whether it asks or waits.
        for state in [Idle, WaitingPermission] {
            assert_eq!(
                next_waiting(
                    &[
                        (Working, Plain),
                        (WaitingPermission, Plain),
                        (state, Fixing)
                    ],
                    0
                ),
                Some(2)
            );
        }
        // Still fixing, or starting to: busy.
        for state in [Starting, Working, Asleep] {
            assert_eq!(
                next_waiting(&[(Idle, Plain), (state, Fixing), (Starting, Plain)], 0),
                Some(2)
            );
        }
        // The session merged has nothing to do while another one fixes its conflicts, unless its
        // Claude asks you something.
        assert_eq!(next_waiting(&[(Working, Plain), (Idle, Merged)], 0), None);
        assert_eq!(
            next_waiting(
                &[(Working, Plain), (Idle, Plain), (WaitingPermission, Merged)],
                0
            ),
            Some(2)
        );
    }

    #[test]
    fn titles_are_read_while_a_turn_runs() {
        let mut session = Session::default();
        let mut read = vec![in_turn(&session)];
        for event in [
            Bridge::Ready {
                session_id: "5c1c".into(),
                root: "/work".into(),
            },
            Bridge::Submitted,
            Bridge::Permission,
            Bridge::Approved,
            Bridge::Asleep,
            Bridge::Idle,
        ] {
            session.bridge(event);
            read.push(in_turn(&session));
        }
        // Starting, Idle, Working, WaitingPermission, Working, Working, Asleep.
        assert_eq!(read, [false, false, true, true, true, true, false]);
    }

    #[test]
    fn closing_a_session_keeps_the_one_on_screen() {
        // A session before it closed: it moved up.
        assert_eq!(active_after_close(2, 0, 3), 1);
        // One after it closed: it stays.
        assert_eq!(active_after_close(1, 2, 2), 1);
        // It closed: the next takes its place, or the previous if it was last.
        assert_eq!(active_after_close(1, 1, 2), 1);
        assert_eq!(active_after_close(2, 2, 2), 1);
    }

    /// An app with no session, whose terminals report nowhere.
    fn app() -> App {
        let (scans, _) = mpsc::unbounded_channel();
        let (ptys, _) = mpsc::unbounded_channel();
        App::new(Config::default(), scans, ptys, PathBuf::new(), None)
    }

    /// A session in `cwd` whose Claude never ran.
    fn view(key: u64, cwd: PathBuf) -> View {
        View {
            key,
            repo: cwd.clone(),
            cwd,
            worktree: None,
            waiting: false,
            session: Session::default(),
            claude: Tab {
                def: TabDef {
                    name: "Chat".into(),
                    key: Bindings::after_leader(Key::char('c')),
                    split_key: Bindings::default(),
                    mode: Mode::Lazy,
                    command: CLAUDE.into(),
                },
                pty: None,
                key,
                exited: None,
                error: None,
            },
            last_key: None,
            tabs: Vec::new(),
            tab: 0,
            split: None,
            merge: None,
        }
    }

    #[test]
    fn a_new_session_takes_the_spare_in_its_directory() {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let mut app = app();
        // Still starting, so no other spare starts. Its files are in its worktree.
        let mut spare = view(7, cwd.clone());
        spare.cwd = cwd.join(".claude/worktrees/ccs-7");
        app.spare = Some(spare);
        app.open(&cwd, None, true).unwrap();
        assert_eq!(app.views.iter().map(|v| v.key).collect::<Vec<_>>(), [7]);
        assert_eq!(app.active, 0);
        assert!(app.spare.is_none());
    }

    #[test]
    fn leader_keys_choose_what_a_submitted_prompt_does() {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let mut app = app();
        for key in 0..3 {
            let mut view = view(key, cwd.clone());
            // In place of Claude, to take the Enter ccs presses.
            let pty = Pty::spawn("cat", &cwd, &[], (80, 24), key, app.ptys.clone()).unwrap();
            view.claude.pty = Some(pty);
            // Waiting for a prompt; through the view, so no spare starts.
            view.session.bridge(Bridge::Ready {
                session_id: format!("s{key}"),
                root: cwd.clone(),
            });
            app.views.push(view);
        }
        let leader_key = |app: &mut App, code| app.on_leader_key(KeyEvent::from(code));
        // Leader w stays, though others wait.
        leader_key(&mut app, KeyCode::Char('w'));
        app.on_bridge(0, Bridge::Submitted);
        assert_eq!(app.active, 0);
        // Enter goes to the next one waiting.
        app.key_to_terminal(KeyEvent::from(KeyCode::Enter));
        app.on_bridge(0, Bridge::Submitted);
        assert_eq!(app.active, 1);
        // Leader Enter opens a session without a worktree, though one waits: the spare, as it
        // has none.
        app.spare = Some(view(7, cwd.clone()));
        leader_key(&mut app, KeyCode::Enter);
        app.on_bridge(1, Bridge::Submitted);
        assert_eq!(
            app.views.iter().map(|v| v.key).collect::<Vec<_>>(),
            [0, 1, 2, 7]
        );
        assert_eq!(app.active, 3);
        // Leader n and N open one without submitting anything.
        app.spare = Some(view(8, cwd.clone()));
        leader_key(&mut app, KeyCode::Char('n'));
        app.spare = Some(view(9, cwd));
        leader_key(&mut app, KeyCode::Char('N'));
        assert_eq!(
            app.views.iter().map(|v| v.key).collect::<Vec<_>>(),
            [0, 1, 2, 7, 8, 9]
        );
        assert_eq!(app.active, 5);
        assert!(app.notice.is_none());
    }

    #[test]
    fn keys_of_the_config_work_with_the_leader_or_without() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        app.config = Config::parse(
            "[keys]\nsession_2 = [\"<leader>2\", \"<A-2>\"]\nquit = \"<leader>x\"\n\
             submit_and_stay = \"<A-w>\"\n\
             [[tab]]\nname = \"shell\"\ncommand = \"cat\"\nkey = \"<A-s>\"",
        )
        .unwrap();
        for key in 0..2 {
            let mut view = view(key, cwd.clone());
            view.tabs.push(Tab {
                def: app.config.tabs[0].clone(),
                pty: None,
                key: key + 10,
                exited: None,
                error: None,
            });
            app.views.push(view);
        }
        let press =
            |app: &mut App, c, modifiers| app.on_key(KeyEvent::new(KeyCode::Char(c), modifiers));

        // Alt-2, without the leader, goes to session 2; nothing reaches Claude.
        press(&mut app, '2', KeyModifiers::ALT);
        assert_eq!(app.active, 1);
        assert!(app.views[1].last_key.is_none());
        // Alt-s goes to the tab; there, Alt-w doesn't submit anything: it is the program's.
        press(&mut app, 's', KeyModifiers::ALT);
        assert_eq!(app.views[1].tab, 1);
        let alt_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT);
        assert!(app.command_for(&alt_w, false).is_none());
        app.views[1].tab = 0;
        assert!(matches!(
            app.command_for(&alt_w, false),
            Some(Command::Action(Action::SubmitAndStay))
        ));

        // The leader still goes to session 2, and x quits in place of q.
        app.active = 0;
        press(&mut app, ' ', KeyModifiers::CONTROL);
        press(&mut app, '2', KeyModifiers::NONE);
        assert_eq!(app.active, 1);
        app.on_leader_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(!app.quit);
        app.on_leader_key(KeyEvent::from(KeyCode::Char('x')));
        assert!(app.quit);
    }

    #[test]
    fn the_spare_stays_out_of_sight() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        app.views.push(view(0, cwd.clone()));
        app.spare = Some(view(1, cwd.clone()));
        app.on_bridge(
            1,
            Bridge::Ready {
                session_id: "5c1c".into(),
                root: cwd.clone(),
            },
        );
        let spare = app.spare.as_ref().unwrap();
        assert_eq!(spare.session.state(), Idle);
        assert_eq!(spare.session.session_id.as_deref(), Some("5c1c"));
        assert_eq!(app.views[0].session.state(), Starting);
        // Its Claude exiting closes no session, and says nothing.
        app.on_pty(1, PtyEvent::Exited("exit 1".into()));
        assert!(app.spare.is_none());
        assert_eq!(app.views.len(), 1);
        assert!(app.notice.is_none() && !app.quit);
    }

    /// A tab in `mode` whose command waits for input.
    fn tab_def(name: &str, mode: Mode) -> TabDef {
        TabDef {
            name: name.into(),
            key: Bindings::default(),
            split_key: Bindings::default(),
            mode,
            command: "cat".into(),
        }
    }

    #[test]
    fn regular_tabs_run_before_they_are_opened() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        app.config.tabs = vec![
            tab_def("regular", Mode::Regular),
            tab_def("lazy", Mode::Lazy),
            tab_def("eager", Mode::Eager),
        ];
        let mut view = view(0, cwd.clone());
        app.peer_tabs(&mut view);
        let started: Vec<bool> = view.tabs.iter().map(|t| t.pty.is_some()).collect();
        assert_eq!(started, [true, false, false]);
        app.views.push(view);
        let keys = |app: &App| app.views[0].tabs.iter().map(|t| t.key).collect::<Vec<_>>();
        let regular = keys(&app)[0];
        // Opening it, or coming back to it, runs nothing again.
        app.select_tab(1);
        app.select_tab(0);
        app.select_tab(1);
        assert_eq!(keys(&app)[0], regular);
        // A lazy tab starts the first time it is opened, and only then.
        app.select_tab(2);
        let lazy = keys(&app)[1];
        assert!(app.views[0].tabs[1].pty.is_some());
        app.select_tab(0);
        app.select_tab(2);
        assert_eq!(keys(&app)[1], lazy);
        // An eager tab runs again every time you switch to it.
        app.select_tab(3);
        let eager = keys(&app)[2];
        app.select_tab(0);
        app.select_tab(3);
        assert_ne!(keys(&app)[2], eager);
    }

    /// An app showing a session with the default tabs, review (eager) and shell (regular), whose
    /// commands wait for input, in a window of 80 by 24.
    fn app_with_default_tabs() -> App {
        let mut app = app();
        app.window = Rect::new(0, 0, 80, 24);
        app.config.tabs = Config::default()
            .tabs
            .into_iter()
            .map(|def| TabDef {
                command: "cat".into(),
                ..def
            })
            .collect();
        let mut view = view(0, std::env::temp_dir());
        app.peer_tabs(&mut view);
        app.views.push(view);
        app
    }

    /// The tab with the keys of the session on screen, and its split: left, right.
    fn layout(app: &App) -> (usize, Option<(usize, usize)>) {
        let view = &app.views[app.active];
        (view.tab, view.split.map(|split| (split.main, split.side)))
    }

    #[test]
    fn split_keys_put_a_tab_beside_another() {
        let mut app = app_with_default_tabs();
        // Tabs: 0 is Chat, 1 review, 2 shell. Each run of review gives it a new key.
        let review = |app: &App| app.views[0].tabs[0].key;
        let key = |app: &mut App, c| leader_key(app, KeyCode::Char(c));
        key(&mut app, 'R');
        assert_eq!(layout(&app), (1, Some((0, 1))));
        let first = review(&app);
        assert!(app.views[0].tabs[0].pty.is_some());
        // Again: it goes.
        key(&mut app, 'R');
        assert_eq!(layout(&app), (0, None));
        // Coming back runs the eager tab again; shell takes its place on the right.
        key(&mut app, 'R');
        let second = review(&app);
        assert_ne!(second, first);
        key(&mut app, 'S');
        assert_eq!(layout(&app), (2, Some((0, 2))));
        key(&mut app, 'o');
        assert_eq!(layout(&app), (0, Some((0, 2))));
        // The plain key: on its own.
        key(&mut app, 'r');
        assert_eq!(layout(&app), (1, None));
        let third = review(&app);
        assert_ne!(third, second);
        key(&mut app, 'S');
        assert_eq!(layout(&app), (2, Some((1, 2))));
        // Chat takes the right's place, on the left.
        key(&mut app, 'C');
        assert_eq!(layout(&app), (0, Some((0, 1))));
        key(&mut app, 'C');
        assert_eq!(layout(&app), (1, None));
        // On screen alone: Chat comes beside it.
        key(&mut app, 'R');
        assert_eq!(layout(&app), (0, Some((0, 1))));
        key(&mut app, 'o');
        assert_eq!(layout(&app), (1, Some((0, 1))));
        // Review never left the screen, so it didn't run again.
        key(&mut app, 'r');
        assert_eq!(layout(&app), (1, None));
        assert_eq!(review(&app), third);
        key(&mut app, 'c');
        assert_eq!(layout(&app), (0, None));
        // Chat beside itself is nothing.
        key(&mut app, 'C');
        assert_eq!(layout(&app), (0, None));
    }

    #[test]
    fn split_tabs_take_half_the_window_and_a_click_gives_one_the_keys() {
        let mut app = app_with_default_tabs();
        leader_key(&mut app, KeyCode::Char('R'));
        let view = &app.views[0];
        // The rule between the two takes a column. Hidden tabs keep the whole window.
        assert_eq!(view.size(0, app.window), (21, 40));
        assert_eq!(view.size(1, app.window), (21, 39));
        assert_eq!(view.size(2, app.window), (21, 80));
        click(&mut app, 10, 5);
        assert_eq!(layout(&app), (0, Some((0, 1))));
        click(&mut app, 60, 5);
        assert_eq!(layout(&app), (1, Some((0, 1))));
        // Neither the rule nor the bars.
        click(&mut app, 40, 5);
        click(&mut app, 10, 23);
        assert_eq!(layout(&app), (1, Some((0, 1))));
    }

    #[test]
    fn advancing_keeps_a_split_with_chat() {
        let mut app = app_with_default_tabs();
        let mut other = view(1, std::env::temp_dir());
        app.peer_tabs(&mut other);
        app.views.push(other);
        for view in &mut app.views {
            view.session.bridge(Bridge::Ready {
                session_id: "5c1c".into(),
                root: std::env::temp_dir(),
            });
        }
        app.views[1].tab = 1;
        app.views[1].split = Some(Split { main: 0, side: 1 });
        app.advance();
        assert_eq!((app.active, layout(&app)), (1, (0, Some((0, 1)))));
        // Without Chat, Chat comes on its own.
        app.views[0].tab = 2;
        app.views[0].split = Some(Split { main: 1, side: 2 });
        app.advance();
        assert_eq!((app.active, layout(&app)), (0, (0, None)));
    }

    /// A directory for tabs to run in.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ccs-app-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn tabs_wait_for_the_worktree_claude_makes() {
        let repo = scratch("repo");
        let root = scratch("tree");
        let mut app = app();
        app.config.tabs = vec![
            tab_def("regular", Mode::Regular),
            tab_def("lazy", Mode::Lazy),
        ];
        // Being ready starts no other spare.
        app.spare = Some(view(9, repo.clone()));
        let mut view = view(0, repo.clone());
        view.worktree = Some(Worktree {
            name: "ccs-0".into(),
            base: "abc".into(),
        });
        view.waiting = true;
        app.peer_tabs(&mut view);
        app.views.push(view);
        let started = |app: &App| {
            app.views[0]
                .tabs
                .iter()
                .map(|t| t.pty.is_some())
                .collect::<Vec<_>>()
        };
        // Not even when opened, or run again: the worktree isn't there yet.
        app.select_tab(2);
        app.on_leader_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(started(&app), [false, false]);
        app.on_bridge(
            0,
            Bridge::Ready {
                session_id: "5c1c".into(),
                root: root.clone(),
            },
        );
        // The regular tab, and the one on screen, start in the worktree.
        assert_eq!(app.views[0].cwd, root);
        assert!(!app.views[0].waiting);
        assert_eq!(started(&app), [true, true]);
        assert!(app.notice.is_none());
        std::fs::remove_dir_all(repo).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_spare_waits_for_its_worktree_too() {
        let repo = scratch("spare-repo");
        let root = scratch("spare-tree");
        let mut app = app();
        app.config.tabs = vec![tab_def("regular", Mode::Regular)];
        app.views.push(view(0, repo.clone()));
        app.next_key = 2;
        let mut spare = view(1, repo);
        spare.waiting = true;
        app.peer_tabs(&mut spare);
        app.spare = Some(spare);
        assert!(app.spare.as_ref().unwrap().tabs[0].pty.is_none());
        app.on_bridge(
            1,
            Bridge::Ready {
                session_id: "5c1c".into(),
                root: root.clone(),
            },
        );
        let spare = app.spare.as_ref().unwrap();
        assert_eq!(spare.cwd, root);
        assert!(spare.tabs[0].pty.is_some());
        std::fs::remove_dir_all(app.views[0].repo.clone()).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tabs_follow_claude_when_it_moves() {
        let repo = scratch("moves");
        let mut app = app();
        app.config.tabs = vec![tab_def("lazy", Mode::Lazy)];
        // Being ready starts no other spare.
        app.spare = Some(view(9, repo.clone()));
        let mut view = view(0, repo.clone());
        app.peer_tabs(&mut view);
        app.views.push(view);
        // Ready doesn't move a session that asked for no worktree: Claude reports the top of
        // its repository even when started below it.
        let ready = Bridge::Ready {
            session_id: "5c1c".into(),
            root: "/elsewhere".into(),
        };
        app.on_bridge(0, ready);
        assert_eq!(app.views[0].cwd, repo);
        // Nothing runs where it was, so nothing to say.
        let tree = repo.join(".claude/worktrees/a");
        app.on_bridge(0, Bridge::Root { root: tree.clone() });
        assert_eq!(app.views[0].cwd, tree);
        assert!(app.notice.is_none());
        // A tab still runs where it was: it stays there, and you are told.
        app.on_bridge(0, Bridge::Root { root: repo.clone() });
        app.select_tab(1);
        app.on_bridge(0, Bridge::Root { root: tree.clone() });
        assert_eq!(app.views[0].cwd, tree);
        assert!(app.notice.as_deref().is_some_and(|n| n.contains("moved")));
        std::fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn tabs_get_the_variables_of_their_session() {
        let repo = scratch("env");
        let mut view = view(0, repo.clone());
        let env = |view: &View| {
            view.tab_env()
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>()
        };
        let path = |path: &Path| path.display().to_string();
        let before = env(&view);
        assert_eq!(before["CS_CWD"], path(&repo));
        assert_eq!(before["CS_REPO"], path(&repo));
        // Not known yet, or not there: empty, but set.
        for name in [
            "CS_BRANCH",
            "CS_REPO_BRANCH",
            "CS_GIT_BASE",
            "CS_SESSION_ID",
        ] {
            assert_eq!(before[name], "", "{name}");
        }
        view.cwd = repo.join(".claude/worktrees/ccs-0");
        view.worktree = Some(Worktree {
            name: "ccs-0".into(),
            base: "abc".into(),
        });
        view.session.session_id = Some("5c1c".into());
        let after = env(&view);
        assert_eq!(after["CS_CWD"], path(&view.cwd));
        assert_eq!(after["CS_REPO"], path(&repo));
        assert_eq!(after["CS_GIT_BASE"], "abc");
        assert_eq!(after["CS_SESSION_ID"], "5c1c");
        std::fs::remove_dir_all(repo).unwrap();
    }

    /// Draws `app`, then clicks at `column`, `row` with the left button.
    fn click(app: &mut App, column: u16, row: u16) {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| ui::render(frame, app)).unwrap();
        app.on_terminal(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }));
    }

    #[test]
    fn clicking_a_chip_or_a_title_goes_there() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        app.config.tabs = vec![tab_def("shell", Mode::Lazy)];
        for key in 0..2 {
            let mut view = view(key, cwd.clone());
            view.session.name = Some("api".into());
            app.peer_tabs(&mut view);
            app.views.push(view);
        }
        // The session bar: ` 1 ◌ api   2 ◌ api `.
        click(&mut app, 12, 0);
        assert_eq!(app.active, 1);
        // The tab rule: `── c Chat ── shell ──`. The rule between titles is no tab.
        click(&mut app, 10, 1);
        assert_eq!(app.views[1].tab, 0);
        click(&mut app, 14, 1);
        assert_eq!(app.views[1].tab, 1);
        assert!(app.views[1].tabs[0].pty.is_some());
        assert!(app.views[0].tabs[0].pty.is_none());
        // Back to Chat.
        click(&mut app, 3, 1);
        assert_eq!(app.views[1].tab, 0);
    }

    #[test]
    fn swiping_sideways_goes_one_session_a_swipe() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        for key in 0..3 {
            app.views.push(view(key, cwd.clone()));
        }
        let scroll = |app: &mut App, kind| {
            app.on_terminal(Event::Mouse(MouseEvent {
                kind,
                column: 5,
                row: 10,
                modifiers: KeyModifiers::NONE,
            }));
        };
        let pause = |app: &mut App| *app.last_swipe.as_mut().unwrap() -= SWIPE_GAP;
        scroll(&mut app, MouseEventKind::ScrollRight);
        assert_eq!(app.active, 1);
        // The rest of the same swipe.
        scroll(&mut app, MouseEventKind::ScrollRight);
        assert_eq!(app.active, 1);
        pause(&mut app);
        scroll(&mut app, MouseEventKind::ScrollRight);
        assert_eq!(app.active, 2);
        // The last session is as far as it goes.
        pause(&mut app);
        scroll(&mut app, MouseEventKind::ScrollRight);
        assert_eq!(app.active, 2);
        pause(&mut app);
        scroll(&mut app, MouseEventKind::ScrollLeft);
        assert_eq!(app.active, 1);
    }

    #[test]
    fn shift_scrolling_goes_one_session_a_swipe() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        for key in 0..3 {
            app.views.push(view(key, cwd.clone()));
        }
        let scroll = |app: &mut App, kind, modifiers| {
            app.on_terminal(Event::Mouse(MouseEvent {
                kind,
                column: 5,
                row: 10,
                modifiers,
            }));
        };
        let pause = |app: &mut App| *app.last_swipe.as_mut().unwrap() -= SWIPE_GAP;
        scroll(&mut app, MouseEventKind::ScrollDown, KeyModifiers::SHIFT);
        assert_eq!(app.active, 1);
        // The rest of the same swipe.
        scroll(&mut app, MouseEventKind::ScrollDown, KeyModifiers::SHIFT);
        assert_eq!(app.active, 1);
        pause(&mut app);
        scroll(&mut app, MouseEventKind::ScrollUp, KeyModifiers::SHIFT);
        assert_eq!(app.active, 0);
        // Without Shift the wheel scrolls, and stays in the session.
        pause(&mut app);
        scroll(&mut app, MouseEventKind::ScrollDown, KeyModifiers::NONE);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn switching_sessions_slides_in_from_the_side_of_the_chip() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        app.window = Rect::new(0, 0, 80, 24);
        for key in 0..3 {
            app.views.push(view(key, cwd.clone()));
        }
        let slid = |app: &App| app.slide.as_ref().map(|s| (s.from, s.forward));
        let leader_key = |app: &mut App, c| app.on_leader_key(KeyEvent::from(KeyCode::Char(c)));
        // The next session comes in from the right, the previous one from the left.
        leader_key(&mut app, '.');
        app.slide_from(0);
        assert_eq!(slid(&app), Some((0, true)));
        leader_key(&mut app, ',');
        app.slide_from(1);
        assert_eq!(slid(&app), Some((1, false)));
        // Before the first is the last, whose chip is on the right.
        leader_key(&mut app, ',');
        app.slide_from(0);
        assert_eq!(slid(&app), Some((0, true)));
        // It slides from nothing to all of it.
        assert_eq!(app.slide(80).map(|(_, moved, _)| moved), Some(0));
        app.slide.as_mut().unwrap().started -= Duration::from_secs(1);
        assert_eq!(app.slide(80).map(|(_, moved, _)| moved), Some(80));

        // At one speed: twice as wide takes twice as long.
        let took = |app: &App| app.slide.as_ref().map(|s| s.duration);
        app.window = Rect::new(0, 0, 100, 24);
        leader_key(&mut app, '.');
        app.slide_from(2);
        assert_eq!(took(&app), Some(Duration::from_millis(150)));
        app.window = Rect::new(0, 0, 200, 24);
        leader_key(&mut app, '.');
        app.slide_from(0);
        assert_eq!(took(&app), Some(Duration::from_millis(300)));

        // No switch, a session that closed since, or no slide in the config: no slide.
        app.slide = None;
        app.slide_from(1);
        app.slide_from(7);
        assert_eq!(slid(&app), None);
        app.config.slide_ms = 0;
        leader_key(&mut app, '.');
        app.slide_from(1);
        assert_eq!(slid(&app), None);
        // Nor one too short to draw.
        app.config.slide_ms = 1;
        app.window = Rect::new(0, 0, 80, 24);
        leader_key(&mut app, '.');
        app.slide_from(2);
        assert_eq!(slid(&app), None);
    }

    #[test]
    fn a_spare_tab_that_exits_is_marked_finished() {
        let cwd = std::env::temp_dir();
        let mut app = app();
        app.config.tabs = vec![tab_def("regular", Mode::Regular)];
        // Past the keys of the two sessions.
        app.next_key = 2;
        let mut spare = view(1, cwd.clone());
        app.peer_tabs(&mut spare);
        let key = spare.tabs[0].key;
        app.views.push(view(0, cwd));
        app.spare = Some(spare);
        app.on_pty(key, PtyEvent::Exited("exit 0".into()));
        let tab = &app.spare.as_ref().unwrap().tabs[0];
        assert_eq!(tab.exited.as_deref(), Some("exit 0"));
        assert_eq!(app.views.len(), 1);
        assert!(app.notice.is_none() && !app.quit);
    }

    type PtyEvents = mpsc::UnboundedReceiver<(u64, PtyEvent)>;

    /// An app with no session, whose terminals report to the receiver it comes with. Its keys
    /// start past those of the sessions the tests make.
    fn app_with_ptys() -> (App, PtyEvents) {
        let (scans, _) = mpsc::unbounded_channel();
        let (ptys, events) = mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), scans, ptys, PathBuf::new(), None);
        app.next_key = 10;
        (app, events)
    }

    /// Waits for terminal `key` to exit, and tells `app`.
    fn wait_exit(app: &mut App, events: &mut PtyEvents, key: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match events.try_recv() {
                Ok((at, PtyEvent::Exited(status))) if at == key => {
                    return app.on_pty(at, PtyEvent::Exited(status));
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        panic!("terminal {key} didn't exit");
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "git {args:?}: {stderr}");
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    }

    /// A repository with one commit on `main`, and a worktree `ccs-1` on `worktree-ccs-1` as
    /// Claude makes it. Their paths, canonical.
    fn merge_repo(name: &str) -> (PathBuf, PathBuf) {
        let repo = std::env::temp_dir().join(format!("ccs-merge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "-q", "-b", "main"]);
        // Whatever your own git config says.
        for (key, value) in [
            ("user.name", "ccs"),
            ("user.email", "ccs@example.com"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            run_git(&repo, &["config", key, value]);
        }
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        run_git(&repo, &["add", "a.txt"]);
        run_git(&repo, &["commit", "-qm", "init"]);
        git::exclude_worktrees(&repo).unwrap();
        let tree = repo.join(".claude/worktrees/ccs-1");
        let path = tree.to_str().unwrap();
        run_git(
            &repo,
            &["worktree", "add", "-q", "-b", "worktree-ccs-1", path],
        );
        (repo.canonicalize().unwrap(), tree.canonicalize().unwrap())
    }

    /// A session started in `repo`, working in `cwd`, waiting for a prompt.
    fn ready_view(key: u64, repo: &Path, cwd: &Path) -> View {
        let mut view = view(key, repo.to_owned());
        view.cwd = cwd.to_owned();
        view.session.bridge(Bridge::Ready {
            session_id: format!("s{key}"),
            root: cwd.to_owned(),
        });
        view
    }

    fn leader_key(app: &mut App, code: KeyCode) {
        app.on_leader_key(KeyEvent::from(code));
    }

    fn modal_keys(app: &mut App, text: &str) {
        for c in text.chars() {
            app.on_modal_key(KeyEvent::from(KeyCode::Char(c)));
        }
    }

    /// Runs the merge the panel on screen shows, and waits for it to end.
    fn merge(app: &mut App, events: &mut PtyEvents) {
        app.on_modal_key(KeyEvent::from(KeyCode::Enter));
        assert!(app.modal.is_none(), "{:?}", app.notice);
        let view = &app.views[app.active];
        assert!(view.on_merge_tab());
        let key = view.tabs.last().unwrap().key;
        wait_exit(app, events, key);
    }

    fn keys(app: &App) -> Vec<u64> {
        app.views.iter().map(|v| v.key).collect()
    }

    #[test]
    fn leader_m_commits_merges_and_closes_the_session() {
        let (repo, tree) = merge_repo("in");
        let (mut app, mut events) = app_with_ptys();
        // Not the last session, which would start a Claude in its place.
        app.views.push(view(0, repo.clone()));
        let mut view = ready_view(1, &repo, &tree);
        view.session.name = Some("Add b".into());
        app.views.push(view);
        app.active = 1;
        std::fs::write(tree.join("b.txt"), "b\n").unwrap();
        leader_key(&mut app, KeyCode::Char('m'));
        let Some(Modal::Merge(panel)) = &app.modal else {
            panic!("no panel: {:?}", app.notice);
        };
        assert_eq!(panel.branch, "worktree-ccs-1");
        assert_eq!(panel.target.as_deref(), Some("main"));
        assert_eq!(panel.changes, ["?? b.txt"]);
        // Prefilled with the session's name.
        assert_eq!(panel.message.text(), "Add b");
        merge(&mut app, &mut events);
        // The session closed, its worktree went, and main has its commit.
        assert_eq!(keys(&app), [0]);
        assert_eq!(app.notice.as_deref(), Some("merged Add b into main"));
        assert!(app.notice_ok);
        for cleanup in app.cleanups.drain(..) {
            cleanup.join().unwrap();
        }
        assert!(!tree.exists());
        assert_eq!(run_git(&repo, &["log", "--format=%s"]), "Add b\ninit");
        assert_eq!(
            run_git(&repo, &["branch", "--format=%(refname:short)"]),
            "main"
        );
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn a_merge_moves_you_on_as_a_submitted_prompt_does() {
        // Past the busy session beside it, to one that waits; with none waiting, to a new one.
        for waits in [true, false] {
            let (repo, tree) = merge_repo(if waits { "on-waits" } else { "on-new" });
            let (mut app, mut events) = app_with_ptys();
            app.views.push(ready_view(0, &repo, &tree));
            let mut busy = ready_view(1, &repo, &repo);
            busy.session.bridge(Bridge::Submitted);
            app.views.push(busy);
            if waits {
                app.views.push(ready_view(2, &repo, &repo));
            } else {
                // Still starting, so no other spare starts.
                app.spare = Some(view(2, repo.clone()));
            }
            std::fs::write(tree.join("b.txt"), "b\n").unwrap();
            leader_key(&mut app, KeyCode::Char('m'));
            modal_keys(&mut app, "Add b");
            merge(&mut app, &mut events);
            assert_eq!(keys(&app), [1, 2]);
            let view = &app.views[app.active];
            assert_eq!(view.key, 2);
            assert_eq!(view.tab, 0);
            for cleanup in app.cleanups.drain(..) {
                cleanup.join().unwrap();
            }
            std::fs::remove_dir_all(&repo).unwrap();
        }
    }

    #[test]
    fn leader_m_waits_for_claude_and_for_something_to_merge() {
        let (repo, tree) = merge_repo("refuse");
        let mut app = app();
        app.views.push(ready_view(0, &repo, &tree));
        let refused = |app: &mut App| {
            leader_key(app, KeyCode::Char('m'));
            assert!(app.modal.is_none());
            app.notice.take().unwrap_or_default()
        };
        assert_eq!(
            refused(&mut app),
            "nothing to merge: worktree-ccs-1 is in main already"
        );
        std::fs::write(tree.join("b.txt"), "b\n").unwrap();
        // Working, or waiting on background work, which closing the session would stop.
        app.views[0].session.bridge(Bridge::Submitted);
        assert_eq!(refused(&mut app), "Claude is working: merge when it's done");
        app.views[0].session.bridge(Bridge::Asleep);
        app.views[0].session.bridge(Bridge::Idle);
        assert_eq!(app.views[0].session.state(), State::Asleep);
        assert_eq!(
            refused(&mut app),
            "Claude is waiting on background work or a wakeup: stop it before merging"
        );
        app.views[0].session.bridge(Bridge::Submitted);
        app.views[0].session.bridge(Bridge::Idle);
        leader_key(&mut app, KeyCode::Char('m'));
        assert!(matches!(app.modal, Some(Modal::Merge(_))));
        // A message it has to have.
        app.on_modal_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(&app.modal, Some(Modal::Merge(panel)) if panel.needs_message));
        app.modal = None;

        // A session in the main checkout, on main, with nothing to commit.
        app.views[0].cwd.clone_from(&repo);
        assert_eq!(refused(&mut app), "nothing to commit on main");
        // Outside git.
        let outside = scratch("merge-nogit");
        app.views[0].cwd.clone_from(&outside);
        assert_eq!(refused(&mut app), "this session is on no branch");
        app.config.merges.clear();
        assert_eq!(refused(&mut app), "no [[merge]] action in the config");
        std::fs::remove_dir_all(&repo).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn conflicts_in_the_worktree_go_to_its_own_claude() {
        let (repo, tree) = merge_repo("cwd");
        let (mut app, mut events) = app_with_ptys();
        app.config.merges = vec![MergeDef {
            name: "update".into(),
            commit: MERGE_COMMIT.into(),
            command: "git merge --no-edit main".into(),
            on_conflict: Some("Fix the merge of $CS_REPO_BRANCH into ${CS_BRANCH}.".into()),
        }];
        let mut view = ready_view(0, &repo, &tree);
        // In place of Claude, to take the prompt.
        let pty = Pty::spawn("cat", &tree, &[], (80, 24), 0, app.ptys.clone()).unwrap();
        view.claude.pty = Some(pty);
        app.views.push(view);
        // Waiting for you, for you to go to.
        app.views.push(ready_view(1, &repo, &repo));
        // The same line, changed on both sides.
        std::fs::write(tree.join("a.txt"), "tree\n").unwrap();
        run_git(&tree, &["commit", "-qam", "tree"]);
        std::fs::write(repo.join("a.txt"), "main\n").unwrap();
        run_git(&repo, &["commit", "-qam", "main"]);
        leader_key(&mut app, KeyCode::Char('m'));
        merge(&mut app, &mut events);
        assert!(git::unmerged(&tree));
        assert_eq!(
            app.notice.as_deref(),
            Some("merging new stopped on conflicts: its Claude is fixing them")
        );
        // Busy fixing them: you go on, and its prompt being submitted doesn't move you again.
        assert_eq!(app.active, 1);
        let view = &app.views[0];
        assert!(matches!(view.last_key, Some((_, Then::Stay))));
        // Once it waits for you, it goes first.
        assert_eq!(app.role(view), Role::Fixing);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let wanted = "Fix the merge of main into worktree-ccs-1.";
        while !view
            .claude
            .pty
            .as_ref()
            .unwrap()
            .contents()
            .contains(wanted)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the prompt never came"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::remove_dir_all(&repo).unwrap();
    }

    /// Two sessions of a repository, the second on screen with a commit that conflicts with main,
    /// merged into the main checkout, where it stopped. No Claude starts to fix it, but a third
    /// session is linked as the merge session that does.
    fn conflict_in_main(name: &str) -> (App, PtyEvents, PathBuf, PathBuf) {
        let (repo, tree) = merge_repo(name);
        let (mut app, mut events) = app_with_ptys();
        app.config.merges[0].on_conflict = None;
        app.views.push(ready_view(0, &repo, &repo));
        app.views.push(ready_view(1, &repo, &tree));
        app.active = 1;
        std::fs::write(tree.join("a.txt"), "tree\n").unwrap();
        std::fs::write(repo.join("a.txt"), "main\n").unwrap();
        run_git(&repo, &["commit", "-qam", "main"]);
        leader_key(&mut app, KeyCode::Char('m'));
        modal_keys(&mut app, "Change a");
        merge(&mut app, &mut events);
        let notice = app.notice.take().unwrap_or_default();
        assert!(
            notice.starts_with("merge stopped on conflicts in"),
            "{notice}"
        );
        assert!(git::unmerged(&repo));
        assert_eq!(keys(&app), [0, 1]);
        // A session fixing it, as one started with the conflict prompt would.
        app.views.push(ready_view(2, &repo, &repo));
        let tip = git::head(&tree).unwrap();
        app.views[1].merge.as_mut().unwrap().fixer = Some(Fixer { key: 2, tip });
        (app, events, repo, tree)
    }

    #[test]
    fn a_merge_session_is_reviewed_then_finished_with_leader_m() {
        let (mut app, mut events, repo, tree) = conflict_in_main("finish");
        // Busy, right after the merge session.
        let mut busy = ready_view(3, &repo, &repo);
        busy.session.bridge(Bridge::Submitted);
        app.views.push(busy);
        // Fixing: you go on to a session that waits, not to the one merged.
        app.views[2].session.bridge(Bridge::Submitted);
        app.advance();
        assert_eq!(app.active, 0);
        // Done: it goes first, before the session merged.
        app.on_bridge(2, Bridge::Idle);
        app.advance();
        assert_eq!(app.active, 2);
        assert_eq!(app.merge_session_target(2), Some("main"));
        // Leader m only finishes the merge, once nothing is in conflict.
        leader_key(&mut app, KeyCode::Char('m'));
        assert!(app.modal.is_none());
        assert_eq!(
            app.notice.take().as_deref(),
            Some("files are still in conflict: resolve them and git add them first")
        );
        std::fs::write(repo.join("a.txt"), "both\n").unwrap();
        run_git(&repo, &["add", "a.txt"]);
        // Your edit while reviewing it goes in too.
        std::fs::write(repo.join("a.txt"), "both, reviewed\n").unwrap();
        leader_key(&mut app, KeyCode::Char('m'));
        let Some(Modal::Merge(panel)) = &app.modal else {
            panic!("no panel: {:?}", app.notice);
        };
        assert!(panel.finish);
        assert_eq!(panel.branch, "worktree-ccs-1");
        assert_eq!(panel.target.as_deref(), Some("main"));
        assert_eq!(panel.changes, ["MM a.txt"]);
        assert_eq!(panel.commit_message(), None);
        merge(&mut app, &mut events);
        // Both sessions closed, the worktree went, and the finder sees the branch merged. You go
        // on to the session that waits, past the busy one.
        assert_eq!(keys(&app), [0, 3]);
        assert_eq!(app.views[app.active].key, 0);
        assert_eq!(app.notice.as_deref(), Some("merged new into main"));
        assert_eq!(run_git(&repo, &["show", "HEAD:a.txt"]), "both, reviewed");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), "");
        assert!(
            git::merged_into(&repo, "main")
                .unwrap()
                .contains(&"worktree-ccs-1".to_owned())
        );
        for cleanup in app.cleanups.drain(..) {
            cleanup.join().unwrap();
        }
        assert!(!tree.exists());
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn a_merge_fixed_in_the_main_checkout_closes_both_sessions() {
        let (mut app, _events, repo, tree) = conflict_in_main("fixed");
        // Still in conflict: going idle changes nothing.
        app.on_bridge(2, Bridge::Idle);
        assert_eq!(keys(&app), [0, 1, 2]);
        // Nothing else merges there meanwhile.
        app.active = 0;
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        leader_key(&mut app, KeyCode::Char('m'));
        let notice = app.notice.take().unwrap_or_default();
        assert!(
            notice.ends_with("is merging there already: one merge at a time"),
            "{notice}"
        );
        std::fs::remove_file(repo.join("c.txt")).unwrap();
        // Fixed, and the merge committed.
        std::fs::write(repo.join("a.txt"), "both\n").unwrap();
        run_git(&repo, &["commit", "-qam", "merged"]);
        app.on_bridge(2, Bridge::Idle);
        assert_eq!(keys(&app), [0]);
        assert_eq!(app.notice.as_deref(), Some("merged new into main"));
        for cleanup in app.cleanups.drain(..) {
            cleanup.join().unwrap();
        }
        assert!(!tree.exists());
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn an_aborted_merge_lets_both_sessions_be() {
        let (mut app, _events, repo, _) = conflict_in_main("aborted");
        run_git(&repo, &["merge", "--abort"]);
        app.on_bridge(2, Bridge::Idle);
        assert_eq!(keys(&app), [0, 1, 2]);
        assert_eq!(
            app.notice.as_deref(),
            Some("the merge into main was aborted")
        );
        assert!(!app.views[1].merging());
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn a_running_merge_holds_quitting_and_a_finished_one_does_not_run_again() {
        let (repo, tree) = merge_repo("quit");
        let (mut app, mut events) = app_with_ptys();
        // Waits for a line, then fails.
        app.config.merges[0].command = "read line; exit 1".into();
        app.views.push(ready_view(0, &repo, &tree));
        std::fs::write(tree.join("b.txt"), "b\n").unwrap();
        leader_key(&mut app, KeyCode::Char('m'));
        modal_keys(&mut app, "Add b");
        app.on_modal_key(KeyEvent::from(KeyCode::Enter));
        let key = app.views[0].tabs.last().unwrap().key;
        leader_key(&mut app, KeyCode::Char('q'));
        assert!(!app.quit);
        let notice = app.notice.take().unwrap_or_default();
        assert!(
            notice.ends_with("leader q again to quit anyway"),
            "{notice}"
        );
        leader_key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
        app.quit = false;
        // Keys go to it while it runs.
        app.key_to_terminal(KeyEvent::from(KeyCode::Enter));
        wait_exit(&mut app, &mut events, key);
        assert_eq!(
            app.notice.as_deref(),
            Some("merge failed (exit 1): see its tab")
        );
        assert_eq!(keys(&app), [0]);
        // Over: neither Enter nor leader Enter runs it again.
        app.key_to_terminal(KeyEvent::from(KeyCode::Enter));
        leader_key(&mut app, KeyCode::Enter);
        assert_eq!(app.views[0].tabs.last().unwrap().key, key);
        // The commit stays.
        assert_eq!(run_git(&tree, &["log", "-1", "--format=%s"]), "Add b");
        leader_key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
        // Beside Chat, the merge tab gives way to the next merge, on its own.
        leader_key(&mut app, KeyCode::Char('C'));
        assert_eq!(layout(&app), (0, Some((0, 1))));
        leader_key(&mut app, KeyCode::Char('m'));
        app.on_modal_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(layout(&app), (1, None));
        assert_ne!(app.views[0].tabs[0].key, key);
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
