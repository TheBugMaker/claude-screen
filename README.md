# claude-screen (`ccs`)

A terminal workspace for running and navigating many Claude Code sessions.

You type a prompt, submit it, and `ccs` moves you to the next session that is waiting for you, or opens a new one. Every session comes with tabs like iTerm2 Workgroups: a diff, a shell, a test runner, a reviewer. A side panel shows what every session is doing.

> **Status:** early. Milestones 0–3 are implemented: Claude sessions you can switch between, submit-and-advance, and a fuzzy session finder. Milestone 4 is under way: every session has terminal tabs next to its Chat tab.

## How it works

```
ccs (ratatui)
├── Session panel           working / waiting / idle / asleep, sorted by what needs you
└── Session "auth-refactor" [1 Chat] [2 diff] [3 shell] [4 review]
    ├── Chat tab   → the interactive `claude --plugin-dir mods`, in a pseudo-terminal
    │                the ccs-bridge mod reports what it does: `ccs hook` → Unix socket → ccs
    └── Peer tabs  → commands in a pseudo-terminal
                     portable-pty → vt100 (in-memory screen) → tui-term widget
```

Claude Code draws its own screen in the Chat tab, so all of its features work as usual. The ccs-bridge mod (`mods/ccs-bridge`) tells ccs when a session is ready, works, shows a permission dialog, is answered and goes idle, when it waits on background work or a scheduled wakeup to go on (asleep, ◐, which submit-and-advance skips like a working one), and when it is renamed. Claude titles a session started with `/plan <prompt>` only once the plan is approved, so until then the mod has Haiku title it from the prompt's first 120 characters. The ccs-traces mod (`mods/ccs-traces`) draws each run of tool calls (the calls Claude makes between two of its texts) as one line, `● 5 tool calls · Read 3 · Bash 2`: click it to list the calls, click a call to see its whole input and output, and click it, or anything it shows, again to fold it. It draws only in Claude's fullscreen mode (`"tui": "fullscreen"`), the one that takes clicks. Mods are an early-access Claude Code feature (2.1.301 and later), and their API can change between releases.

| Directory | Role |
|---|---|
| `crates/ccs` | The `ccs` binary: CLI, terminal setup, event loop, UI |
| `crates/ccs-core` | Session/group model, `[[tab]]` config, file paths |
| `crates/ccs-claude` | Session state from the ccs-bridge mod, and past sessions in `~/.claude/projects` |
| `crates/ccs-term` | PTY-backed tabs, key and mouse encoding |
| `mods/ccs-bridge` | The Claude Code mod that reports each session's state to ccs |
| `mods/ccs-traces` | The Claude Code mod that folds each run of tool calls into one line you can click open |
| `themes` | The built-in themes |

## Use cases

### M0: Bootstrap
- [x] Cargo workspace, pinned toolchain, lints, Makefile, cargo-deny, licenses
- [x] `ccs` opens an empty screen and quits cleanly; logs go to a file

### M1: Claude sessions
- [x] Start a session in a directory: the interactive `claude` in a pseudo-terminal, with the ccs-bridge mod
- [x] Track state from the mod's reports: starting, working, waiting for permission, idle, asleep on background work
- [x] Everything else is Claude Code's own: replies, tool calls, permission dialogs, interrupts, slash commands, permission modes, plans
- [x] New sessions start in Claude's plan mode; `plan = false` turns it off
- [x] Resume a session (`--resume`), and past sessions from `~/.claude/projects`
- [x] Exiting Claude closes its session and its tabs; ccs exits after the last one

### M2: Submit and advance
- [x] Submitting a prompt moves to a session that is waiting for a user answer or a prompt
- [x] If none is waiting, open a new session in the same project
- [x] Never switch while you are typing: only right after your own key, in the session on screen
- [x] Allowing a tool in Claude's permission dialog also advances; denying stays
- [x] leader w submits and stays; leader Enter submits and opens a new session without a worktree, even if another one is waiting
- [x] leader n / N opens a new session, in a worktree of its own / without one

### M3: Session panel and navigation
- [x] leader f opens fuzzy finder panel to navigate between sessions
- [x] fuzzy search searches your own session prompts and session name
- [x] indicate active sessions (currently open in claude screen) and old sessions
- [x] shift-tab on the search pannel to cycle through searching only active sessions or both
- [x] `/rename` in Claude names a session; the session bar and the finder show it

### M4: Workgroup tabs
- [x] Load `[[tab]]` definitions from `~/.config/ccs/config.toml` (see [`config/example.toml`](config/example.toml))
- [x] Peer tabs running any command in a pseudo-terminal (`git diff`, `$SHELL`, `lazygit`, …)
- [x] `CS_*` environment variables in every tab: `CS_CWD`, `CS_REPO`, `CS_BRANCH`, `CS_REPO_BRANCH`, `CS_GIT_BASE`, `CS_SESSION_ID`
- [ ] Parameters as `CS_*` environment variables, asked for by prompt or picker (changed files)
- [x] Each session in its own git worktree (`claude --worktree`), its tabs there too; `worktree = false` turns it off
- [x] Tab modes: `eager`, `lazy`, `regular`
- [ ] Tab modes: `on_demand`, `on_change` (re-run when Claude edits files), `on_idle`
- [x] Leader key, then `c` / the tab's own key / `[` `]` to switch tabs
- [x] Every key in the config, written as vim writes them: after the leader (`<leader>f`) or on its own with Ctrl or Alt (`<C-f>`, `<A-1>`), several for one action
- [x] Resize tabs with the window, including hidden ones
- [x] Review tab: runs a preconfigured command, by default `git diff`
- [x] Terminal tab: opens a shell in the project directory, by default `$SHELL`, configurable

### Merge
- [x] leader m commits a session's changes, with a message you type, and merges its branch into the main checkout's branch
- [x] `[[merge]]` actions in the config, any shell command: merge, push, open a pull request, rebase; Tab picks one
- [x] The action runs in a tab of the session, so passphrases and other prompts work; a successful one closes the session and removes its worktree
- [x] Conflicts go to a Claude that can work where they are: a new merge session in the main checkout, whose fix you review and commit with leader m, or the session's own Claude in its worktree

### Themes
- [x] One theme file colors Claude, ccs, the mods and every tab
- [x] Built in: gruvbox and Tokyo Night (dark and light), GitHub Dark, One Dark
- [x] Your own themes, or other people's, in `~/.config/ccs/themes`

## Usage

```sh
ccs [DIR]                 # start a Claude session in DIR (default: current directory)
ccs --resume <SESSION_ID> # resume a past Claude Code session in its directory
```

| Key | Action |
|---|---|
| Any key in the Chat tab | Goes to Claude, Ctrl-C included. Submitting a prompt, or allowing a tool, takes you to the session that needs you most (or a new one). To stay, or to go to a new session without a worktree, submit with leader w or leader Enter instead |
| `/exit`, or Ctrl-C twice, in Claude | Closes the session and its tabs; after the last one, ccs exits. If Claude failed, ccs says how |
| Any key in a terminal tab | Goes to the program, Ctrl-C included |
| Enter (in a tab whose command exited) | Run it again |
| Mouse wheel | Scroll Claude's transcript, or a terminal tab's output |
| Click a session or a tab | Go to it |
| Scroll sideways (touchpad swipe), or Shift+scroll | Go to the previous / next session, one per swipe |

Everything else starts with the leader, Ctrl-Space, then one key. These are the defaults: the config changes any of them, and can give each a Ctrl or Alt key that works without the leader (see below).

| Leader, then | Action |
|---|---|
| (wait) | A panel lists the keys that can follow and what they do; Esc closes it |
| `1`–`9` | Go to session 1–9 |
| `,` / `.` | Previous / next session |
| `n` / `N` | Open a new session in this project: in a worktree of its own (unless `worktree = false`) / in the main checkout, without a worktree |
| `f` | Find a session: fuzzy search over names, titles, project names and your prompts, in open sessions and past ones from this directory. A past session from a worktree says `merged` once the reflog of the main checkout's branch shows the session's branch merged in, and `not merged` while the session's branch has commits that one doesn't (merges made in this clone only, as far back as the reflog goes). Enter goes to it (resuming a past one), Shift-Tab switches between open sessions only and both |
| `m` | Commit and merge the session's work: see [Merging a session](#merging-a-session) |
| `c` / a tab's key | Go to the Chat tab / that tab |
| `[` / `]` | Previous / next tab |
| Enter | In the Chat tab: submit the prompt, then open a new session in the main checkout, without a worktree, even if another one is waiting. In a terminal tab: run the tab's command again, stopping it first if it is still running |
| `w` | In the Chat tab: submit the prompt and stay |
| `q` | Quit. While a merge runs, it takes `q` again |
| Ctrl-Space | Send Ctrl-Space to the program in the tab, Claude included |

Tabs come from `~/.config/ccs/config.toml` (see [`config/example.toml`](config/example.toml)); without one, every session gets a `review` tab (`git diff`), run again every time you open it, and a `shell` tab (`$SHELL`), started in the background with the session. Tab commands run in the session's directory and get `CS_*` variables such as `$CS_CWD` and `$CS_GIT_BASE` (listed in the example). Each new session works in its own git worktree and branch, made by `claude --worktree`, and its tabs run there; `worktree = false` turns that off. New sessions start in Claude's plan mode (`claude --permission-mode plan`); `plan = false` turns that off. Keys are written as vim writes them, starting with the leader: `leader = "<C-a>"`. Under `[keys]`, each action (`find`, `quit`, `session_1`, … all listed in the example) takes one key or a list of them, after the leader (`"<leader>f"`) or on its own with Ctrl or Alt (`"<C-f>"`, `"<A-1>"`); a tab's `key` takes the same. A key without the leader goes to ccs rather than to Claude or the program in the tab, so `session_1 = ["<leader>1", "<A-1>"]` keeps the leader's way and adds Alt-1. Switching sessions slides the new one in from the side its chip is on; `slide_ms = 150` sets how long it takes per 100 columns, so every width slides at the same speed, and `slide_ms = 0` switches at once.

Each session is a plain `claude` with your own settings and plugins; ccs only adds its mods, and starts a new session in plan mode. Resumed sessions keep your own permission mode. ccs loads them from this repository's `mods`, so a ccs binary needs its source tree for now. Start ccs from your own terminal rather than from inside a Claude Code session, whose `CLAUDE_CODE_*` variables would reach the sessions.

## Merging a session

Claude Code keeps a session that works in a worktree out of the main checkout: its git commands can't reach it. ccs isn't the session, so it can do this step for you. Press the leader, then `m`:

1. A panel shows the branch and where it goes (the branch checked out in the main checkout), its commits, and the files not committed yet. When there are some, it asks for a commit message, prefilled with the session's title (Ctrl-J, or Shift-Enter where your terminal reports it, for a new line). With several `[[merge]]` actions, Tab picks one. Enter runs it, Esc cancels.
2. The action runs in a tab of the session: first its `commit` (by default `git add -A && git commit -m "$CS_MESSAGE"`) when there is something to commit, then its `command`. The default command merges the session's branch in the main checkout, if that has no uncommitted changes. A session that works on that branch itself, without a worktree, only commits.
3. When the command exits 0, the session closes and its worktree goes, if nothing in it would be lost. After the last session, a new one opens.
4. When it stops on conflicts, a Claude that can work where they are gets the action's `on_conflict` prompt.
   - **Conflicts in the main checkout** go to a new **merge session** there, without a worktree.
     - Its Claude resolves the conflicts and stages them, but doesn't commit.
     - You review the result there (its tabs run in the main checkout) and edit what you like. Leader `m` in the merge session then only finishes the merge: it runs `git commit -a --no-edit`, which takes your edits to tracked files too, but not untracked files. It refuses while files are still in conflict.
     - Once the merge is committed, the merge session closes along with the session merged. If the merge is aborted instead, both stay.
   - **Conflicts in the session's worktree** go to the session's own Claude. These come from an action that merges or rebases onto the main checkout's branch first. You press leader `m` again once it's done.
   - While Claude fixes the conflicts, that session counts as busy, and so does the session merged. If you were on the session merged, you go on to the session that needs you most, or to a new one. Once the fixing Claude waits for you, submitting elsewhere takes you to it before any other session.

Leader `m` does nothing while Claude is working, waiting for you, or waiting on background work, nor when there is nothing to merge. Only one merge runs in a main checkout at a time. Any other action fails with a notice and leaves its tab open; leader `m` runs it again. `[[merge]]` entries in the config replace the default; [`config/example.toml`](config/example.toml) has the default and examples that push, open a pull request, or rebase.

## Themes

Set `theme = "gruvbox-dark"` in `~/.config/ccs/config.toml`. The built-in themes are `gruvbox-dark`, `gruvbox-light`, `tokyonight-night`, `tokyonight-day`, `github-dark` and `one-dark` (in [`themes`](themes)). Without a theme, Claude draws in your own Claude theme and the tabs in your terminal's colors.

A theme is a [Claude Code theme file](themes/gruvbox-dark.json) with a `ccs` section:

```json
{
  "name": "Gruvbox Dark",
  "base": "dark",
  "overrides": { "claude": "#fe8019", "promptBorder": "#7c6f64" },
  "ccs": {
    "foreground": "#ebdbb2",
    "background": "#282828",
    "palette": ["#282828", "#cc241d", "…16 colors…"]
  }
}
```

- `base` and `overrides` are Claude's: a base theme (`dark`, `light`, `dark-daltonized`, `light-daltonized`, `dark-ansi`, `light-ansi`) and the colors that replace its own, by the keys of the base theme (`claude`, `promptBorder`, `permission`, `success`, `diffAdded`, …). Claude skips a key its base theme doesn't have.
- `ccs.foreground`, `ccs.background` and `ccs.palette` (the 16 ANSI colors, 0–7 then their bright versions) are drawn in place of your terminal's own colors in every tab, Claude's included, and in ccs's own bars and popups. Claude draws its body text in the terminal's default color, so `foreground` is what colors it. Each one is optional.
- The other `ccs` keys color ccs itself. Each one falls back to the Claude key that means the same, so a plain Claude theme colors ccs too:

  | `ccs` key | What it colors | Falls back to |
  |---|---|---|
  | `accent` | The session and the tab on screen, popup titles, the keys after the leader | `claude` |
  | `onAccent` | Text on `accent` | `inverseText` |
  | `dim` | Rules, borders, hints | `inactive` |
  | `selection` | The session picked in the finder | `userMessageBackground` |
  | `starting`, `idle`, `asleep`, `working`, `waiting` | The state of each session: ◌ ○ ◐ ● ◆ | `inactive`, `success`, `warning`, `warning`, `permission` |
  | `error` | Notices, and what failed | `error` |
  | `warning` | A tab whose command exited | `warning` |
  | `leaderPending` | The leader hint, while the leader waits for a key | `suggestion` |

- Colors take Claude's forms: `#rrggbb`, `#rgb`, `rgb(r,g,b)`, `ansi256(n)` and `ansi:<name>` (`ansi:red`, `ansi:redBright`, …).

To add a theme, or change a built-in one, put a file in `~/.config/ccs/themes/<name>.json` and set `theme = "<name>"`. A file there takes the place of a built-in theme with the same name. Any Claude Code theme file works as it is. ccs checks the whole file when it starts, including the colors Claude would skip without a word. ccs gives the theme to its Claude sessions only, through a plugin it writes for the run (`ccs-<pid>-theme`, next to its socket), so Claude's own theme setting stays as it is. `/theme` in a session lists the theme as "from ccs-theme".

Mods paint with Claude's theme keys (`<Text color="error">`, `dimColor`), so they follow the theme: its overrides set their colors, and their plain text takes `ccs.foreground`. A mod should use theme keys rather than raw colors. A theme can't add keys of its own for a mod, because Claude only takes the keys of its base theme.

## Platforms

Linux (X11 or Wayland, any terminal) and macOS.

## Development

Requires [rustup](https://rustup.rs). The toolchain version is pinned in `rust-toolchain.toml` and installed automatically.

```sh
make help    # list all commands
make run     # run ccs (flags via ARGS="--log-level debug")
make fmt     # format
make check   # format check, clippy -D warnings, tests, cargo-deny
make log     # follow the log file
```

`make deny` needs `cargo install cargo-deny`.

Logs are written to `~/.local/state/ccs/ccs.log`. Set the level with `--log-level` or `CCS_LOG` (for example `CCS_LOG=ccs=debug` logs every report of the ccs-bridge mods).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
