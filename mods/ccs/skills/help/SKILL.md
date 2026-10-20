---
name: help
description: This Claude Code session runs inside ccs (claude-screen), a terminal workspace for many Claude sessions, each with its own tabs. Use when the user asks how to use ccs, or about its sessions, tabs, the leader key (Ctrl-Space), switching or finding sessions, submit and advance, new sessions, worktrees, merging a session (leader m), themes, ~/.config/ccs/config.toml, or a project's .ccs/config.toml.
---

# Explaining ccs

You run inside ccs (claude-screen). Use this to answer the user's questions about ccs. The keys below are the defaults. The user's config can change any of them, so when a key matters, call it the default and tell the user how to see their own: press the leader and wait, and a panel lists the keys that work in the tab on screen. Don't offer what "Not there yet" lists.

## What ccs is

ccs runs many Claude Code sessions in one terminal. Each session has a Chat tab, which is this Claude, plus terminal tabs: by default a `review` tab (`git diff`) and a `shell` tab (`$SHELL`). Claude draws its own screen in the Chat tab, so all of Claude Code works as usual.

- **The top row** holds a chip for each session, with its state:
  - ◌ starting
  - ○ idle, waiting for a prompt
  - ◐ asleep, waiting on background work or a scheduled wakeup
  - ● working
  - ◆ waiting for the user, on a permission dialog
- **The second row** holds the session's tabs, each with its key, and the directory on the right.
- **The bottom rule** shows notices on the left and the leader hint on the right.
- **Ending a session.** `/exit` or Ctrl-C twice in Claude closes the session and its tabs. After the last session, ccs exits.

## Submit and advance

Submitting a prompt with Enter takes the user to the session that needs them most. That is a session waiting on a permission dialog first, then an idle one, then one still starting, nearest first. When no session is waiting, it opens a new session in the same project. Allowing a tool in Claude's permission dialog advances too, and denying one stays. ccs only switches right after the user's own key, in the session on screen, never in the background. Asleep (◐) and working sessions are skipped.

- **leader w** submits and stays.
- **leader Enter**, in the Chat tab, submits and then always opens a new session in the main checkout, without a worktree, even if another session is waiting.

## Keys

Without the leader:

- Any key in the Chat tab goes to Claude, Ctrl-C included.
- Any key in a terminal tab goes to its program, Ctrl-C included.
- Enter, in a tab whose command exited, runs the command again.
- The mouse wheel scrolls Claude's transcript or a tab's output. With two tabs side by side, it scrolls the one under the pointer.
- A click on a session or a tab goes to it. A click on one of two tabs side by side sends the keys there.
- A sideways scroll (touchpad swipe), or Shift and the wheel, goes to the previous or next session, one per swipe.

The leader is Ctrl-Space. Press it, then one key:

| Leader, then | Action |
|---|---|
| (wait) | A panel lists the keys that can follow; Esc closes it |
| `1`–`9` | Go to session 1–9 |
| `,` / `.` | Previous / next session |
| `n` | New session in this project, in a worktree of its own |
| `N` | New session in the main checkout, without a worktree |
| `f` | Find a session, open or past |
| `m` | Commit and merge the session's work |
| `c` | Go to the Chat tab, on its own |
| a tab's key (`r` review, `s` shell by default) | Go to that tab, on its own |
| `C` / a tab's split key (`R` review, `S` shell by default) | Show Chat / that tab beside the one on screen, with the keys. Chat goes on the left. With two on screen, it takes the place of the one on the right, or goes if it's one of them. On screen alone, Chat comes beside it |
| `o` | With two tabs side by side: send the keys to the other one |
| `[` / `]` | Previous / next tab |
| Enter | In Chat: submit, then open a new session without a worktree. In a terminal tab: run its command again, stopping it first if it still runs |
| `w` | In Chat: submit and stay |
| `q` | Quit ccs. While a merge runs, press `q` again |
| Ctrl-Space | Send Ctrl-Space itself to the program in the tab, Claude included |

Rebinding goes under `[keys]` in the config. Each action (`session_1` … `session_9`, `previous_session`, `next_session`, `new_session`, `new_session_no_worktree`, `find`, `merge`, `chat`, `chat_split`, `other_pane`, `previous_tab`, `next_tab`, `rerun_tab`, `submit_and_new_session`, `submit_and_stay`, `quit`) takes one key or a list. Keys are written the way vim writes them:

- `"<leader>f"` is f after the leader.
- `"<C-f>"` and `"<A-1>"` work on their own, without the leader. A key without the leader needs Ctrl or Alt, because the other keys are typing.
- Shift is in the letter's case (`"<leader>N"`).
- `<Space>`, `<CR>` (Enter) and `<lt>` (<) name those keys.
- `[]` leaves an action without a key.

For example, `session_1 = ["<leader>1", "<A-1>"]` adds Alt-1 and keeps the leader's way. The leader itself is `leader = "<C-Space>"`.

## Sessions

- **New sessions.** By default each new session works in its own git worktree and branch, made by `claude --worktree` (`.claude/worktrees/ccs-<id>`, branch `worktree-ccs-<id>`), so sessions don't change each other's files. Its tabs run there too. Outside git, or before the first commit, a session gets no worktree. `worktree = false` turns worktrees off.
- **Plan mode.** New sessions start in Claude's plan mode, so Claude plans and asks before it changes anything. Shift+Tab changes the mode as usual. `plan = false` turns this off. Resumed sessions keep the user's own default mode.
- **Names.** `/rename <name>` in Claude names the session. The session bar and the finder show the name. Otherwise they show Claude's title for the session.
- **The finder** (leader f) searches names, titles, project names and the user's own prompts. It covers the sessions open in ccs and the past sessions from this directory.
  - Enter goes to a session, and resumes it if it's a past one. Shift-Tab switches between open sessions only and both.
  - A past session from a worktree says `merged` once its branch was merged into the main checkout's branch. It says `not merged` while its branch has commits that branch doesn't.
- **Resuming from the shell.** `ccs --resume <session id>` resumes a past Claude Code session in its directory. `ccs [DIR]` starts ccs on a directory.
- **Session switches** slide the new session in from the side of its chip. `slide_ms` sets the speed, and `slide_ms = 0` switches at once.

## Tabs

Tabs come from `[[tab]]` entries in the config. Each has a `name`, a `key`, an optional `split_key`, a `command` (run with `sh -c` in the session's directory) and a `mode`:

- `eager` runs every time the tab comes on screen. The default `review` tab is eager.
- `lazy`, the default mode, runs the first time the tab is opened.
- `regular` starts in the background with the session, so it's up before the tab is opened. The default `shell` tab is regular.

Commands get these variables, each empty when unknown:

- `CS_CWD` is the session's directory (its worktree).
- `CS_REPO` is where the session started, the main checkout.
- `CS_BRANCH` is the branch in `CS_CWD`.
- `CS_REPO_BRANCH` is the branch in `CS_REPO`, which leader m merges into.
- `CS_GIT_BASE` is the commit the worktree started from.
- `CS_SESSION_ID` is Claude's session id.

For example, a tab running the tests:

```toml
[[tab]]
name = "tests"
key = "<leader>t"
command = "cargo test"
```

Any `[[tab]]` in the user's config replaces the default `review` and `shell` tabs, so a user who adds one and wants those back lists them too. A project's `[[tab]]` adds to them instead (see Configuration).

## Merging a session (leader m)

Claude Code keeps a session in a worktree out of the main checkout: your git commands can't reach it, on purpose. So when the user wants the work in their main branch, tell them to press leader m rather than trying it yourself.

1. A panel shows the branch, where it goes (the branch checked out in the main checkout), its commits, and the uncommitted files. When there are some, it asks for a commit message, prefilled with the session's title. Ctrl-J (or Shift-Enter where the terminal reports it) adds a new line. Tab picks between several `[[merge]]` actions. Enter runs the action, and Esc cancels.
2. The action runs in a tab of the session, so passphrases and other prompts work. First its `commit` runs, by default `git add -A && git commit -m "$CS_MESSAGE"`, when there's something to commit. Then its `command` runs. The default merges the session's branch in the main checkout, if that has no uncommitted changes. A session without a worktree, working on that branch itself, only commits.
3. When the command exits 0, the session closes and its worktree goes, if nothing in it would be lost. After the last session, a new one opens.
4. When it stops on conflicts, the action's `on_conflict` prompt goes to a Claude that can work where the conflicts are. Conflicts in the main checkout go to a new session there, without a worktree. Once that session commits the merge, it closes along with the merged session. Conflicts in the session's own worktree, as with a rebase-first action, go to the session's own Claude. Leader m runs again once they're fixed.

Leader m does nothing while Claude is working, waiting for the user, or asleep, nor when there's nothing to merge. Only one merge runs in a main checkout at a time. A failed action leaves its tab open with a notice, and leader m runs it again. `[[merge]]` entries in the config replace the default, for example to push, open a pull request with `gh pr create`, or rebase first.

## Themes

`theme = "<name>"` colors Claude, ccs and every tab with one theme. The built-in themes are `gruvbox-dark`, `gruvbox-light`, `tokyonight-night`, `tokyonight-day`, `github-dark` and `one-dark`. A file `~/.config/ccs/themes/<name>.json` adds a theme, or replaces a built-in one of that name. Any Claude Code theme file works. An optional `ccs` section can set `foreground`, `background`, the 16-color `palette`, and ccs's own colors. Without a theme, Claude keeps the user's own Claude theme and the tabs keep the terminal's colors. `/theme` in a session lists the ccs theme as "from ccs-theme".

## Configuration

The user's config lives in `~/.config/ccs/config.toml`. ccs reads it when it starts, so a change takes a restart of ccs. Top-level options:

- `leader`, default `"<C-Space>"`.
- `theme`, unset by default.
- `slide_ms = 150`.
- `worktree = true`.
- `plan = true`.
- `[keys]`, `[[tab]]` and `[[merge]]`, as above.

A project can change any of it for its own sessions with `.ccs/config.toml` in its root: the main checkout, also when ccs starts in a worktree or a subdirectory. It wins over the user's config and lists only what it changes:

- An option it sets replaces the user's. Each action under its `[keys]` takes its keys instead of the user's.
- A `[[tab]]` or `[[merge]]` with the name of one of the user's changes only the fields it lists, so this changes the review tab's command and keeps its keys and mode:

  ```toml
  [[tab]]
  name = "review"
  command = "git diff --stat"
  ```

- A new name adds a tab or action after the user's. It needs a `command`.
- `disabled_tabs = ["shell"]` takes tabs away. A name the user doesn't have is skipped, and a disabled tab's keys are free for the project's tabs.

The project file is read when ccs starts too. ccs doesn't ask before using it, and its `regular` tabs run as soon as a session starts.

ccs rejects unknown fields and clashing keys when it starts, and says which file.

## Not there yet

- Tab parameters asked for when a tab opens (`params`, a changed-files picker).
- The tab modes `on_demand`, `on_change` (rerun when Claude edits files) and `on_idle`.

ccs rejects these fields in the config.
