use std::{path::Path, time::SystemTime};

use ccs_claude::session::State;
use ccs_core::{
    config::Config,
    keys::{Action, Bindings},
};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Position, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Padding, Paragraph},
};

use crate::{
    app::{App, Modal, Tab, View},
    finder::{Finder, Target},
    merge::MergePanel,
    theme,
};

/// Between two tabs in the tab bar.
const TAB_GAP: &str = " ── ";
/// The most changed files the merge panel lists.
const MERGE_CHANGES: usize = 8;

/// The rows of the screen: session bar, tab rule, terminal, bottom rule.
fn layout(area: Rect) -> [Rect; 4] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// Where terminal tabs draw in a window of `area`.
pub fn terminal_area(area: Rect) -> Rect {
    layout(area)[2]
}

/// The size (rows, columns) of every terminal tab in a window of `area`.
pub fn terminal_size(area: Rect) -> (u16, u16) {
    let terminal = terminal_area(area);
    (terminal.height.max(1), terminal.width.max(1))
}

pub fn render(frame: &mut Frame, app: &mut App) {
    let [bar, tabs, terminal, status] = layout(frame.area());
    (app.chip_areas, app.tab_areas) = render_header(frame, app, bar, tabs);
    let view = &app.views[app.active];
    let tab = view.terminal();
    let slide = app.slide(terminal.width);
    match (&tab.pty, &tab.error) {
        (_, Some(error)) => frame.render_widget(
            Paragraph::new(error.clone()).fg(theme::get().error),
            terminal,
        ),
        // The leader panel can cover the cursor's cell, and the cursor would show through. A
        // screen still sliding in has none.
        (Some(pty), None) => {
            let focused = app.modal.is_none() && !app.leader_panel && slide.is_none();
            pty.render(frame, terminal, focused);
        }
        // A peer tab starts once Claude has made the worktree it runs in.
        (None, None) if view.waiting && view.tab > 0 => frame.render_widget(
            Paragraph::new("waiting for Claude to make the session's worktree")
                .fg(theme::get().dim),
            terminal,
        ),
        (None, None) => {}
    }
    if let Some((from, moved, forward)) = slide {
        render_slide(
            frame.buffer_mut(),
            terminal,
            from.terminal(),
            moved,
            forward,
        );
    }
    let notice = app.notice.as_deref().map(|text| (text, app.notice_ok));
    let merge = view
        .on_merge_tab()
        .then(|| key_hint(&app.config, Action::Merge));
    let left = rule_status(notice, tab, view.tab > 0, merge.as_deref());
    render_rule(frame, status, left, leader_hint(app));
    render_leader(frame, app, status);
    render_modal(frame, app);
    theme::get().paint(frame.buffer_mut());
}

/// Pushes the screen of `from` `moved` columns off `area`, by the screen drawn there, which comes
/// in from the right if `forward`.
fn render_slide(buffer: &mut Buffer, area: Rect, from: &Tab, moved: u16, forward: bool) {
    let mut old = Buffer::empty(area);
    // A tab that failed to start leaves blank.
    if let Some(pty) = &from.pty {
        pty.draw(&mut old, area);
    }
    let mut new = Buffer::empty(area);
    for at in area.positions() {
        new[at] = buffer[at].clone();
    }
    slide(&old, &new, area, moved, forward, buffer);
}

/// `old` pushed `moved` columns off `area` by `new`, which comes in from the right if `forward`,
/// else from the left. Into `out`.
fn slide(old: &Buffer, new: &Buffer, area: Rect, moved: u16, forward: bool, out: &mut Buffer) {
    let width = area.width;
    for y in area.top()..area.bottom() {
        let at = |x: u16| Position::new(area.x + x, y);
        for x in 0..width {
            out[at(x)] = if forward {
                if x + moved < width {
                    old[at(x + moved)].clone()
                } else {
                    new[at(x + moved - width)].clone()
                }
            } else if x < moved {
                new[at(x + width - moved)].clone()
            } else {
                old[at(x - moved)].clone()
            };
        }
        // Only here can a wide character be cut in two, and drawn there it would spill onto the
        // next row.
        if let Some(last) = width.checked_sub(1) {
            let cell = &mut out[at(last)];
            if Span::raw(cell.symbol()).width() > 1 {
                cell.set_symbol(" ");
            }
        }
    }
}

/// The session bar and the tab bar. Returns where each session's chip and each tab's title went.
fn render_header(frame: &mut Frame, app: &App, bar: Rect, tabs: Rect) -> (Vec<Rect>, Vec<Rect>) {
    let sessions: Vec<(String, State)> = app
        .views
        .iter()
        .map(|view| (session_name(view), view.session.state()))
        .collect();
    let chips = render_session_bar(frame, bar, &sessions, app.active);
    let view = &app.views[app.active];
    let titles = render_tab_bar(
        frame,
        tabs,
        &tab_labels(view),
        view.tab,
        &home_relative(&view.cwd),
    );
    (chips, titles)
}

/// The `width` columns `x` columns into `area`, and only what of them is in it.
fn columns(area: Rect, x: usize, width: usize) -> Rect {
    let x = area.x.saturating_add(u16::try_from(x).unwrap_or(u16::MAX));
    let width = u16::try_from(width).unwrap_or(u16::MAX);
    Rect::new(x, area.y, width, area.height).intersection(area)
}

/// The name of a session: the one it was given, else Claude's title for it, else `new` until
/// Claude has one.
pub fn session_name(view: &View) -> String {
    view.session
        .name
        .clone()
        .or_else(|| view.session.ai_title.clone())
        .unwrap_or_else(|| "new".to_owned())
}

fn render_modal(frame: &mut Frame, app: &mut App) {
    match &mut app.modal {
        Some(Modal::Find(finder)) => {
            let view_states: Vec<State> = app.views.iter().map(|v| v.session.state()).collect();
            render_finder(frame, finder, &view_states);
        }
        Some(Modal::Merge(panel)) => {
            let actions: Vec<&str> = app.config.merges.iter().map(|m| m.name.as_str()).collect();
            render_merge(frame, panel, &actions);
        }
        None => {}
    }
}

/// The merge panel over everything else: what leader m takes, the commit message when there is
/// something to commit, and the `actions` to pick from.
fn render_merge(frame: &mut Frame, panel: &MergePanel, actions: &[&str]) {
    let theme = theme::get();
    let title = match &panel.target {
        _ if panel.plain() => format!(" Commit · {} ", panel.branch),
        Some(target) => format!(" Merge · {} → {target} ", panel.branch),
        None => format!(" Merge · {} ", panel.branch),
    };
    let committing = !panel.changes.is_empty();
    let mut keys = format!(" Enter {} ", actions[panel.action]);
    if actions.len() > 1 {
        keys.push_str("· Tab action ");
    }
    keys.push_str("· Esc cancel ");
    let block = theme::popup(title).title_bottom(Line::from(keys).fg(theme.dim));
    let mut bottom = merge_notes(panel, actions);

    // As wide as the finder, and as high as it needs to be, if it fits.
    let width = frame.area().width / 5 * 4;
    // The message wraps in the room the panel has, after its border and its `› `.
    let (rows, cursor) = panel.message.layout(width.saturating_sub(4));
    let height = |lines: usize| u16::try_from(lines).unwrap_or(u16::MAX);
    // A blank line before each part.
    let message_height = if committing {
        height(rows.len() + 2)
    } else {
        0
    };
    let bottom_height = if bottom.is_empty() {
        0
    } else {
        height(bottom.len() + 1)
    };
    let changes = panel.changes.len().min(MERGE_CHANGES);
    let wanted = height(1 + changes + 2) + message_height + bottom_height;
    let area = frame
        .area()
        .centered(Constraint::Length(width), Constraint::Length(wanted));
    frame.render_widget(Clear, area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [top, message, rest] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(message_height),
        Constraint::Length(bottom_height),
    ])
    .areas(inner);

    let mut counts = Vec::new();
    if panel.ahead > 0 {
        counts.push(plural(panel.ahead, "commit"));
    }
    if committing {
        counts.push(plural(panel.changes.len(), "changed file"));
    }
    let mut lines = vec![Line::from(counts.join(" · "))];
    // As many changes as fit, and how many more there are.
    let room = usize::from(top.height).saturating_sub(1).min(MERGE_CHANGES);
    let shown = if panel.changes.len() > room {
        room.saturating_sub(1)
    } else {
        panel.changes.len()
    };
    lines.extend(
        panel.changes[..shown]
            .iter()
            .map(|change| Line::from(format!("  {change}")).fg(theme.dim)),
    );
    if shown < panel.changes.len() {
        lines.push(Line::from(format!("  … {} more", panel.changes.len() - shown)).fg(theme.dim));
    }
    frame.render_widget(Paragraph::new(lines), top);

    if committing {
        let mut lines = vec![Line::default(), Line::from("Commit message").fg(theme.dim)];
        lines.extend(rows.into_iter().enumerate().map(|(i, row)| {
            let gutter = if i == 0 { "› " } else { "  " };
            Line::from(vec![Span::raw(gutter).fg(theme.accent), Span::raw(row)])
        }));
        frame.render_widget(Paragraph::new(lines), message);
        let at = (message.x + 2 + cursor.0, message.y + 2 + cursor.1);
        if message.contains(at.into()) {
            frame.set_cursor_position(at);
        }
    }
    if !bottom.is_empty() {
        bottom.insert(0, Line::default());
        frame.render_widget(Paragraph::new(bottom), rest);
    }
}

/// The end of the merge panel: the `actions` to pick from when there are several, and what you
/// should know before Enter.
fn merge_notes(panel: &MergePanel, actions: &[&str]) -> Vec<Line<'static>> {
    let theme = theme::get();
    let mut notes = Vec::new();
    if actions.len() > 1 {
        let mut spans = vec![Span::raw("Action ").fg(theme.dim)];
        for (i, name) in actions.iter().enumerate() {
            spans.push(if i == panel.action {
                Span::raw(format!(" ‹ {name} › ")).fg(theme.accent).bold()
            } else {
                Span::raw(format!("   {name}   "))
            });
        }
        notes.push(Line::from(spans));
    }
    if panel.target.is_none() {
        notes.push(Line::from("The main checkout is on no branch.").fg(theme.warning));
    }
    if panel.others > 0 {
        let sessions = plural(panel.others, "other session");
        notes.push(
            Line::from(format!("Merging changes the files of {sessions}.")).fg(theme.warning),
        );
    }
    if panel.needs_message {
        notes.push(Line::from("Type a commit message.").fg(theme.error));
    }
    notes
}

/// `count` `thing`s, in words: `1 commit`, `2 commits`.
fn plural(count: usize, thing: &str) -> String {
    if count == 1 {
        format!("1 {thing}")
    } else {
        format!("{count} {thing}s")
    }
}

/// The finder over everything else; `states` are those of the open sessions.
fn render_finder(frame: &mut Frame, finder: &mut Finder, states: &[State]) {
    let area = centered(frame.area(), 80, 60);
    frame.render_widget(Clear, area);
    let scope = if finder.active_only {
        "open".to_owned()
    } else {
        format!("open + past in {}", home_relative(&finder.cwd))
    };
    let scanning = if finder.loading && !finder.active_only {
        " · scanning…"
    } else {
        ""
    };
    let block = theme::popup(format!(" Find a session · {scope}{scanning} ")).title_bottom(
        Line::from(" ↑↓ select · Enter go · ⇧Tab open/all · Esc cancel ").fg(theme::get().dim),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [query, list] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);

    let (rows, cursor) = finder.query.layout(query.width.saturating_sub(2));
    let row = rows
        .into_iter()
        .nth(usize::from(cursor.1))
        .unwrap_or_default();
    frame.render_widget(
        Line::from(vec![
            Span::raw("› ").fg(theme::get().accent),
            Span::raw(row),
        ]),
        query,
    );
    frame.set_cursor_position((query.x + 2 + cursor.0, query.y));

    if finder.hits.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching sessions.").fg(theme::get().dim),
            list,
        );
        return;
    }
    let now = SystemTime::now();
    let items: Vec<ListItem> = finder
        .hits
        .iter()
        .map(|hit| {
            let item = &finder.items[hit.item];
            // Open sessions: their number and state. Past ones: a dot, and how old they are.
            let mut spans = match item.target {
                Target::View(i) => {
                    let (glyph, color) = state_glyph(states[i]);
                    vec![
                        Span::raw(format!("{:>2} ", i + 1)),
                        Span::raw(glyph).fg(color),
                    ]
                }
                Target::Old(_) => vec![Span::raw("   "), Span::raw("·").fg(theme::get().dim)],
            };
            spans.push(Span::raw(format!(" {}", item.title)));
            if let Target::Old(i) = item.target {
                spans.push(
                    Span::raw(format!(" · {}", age(now, finder.old[i].past.modified)))
                        .fg(theme::get().dim),
                );
                match finder.old[i].merged {
                    Some(true) => spans.push(Span::raw(" · merged").fg(theme::get().idle)),
                    Some(false) => spans.push(Span::raw(" · not merged").fg(theme::get().warning)),
                    None => {}
                }
            }
            let mut text = Text::from(Line::from(spans));
            if let Some(prompt) = hit.prompt {
                text.push_line(
                    Line::raw(format!("      › {}", item.prompts[prompt])).fg(theme::get().dim),
                );
            }
            ListItem::new(text)
        })
        .collect();
    let widget = List::new(items)
        .highlight_symbol(Span::raw("› ").fg(theme::get().accent))
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_style(Style::new().bg(theme::get().selection).bold());
    frame.render_stateful_widget(widget, list, &mut finder.list);
}

/// The sessions as chips, the one on screen filled, and on the right how many others need you.
/// `sessions` are names and states. Returns where each chip went.
fn render_session_bar(
    frame: &mut Frame,
    area: Rect,
    sessions: &[(String, State)],
    active: usize,
) -> Vec<Rect> {
    let mut chips = Vec::new();
    // Where each chip starts in the bar, and its width.
    let mut bounds = Vec::new();
    let mut x = 0;
    for (i, (name, state)) in sessions.iter().enumerate() {
        let first = chips.len();
        let (glyph, color) = state_glyph(*state);
        if i == active {
            let style = Style::new()
                .bg(theme::get().accent)
                .fg(theme::get().on_accent)
                .bold();
            chips.push(Span::styled(format!(" {} {glyph} {name} ", i + 1), style));
        } else {
            chips.push(Span::raw(format!(" {} ", i + 1)).fg(theme::get().dim));
            chips.push(Span::raw(glyph).fg(color));
            chips.push(Span::raw(format!(" {name} ")));
        }
        let width: usize = chips[first..].iter().map(Span::width).sum();
        bounds.push((x, width));
        x += width + 1;
        chips.push(Span::raw(" "));
    }

    let others = |wanted: State| {
        sessions
            .iter()
            .enumerate()
            .filter(|&(i, (_, state))| i != active && *state == wanted)
            .count()
    };
    let mut waiting = Vec::new();
    for (state, label) in [(State::WaitingPermission, "waiting"), (State::Idle, "idle")] {
        let count = others(state);
        if count > 0 {
            let (glyph, color) = state_glyph(state);
            waiting.push(Span::raw(format!("  {glyph} {count} {label}")).fg(color));
        }
    }
    waiting.push(Span::raw(" "));
    let waiting = Line::from(waiting);
    let width = u16::try_from(waiting.width()).unwrap_or_default();
    let [chips_area, waiting_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    frame.render_widget(Line::from(chips), chips_area);
    frame.render_widget(waiting, waiting_area);
    bounds
        .into_iter()
        .map(|(x, width)| columns(chips_area, x, width))
        .collect()
}

/// Chat, then the peer tabs, each with whether it has finished. Chat's state shows in the session
/// bar, so only its end shows here.
fn tab_labels(view: &View) -> Vec<(Option<String>, &str, bool)> {
    std::iter::once(&view.claude)
        .chain(&view.tabs)
        .map(|tab| (tab_key(&tab.def.key), tab.def.name.as_str(), tab.finished()))
        .collect()
}

/// The key shown with a tab: its first after the leader, else its first.
fn tab_key(keys: &Bindings) -> Option<String> {
    keys.0
        .iter()
        .find(|b| b.leader)
        .or(keys.0.first())
        .map(|b| b.key.label())
}

/// The tab rule: the tabs over a rule, and the session's directory in the room left on the right.
/// Returns where each tab's title went.
fn render_tab_bar(
    frame: &mut Frame,
    area: Rect,
    labels: &[(Option<String>, &str, bool)],
    selected: usize,
    cwd: &str,
) -> Vec<Rect> {
    let titles = tab_titles(labels);
    let widths: Vec<usize> = titles.iter().map(Line::width).collect();
    let tabs = tab_line(titles, selected);
    // The rule at both ends and between the two, and the padding of both.
    let room = usize::from(area.width).saturating_sub(tabs.width() + 10);
    let count = cwd.chars().count();
    let cwd = if count <= room {
        cwd.to_owned()
    } else if room >= 8 {
        // Its end tells it apart.
        let tail: String = cwd.chars().skip(count + 1 - room).collect();
        format!("…{tail}")
    } else {
        String::new()
    };
    let drawn = render_rule(frame, area, tabs, Line::from(cwd).fg(theme::get().dim));
    let mut x = 0;
    widths
        .into_iter()
        .map(|width| {
            let title = columns(drawn, x, width);
            x += width + TAB_GAP.chars().count();
            title
        })
        .collect()
}

/// `c Chat`, `r review`, `s shell ✗`: the key that picks each tab after the leader, its name,
/// and whether it has finished.
fn tab_titles(labels: &[(Option<String>, &str, bool)]) -> Vec<Line<'static>> {
    labels
        .iter()
        .map(|(key, name, finished)| {
            let (name, finished) = (*name, *finished);
            let mut spans: Vec<Span> = key
                .as_ref()
                .map(|key| Span::raw(format!("{key} ")).fg(theme::get().dim))
                .into_iter()
                .collect();
            spans.push(Span::raw(name.to_owned()));
            if finished {
                spans.push(Span::raw(" ✗").fg(theme::get().error));
            }
            Line::from(spans)
        })
        .collect()
}

/// The tabs on one line, between rules, the one on screen in the accent color.
fn tab_line(titles: Vec<Line<'static>>, selected: usize) -> Line<'static> {
    let highlight = Style::new().fg(theme::get().accent).bold();
    let mut spans = Vec::new();
    for (i, title) in titles.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(TAB_GAP).fg(theme::get().dim));
        }
        if i == selected {
            spans.extend(
                title
                    .spans
                    .into_iter()
                    .map(|span| span.patch_style(highlight)),
            );
        } else {
            spans.extend(title.spans);
        }
    }
    Line::from(spans)
}

fn state_glyph(state: State) -> (&'static str, Color) {
    let theme = theme::get();
    match state {
        State::Starting => ("◌", theme.starting),
        State::Idle => ("○", theme.idle),
        State::Asleep => ("◐", theme.asleep),
        State::Working => ("●", theme.working),
        State::WaitingPermission => ("◆", theme.waiting),
    }
}

/// A dim rule across `area`, with `left` and `right` written over it, a little way in from its
/// ends. Either can be empty; `right` is left out when both don't fit. Returns where `left` went.
fn render_rule(frame: &mut Frame, area: Rect, left: Line, right: Line) -> Rect {
    frame.render_widget(
        Block::new()
            .borders(Borders::TOP)
            .border_style(Style::new().fg(theme::get().dim)),
        area,
    );
    // A space on each side, between the text and the rule.
    let width = |line: &Line| match line.width() {
        0 => 0,
        width => u16::try_from(width + 2).unwrap_or(u16::MAX),
    };
    // Both ends, and the rule between the two.
    let right = if width(&left).saturating_add(width(&right)) + 5 > area.width {
        Line::default()
    } else {
        right
    };
    let [_, left_area, _, right_area, _] = Layout::horizontal([
        Constraint::Length(2),
        Constraint::Length(width(&left)),
        Constraint::Fill(1),
        Constraint::Length(width(&right)),
        Constraint::Length(2),
    ])
    .areas(area);
    for (line, area) in [(left, left_area), (right, right_area)] {
        // Text the line doesn't color would take the rule's.
        frame.render_widget(Clear, area);
        frame.render_widget(line, area.inner(Margin::new(1, 0)));
    }
    left_area.inner(Margin::new(1, 0))
}

/// How to see what the keys after the leader do; brighter while the leader waits for one.
fn leader_hint(app: &App) -> Line<'static> {
    let hint = Line::from(format!("{} actions", app.config.leader.label()));
    if app.leader_pending.is_some() {
        hint.fg(theme::get().leader_pending)
    } else {
        hint.fg(theme::get().dim)
    }
}

/// The keys after the leader and what they do in the tab on screen, a terminal tab if `peer`, as
/// `App::command_for` finds them. `tabs` are the peer tabs' keys and names.
fn leader_actions(
    config: &Config,
    tabs: &[(&Bindings, &str)],
    peer: bool,
) -> Vec<(String, String)> {
    // The keys after the leader in `keys`, in one column.
    let after = |keys: &Bindings| {
        let labels: Vec<String> = keys
            .0
            .iter()
            .filter(|b| b.leader)
            .map(|b| b.key.label())
            .collect();
        labels.join(" ")
    };
    // With their own digits after the leader, the nine sessions take one row.
    let sessions = Action::ALL[..9]
        .iter()
        .all(|&a| after(&config.keys[&a]) == after(&a.default_bindings()));
    let mut rows = Vec::new();
    for action in Action::ALL.into_iter().filter(|a| a.works_in(peer)) {
        let keys = after(&config.keys[&action]);
        match action.session() {
            Some(0) if sessions => rows.push(("1-9".to_owned(), "go to session".to_owned())),
            Some(_) if sessions => {}
            _ if keys.is_empty() => {}
            _ => rows.push((keys, action_text(action))),
        }
        if action == Action::Chat {
            rows.extend(
                tabs.iter()
                    .map(|&(keys, name)| (after(keys), name.to_owned()))
                    .filter(|(keys, _)| !keys.is_empty()),
            );
        }
    }
    rows.push((config.leader.label(), "send to program".to_owned()));
    rows
}

/// What `action` does, in the leader panel.
fn action_text(action: Action) -> String {
    let text = match action {
        Action::PreviousSession => "previous session",
        Action::NextSession => "next session",
        Action::NewSession => "new session",
        Action::NewSessionNoWorktree => "new session, no worktree",
        Action::Find => "find a session",
        Action::Merge => "commit & merge",
        Action::Chat => "Chat",
        Action::PreviousTab => "previous tab",
        Action::NextTab => "next tab",
        Action::RerunTab => "reload tab",
        Action::SubmitAndNewSession => "submit, new session, no worktree",
        Action::SubmitAndStay => "submit, stay",
        Action::Quit => "quit",
        // Numbered below.
        _ => "go to session",
    };
    match action.session() {
        Some(index) => format!("{text} {}", index + 1),
        None => text.to_owned(),
    }
}

/// How to do `action`, for notices: `leader m`, or a key of its own like `A-m`. Its name in the
/// config if it has no key.
pub fn key_hint(config: &Config, action: Action) -> String {
    match config.keys[&action].0.first() {
        Some(b) if b.leader => format!("leader {}", b.key.label()),
        Some(b) => b.key.label(),
        None => action.name().to_owned(),
    }
}

/// The keys after the leader, which-key style, above the status line `status`, once the leader
/// has waited a moment for one.
fn render_leader(frame: &mut Frame, app: &App, status: Rect) {
    if !app.leader_panel {
        return;
    }
    let view = &app.views[app.active];
    let tabs: Vec<(&Bindings, &str)> = view
        .tabs
        .iter()
        .map(|t| (&t.def.key, t.def.name.as_str()))
        .collect();
    let actions = leader_actions(&app.config, &tabs, view.tab > 0);
    let area = frame.area();
    let above = Rect {
        height: status.y - area.y,
        ..area
    };
    render_leader_panel(frame, above, &app.config.leader.label(), &actions);
}

/// `actions` one per row, key ➜ what it does, in a box in the bottom-right corner of `area`.
fn render_leader_panel(frame: &mut Frame, area: Rect, leader: &str, actions: &[(String, String)]) {
    let key_width = actions
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or_default();
    let lines: Vec<Line> = actions
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::raw(format!("{key:>key_width$}")).fg(theme::get().accent),
                Span::raw(" ➜ ").fg(theme::get().dim),
                Span::raw(action.clone()),
            ])
        })
        .collect();
    // The border and a column of padding on each side.
    let width = lines.iter().map(Line::width).max().unwrap_or_default() + 4;
    let width = u16::try_from(width).unwrap_or(u16::MAX).min(area.width);
    let height = u16::try_from(lines.len() + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let panel = Rect::new(area.right() - width, area.bottom() - height, width, height);
    frame.render_widget(Clear, panel);
    frame.render_widget(
        Paragraph::new(lines).block(
            theme::popup(format!(" {leader} "))
                .padding(Padding::horizontal(1))
                .title_bottom(Line::from(" Esc cancel ").fg(theme::get().dim)),
        ),
        panel,
    );
}

/// What the bottom rule says on the left: `notice` (with whether it is good news), else a peer
/// tab's command and how it ended. Only a peer tab is run again (`peer`); Claude's session closes
/// when it ends. The merge tab shows its action's name, and runs again by the key `merge`.
fn rule_status(
    notice: Option<(&str, bool)>,
    tab: &Tab,
    peer: bool,
    merge: Option<&str>,
) -> Line<'static> {
    if let Some((notice, ok)) = notice {
        let color = if ok {
            theme::get().idle
        } else {
            theme::get().error
        };
        return Line::from(notice.to_owned()).fg(color);
    }
    let again = match merge {
        Some(key) => format!("{key} to try again"),
        None => "Enter re-runs".to_owned(),
    };
    let ended = match (&tab.error, &tab.exited) {
        (Some(_), _) if peer => Span::raw("failed to start · Enter retries").fg(theme::get().error),
        (Some(_), _) => Span::raw("claude failed to start").fg(theme::get().error),
        (None, Some(status)) if peer => {
            Span::raw(format!("exited ({status}) · {again}")).fg(theme::get().warning)
        }
        _ => Span::default(),
    };
    let mut spans = Vec::new();
    if merge.is_some() {
        spans.push(Span::raw(format!("merge: {}", tab.def.name)).fg(theme::get().dim));
        if tab.finished() {
            spans.push(Span::raw(" · ").fg(theme::get().dim));
        }
    } else if peer {
        spans.push(Span::raw(tab.def.command.clone()).fg(theme::get().dim));
        if tab.finished() {
            spans.push(Span::raw(" · ").fg(theme::get().dim));
        }
    }
    spans.push(ended);
    Line::from(spans)
}

fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    area.centered(
        Constraint::Percentage(width_percent),
        Constraint::Percentage(height_percent),
    )
}

fn home_relative(path: &Path) -> String {
    match std::env::var_os("HOME").and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned))
    {
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

fn age(now: SystemTime, then: SystemTime) -> String {
    let secs = now.duration_since(then).map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use ccs_core::{
        config::{Mode, TabDef},
        keys::Key,
    };
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn draw(width: u16, height: u16, render: impl FnOnce(&mut Frame)) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(render).unwrap();
        terminal
    }

    #[test]
    fn session_bar_fills_the_active_chip_and_counts_who_waits() {
        let sessions = [
            ("claude-screen".to_owned(), State::Asleep),
            ("auth-refactor".to_owned(), State::Working),
            ("api".to_owned(), State::WaitingPermission),
            ("web".to_owned(), State::Idle),
        ];
        let mut chips = Vec::new();
        let terminal = draw(80, 1, |frame| {
            chips = render_session_bar(frame, frame.area(), &sessions, 1);
        });
        insta::assert_snapshot!(terminal.backend());
        // Clicks find each chip where it is drawn, the cut one too.
        assert_eq!(
            texts(&terminal, &chips),
            [
                " 1 ◐ claude-screen ",
                " 2 ● auth-refactor ",
                " 3 ◆ api ",
                " 4 ○ w"
            ]
        );
        // Chips past the end are nowhere.
        let terminal = draw(30, 1, |frame| {
            chips = render_session_bar(frame, frame.area(), &sessions, 1);
        });
        assert_eq!(texts(&terminal, &chips), [" 1 ◐ c", "", "", ""]);
    }

    /// The text drawn in each of `areas`.
    fn texts(terminal: &Terminal<TestBackend>, areas: &[Rect]) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        areas
            .iter()
            .map(|area| area.positions().map(|at| buffer[at].symbol()).collect())
            .collect()
    }

    #[test]
    fn finder_with_open_and_past_sessions() {
        use std::{path::PathBuf, time::Duration};

        use ccs_claude::history::PastSession;

        use crate::finder::{Item, OldSession};

        let mut finder = Finder::new(PathBuf::from("/work/web"), &[]);
        finder.old = vec![OldSession {
            past: PastSession {
                id: "s2".into(),
                path: PathBuf::new(),
                title: "Add rate limiting".into(),
                name: None,
                cwd: None,
                modified: SystemTime::now() - Duration::from_hours(3),
            },
            prompts: vec!["limit requests per user".into()],
            merged: Some(true),
        }];
        finder.loading = false;
        finder.items = vec![
            Item {
                target: Target::View(0),
                title: "Fix the login bug".into(),
                prompts: vec!["the login form rejects valid user names".into()],
            },
            Item {
                target: Target::Old(0),
                title: "Add rate limiting".into(),
                prompts: finder.old[0].prompts.clone(),
            },
        ];
        finder.query.insert("user");
        finder.refresh();

        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal
            .draw(|frame| render_finder(frame, &mut finder, &[State::Working]))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn tab_bar_shows_keys_and_states() {
        let labels = [
            (Some("c".to_owned()), "Chat", false),
            (Some("r".to_owned()), "review", true),
            (Some("s".to_owned()), "shell", false),
            (None, "git", false),
        ];
        let mut titles = Vec::new();
        let terminal = draw(60, 1, |frame| {
            titles = render_tab_bar(frame, frame.area(), &labels, 2, "~/work/web");
        });
        insta::assert_snapshot!(terminal.backend());
        // Clicks find each title where it is drawn.
        assert_eq!(
            texts(&terminal, &titles),
            ["c Chat", "r review ✗", "s shell", "git"]
        );
        // Names keep their own color over the dim rule, started or not; the tab on screen is in
        // the accent.
        let buffer = terminal.backend().buffer();
        assert_eq!(
            (buffer[(0, 0)].fg, buffer[(5, 0)].fg),
            (theme::get().dim, Color::Reset)
        );
        assert_eq!(buffer[(38, 0)].symbol(), "g");
        assert_eq!(buffer[(38, 0)].fg, Color::Reset);
        assert_eq!(buffer[(29, 0)].symbol(), "s");
        assert_eq!(buffer[(29, 0)].fg, theme::get().accent);

        // A long directory keeps its end, in the room the tabs leave.
        let terminal = draw(60, 1, |frame| {
            let cwd = "/tmp/claude-1000/scratchpad/projects/claude-screen";
            render_tab_bar(frame, frame.area(), &labels, 2, cwd);
        });
        assert_eq!(
            terminal.backend().to_string(),
            "\"── c Chat ── r review ✗ ── s shell ── git ── …aude-screen ──\"\n"
        );
    }

    fn tab(name: &str, command: &str) -> Tab {
        Tab {
            def: TabDef {
                name: name.into(),
                key: Bindings::default(),
                mode: Mode::Lazy,
                command: command.into(),
            },
            pty: None,
            key: 0,
            exited: None,
            error: None,
        }
    }

    #[test]
    fn bottom_rule_says_how_a_tab_ended() {
        let mut review = tab("review", "git diff");
        assert_eq!(
            rule_status(None, &review, true, None).to_string(),
            "git diff"
        );
        review.exited = Some("exit 1".into());
        let rule = |width| {
            let terminal = draw(width, 1, |frame| {
                let left = rule_status(None, &review, true, None);
                render_rule(frame, frame.area(), left, Line::from("ctrl-space actions"));
            });
            terminal.backend().to_string()
        };
        insta::assert_snapshot!(rule(70));
        // Too narrow for both: how the tab ended comes first.
        assert_eq!(rule(40), "\"── git diff · exited (exit 1) · Enter ──\"\n");

        // Claude says nothing while it runs.
        let claude = tab("Chat", "claude");
        assert_eq!(rule_status(None, &claude, false, None).to_string(), "");
        // A notice comes first, in red unless it is good news.
        let notice = rule_status(Some(("resuming s1 failed", false)), &review, true, None);
        assert_eq!(notice.to_string(), "resuming s1 failed");
        assert_eq!(notice.style.fg, Some(theme::get().error));
        let merged = rule_status(Some(("merged api into main", true)), &review, true, None);
        assert_eq!(merged.style.fg, Some(theme::get().idle));

        // The merge tab says which action ran, and isn't run again by Enter.
        let mut merge = tab("pull request", "git push && gh pr create --fill");
        assert_eq!(
            rule_status(None, &merge, true, Some("leader m")).to_string(),
            "merge: pull request"
        );
        merge.exited = Some("exit 1".into());
        assert_eq!(
            rule_status(None, &merge, true, Some("leader m")).to_string(),
            "merge: pull request · exited (exit 1) · leader m to try again"
        );
    }

    #[test]
    fn merge_panel_shows_what_it_takes() {
        let mut panel = MergePanel {
            view: 0,
            branch: "worktree-ccs-1".into(),
            target: Some("main".into()),
            changes: vec![" M src/app.rs".into(), "?? docs/merge.md".into()],
            ahead: 2,
            others: 1,
            message: crate::input::Input::default(),
            action: 1,
            needs_message: true,
        };
        panel.message.insert("Add the merge action");
        let mut terminal = Terminal::new(TestBackend::new(70, 18)).unwrap();
        terminal
            .draw(|frame| render_merge(frame, &panel, &["merge", "pull request"]))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());

        // Only commits to merge: no message to type. One action: none to pick.
        panel.changes.clear();
        panel.needs_message = false;
        panel.action = 0;
        let text = |panel: &MergePanel| {
            let terminal = draw(70, 18, |frame| render_merge(frame, panel, &["merge"]));
            terminal.backend().to_string()
        };
        let only_commits = text(&panel);
        assert!(only_commits.contains("2 commits"), "{only_commits}");
        assert!(!only_commits.contains("Commit message"), "{only_commits}");
        assert!(!only_commits.contains("Action"), "{only_commits}");
        // A session on the target branch itself only commits.
        panel.branch = "main".into();
        panel.changes = vec![" M src/app.rs".into()];
        assert!(text(&panel).contains("Commit · main"));
    }

    #[test]
    fn leader_panel_lists_the_keys_after_the_leader() {
        let config = Config::default();
        let (review, shell) = (
            Bindings::after_leader(Key::char('r')),
            Bindings::after_leader(Key::char('s')),
        );
        let actions = leader_actions(&config, &[(&review, "review"), (&shell, "shell")], true);
        let mut terminal = Terminal::new(TestBackend::new(44, 16)).unwrap();
        terminal
            .draw(|frame| render_leader_panel(frame, frame.area(), "C-Space", &actions))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());

        // Reloading is for peer tabs; in Chat, Enter and w submit. Claude gets the leader too.
        let action = |actions: &[(String, String)], key: &str| {
            actions
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, action)| action.clone())
        };
        let chat = leader_actions(&config, &[], false);
        assert!(action(&chat, "Enter").is_some_and(|a| a.starts_with("submit")));
        assert!(action(&chat, "w").is_some());
        assert!(action(&actions, "w").is_none());
        assert!(action(&chat, "C-Space").is_some());

        // Keys of their own: the sessions one by one, each of the keys after the leader, none
        // for keys without it.
        let config = Config::parse(
            "[keys]\nsession_2 = [\"<leader>2\", \"<A-2>\"]\nsession_3 = \"<A-3>\"\n\
             find = [\"<leader>f\", \"<leader>/\", \"<C-f>\"]\nquit = \"<C-q>\"",
        )
        .unwrap();
        let chat = leader_actions(&config, &[], false);
        assert_eq!(action(&chat, "1").as_deref(), Some("go to session 1"));
        assert_eq!(action(&chat, "2").as_deref(), Some("go to session 2"));
        assert!(
            !chat
                .iter()
                .any(|(_, a)| a == "go to session 3" || a == "quit")
        );
        assert_eq!(action(&chat, "f /").as_deref(), Some("find a session"));
        assert_eq!(key_hint(&config, Action::Quit), "C-q");
        assert_eq!(key_hint(&config, Action::Merge), "leader m");
        // Keys without the leader aren't listed: the digits after it still take one row.
        let config = Config::parse("[keys]\nsession_2 = [\"<leader>2\", \"<A-2>\"]").unwrap();
        let chat = leader_actions(&config, &[], false);
        assert_eq!(action(&chat, "1-9").as_deref(), Some("go to session"));
    }

    #[test]
    fn the_new_screen_pushes_the_old_one_off() {
        let old = Buffer::with_lines(["abcd"]);
        let slid = |new: &Buffer, moved, forward| {
            let mut out = Buffer::empty(old.area);
            slide(&old, new, old.area, moved, forward, &mut out);
            out
        };
        let new = Buffer::with_lines(["wxyz"]);
        assert_eq!(slid(&new, 0, true), old);
        assert_eq!(slid(&new, 2, true), Buffer::with_lines(["cdwx"]));
        assert_eq!(slid(&new, 4, true), new);
        assert_eq!(slid(&new, 2, false), Buffer::with_lines(["yzab"]));
        // A wide character cut at the right edge is left out.
        let new = Buffer::with_lines(["ab漢"]);
        assert_eq!(slid(&new, 3, true), Buffer::with_lines(["dab "]));
    }

    #[test]
    fn terminal_tabs_fill_the_window_between_the_bars() {
        assert_eq!(terminal_size(Rect::new(0, 0, 80, 24)), (21, 80));
        assert_eq!(terminal_size(Rect::new(0, 0, 10, 2)), (1, 10));
    }
}
