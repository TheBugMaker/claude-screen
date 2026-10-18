# claude-screen (`ccs`)

A terminal workspace for running and navigating many Claude Code sessions.

You type a prompt, submit it, and `ccs` moves you to the next session that is waiting for you, or opens a new one. Every session comes with tabs like iTerm2 Workgroups: a diff, a shell, a test runner, a reviewer. A side panel shows what every session is doing.

> **Status:** early. Milestones 0–3 are implemented: Claude sessions you can switch between, submit-and-advance, and a fuzzy session finder. Milestone 4 is under way: every session has terminal tabs next to its Chat tab.

## How it works

```
ccs (ratatui)
├── Session panel           working / waiting / idle, sorted by what needs you
└── Session "auth-refactor" [1 Chat] [2 diff] [3 shell] [4 review]
    ├── Chat tab   → the interactive `claude --plugin-dir mods`, in a pseudo-terminal
    │                the ccs-bridge mod reports what it does: `ccs hook` → Unix socket → ccs
    └── Peer tabs  → commands in a pseudo-terminal
                     portable-pty → vt100 (in-memory screen) → tui-term widget
```

Claude Code draws its own screen in the Chat tab, so all of its features work as usual. The ccs-bridge mod (`mods/ccs-bridge`) tells ccs when a session is ready, works, shows a permission dialog, is answered and goes idle, and when it is renamed. The ccs-traces mod (`mods/ccs-traces`) draws each run of tool calls (the calls Claude makes between two of its texts) as one line, `● 5 tool calls · Read 3 · Bash 2`: click it to list the calls, and click a call to see its whole input and output. It draws only in Claude's fullscreen mode (`"tui": "fullscreen"`), the one that takes clicks. Mods are an early-access Claude Code feature (2.1.301 and later), and their API can change between releases.

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
- [x] Track state from the mod's reports: starting, working, waiting for permission, idle
- [x] Everything else is Claude Code's own: replies, tool calls, permission dialogs, interrupts, slash commands, permission modes, plans
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
- [x] `CS_*` environment variables in every tab: `CS_CWD`, `CS_REPO`, `CS_BRANCH`, `CS_GIT_BASE`, `CS_SESSION_ID`
- [ ] Parameters as `CS_*` environment variables, asked for by prompt or picker (changed files)
- [x] Each session in its own git worktree (`claude --worktree`), its tabs there too; `worktree = false` turns it off
- [x] Tab modes: `eager`, `lazy`, `regular`
- [ ] Tab modes: `on_demand`, `on_change` (re-run when Claude edits files), `on_idle`
- [x] Leader key, then `c` / the tab's own key / `[` `]` to switch tabs
- [x] Resize tabs with the window, including hidden ones
- [x] Review tab: runs a preconfigured command, by default `git diff`
- [x] Terminal tab: opens a shell in the project directory, by default `$SHELL`, configurable

### Themes
- [x] One theme file colors Claude, ccs, the mods and every tab
- [x] Built in: gruvbox and Tokyo Night, dark and light
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

Everything else starts with the leader, Ctrl-Space, then one key:

| Leader, then | Action |
|---|---|
| (wait) | A panel lists the keys that can follow and what they do; Esc closes it |
| `1`–`9` | Go to session 1–9 |
| `,` / `.` | Previous / next session |
| `n` / `N` | Open a new session in this project: in a worktree of its own (unless `worktree = false`) / in the main checkout, without a worktree |
| `f` | Find a session: fuzzy search over names, titles, project names and your prompts, in open sessions and past ones from this directory. Enter goes to it (resuming a past one), Shift-Tab switches between open sessions only and both |
| `c` / a tab's key | Go to the Chat tab / that tab |
| `[` / `]` | Previous / next tab |
| Enter | In the Chat tab: submit the prompt, then open a new session in the main checkout, without a worktree, even if another one is waiting. In a terminal tab: run the tab's command again, stopping it first if it is still running |
| `w` | In the Chat tab: submit the prompt and stay |
| `q` | Quit |
| Ctrl-Space | Send Ctrl-Space to the program in the tab, Claude included |

Tabs come from `~/.config/ccs/config.toml` (see [`config/example.toml`](config/example.toml)); without one, every session gets a `review` tab (`git diff`), run again every time you open it, and a `shell` tab (`$SHELL`), started in the background with the session. Tab commands run in the session's directory and get `CS_*` variables such as `$CS_CWD` and `$CS_GIT_BASE` (listed in the example). Each new session works in its own git worktree and branch, made by `claude --worktree`, and its tabs run there; `worktree = false` turns that off. The leader is set with `leader = "ctrl-a"` and similar. Switching sessions slides the new one in from the side its chip is on; `slide_ms = 150` sets how long, and `slide_ms = 0` switches at once.

Each session is a plain `claude` with your own settings, plugins and permission mode; ccs only adds its mods. ccs loads them from this repository's `mods`, so a ccs binary needs its source tree for now. Start ccs from your own terminal rather than from inside a Claude Code session, whose `CLAUDE_CODE_*` variables would reach the sessions.

## Themes

Set `theme = "gruvbox-dark"` in `~/.config/ccs/config.toml`. The built-in themes are `gruvbox-dark`, `gruvbox-light`, `tokyonight-night` and `tokyonight-day` (in [`themes`](themes)). Without a theme, Claude draws in your own Claude theme and the tabs in your terminal's colors.

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
  | `starting`, `idle`, `working`, `waiting` | The state of each session: ◌ ○ ● ◆ | `inactive`, `success`, `warning`, `permission` |
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
