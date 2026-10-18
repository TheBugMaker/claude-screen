//! The colors of the screen, and the look shared by popups.
//!
//! A theme is a Claude Code theme file, `{ "name", "base", "overrides" }`, plus a `ccs` section
//! that Claude ignores: the colors of ccs's own screen, and the terminal colors drawn in place of
//! the terminal's own. A ccs color the section leaves out comes from the Claude key that means the
//! same, then from ccs's own colors. Claude gets the file through a plugin ccs writes for it.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use color_eyre::eyre::{OptionExt, WrapErr, bail, eyre};
use ratatui::{
    buffer::Buffer,
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, BorderType},
};
use serde::Deserialize;

/// The themes every ccs has, by name.
const BUILT_IN: &[(&str, &str)] = &[
    (
        "gruvbox-dark",
        include_str!("../../../themes/gruvbox-dark.json"),
    ),
    (
        "gruvbox-light",
        include_str!("../../../themes/gruvbox-light.json"),
    ),
    (
        "tokyonight-night",
        include_str!("../../../themes/tokyonight-night.json"),
    ),
    (
        "tokyonight-day",
        include_str!("../../../themes/tokyonight-day.json"),
    ),
];

/// Claude's base themes. It takes `dark` for any other.
const BASES: &[&str] = &[
    "dark",
    "light",
    "dark-daltonized",
    "light-daltonized",
    "dark-ansi",
    "light-ansi",
];

/// The names `ansi:<name>` takes, in the order of the 16 ANSI colors.
const ANSI: [(&str, Color); 16] = [
    ("black", Color::Black),
    ("red", Color::Red),
    ("green", Color::Green),
    ("yellow", Color::Yellow),
    ("blue", Color::Blue),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
    ("white", Color::Gray),
    ("blackBright", Color::DarkGray),
    ("redBright", Color::LightRed),
    ("greenBright", Color::LightGreen),
    ("yellowBright", Color::LightYellow),
    ("blueBright", Color::LightBlue),
    ("magentaBright", Color::LightMagenta),
    ("cyanBright", Color::LightCyan),
    ("whiteBright", Color::White),
];

/// The plugin that gives Claude the theme: `custom:<PLUGIN>:<name>` to Claude.
const PLUGIN: &str = "ccs-theme";

/// A theme file. Claude reads `name`, `base` and `overrides`, and ignores the rest.
#[derive(Deserialize)]
struct File {
    base: Option<String>,
    /// Claude's colors, by the keys of its base theme.
    #[serde(default)]
    overrides: HashMap<String, String>,
    #[serde(default)]
    ccs: Section,
}

/// The `ccs` section of a theme file.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct Section {
    foreground: Option<String>,
    background: Option<String>,
    palette: Option<Vec<String>>,
    accent: Option<String>,
    on_accent: Option<String>,
    dim: Option<String>,
    selection: Option<String>,
    starting: Option<String>,
    idle: Option<String>,
    asleep: Option<String>,
    working: Option<String>,
    waiting: Option<String>,
    error: Option<String>,
    warning: Option<String>,
    leader_pending: Option<String>,
}

/// The colors the screen is drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// The chip of the session on screen, the tab on screen, popup titles, the keys after the
    /// leader.
    pub accent: Color,
    /// Text on `accent`.
    pub on_accent: Color,
    /// Secondary text, rules and borders.
    pub dim: Color,
    /// Behind the session picked in the finder.
    pub selection: Color,
    /// The states of the sessions.
    pub starting: Color,
    pub idle: Color,
    pub asleep: Color,
    pub working: Color,
    pub waiting: Color,
    /// Notices, and what failed.
    pub error: Color,
    /// A tab whose command exited.
    pub warning: Color,
    /// The leader hint while the leader waits for a key.
    pub leader_pending: Color,
    /// Drawn in place of the terminal's default text color; `None` keeps the terminal's own.
    pub foreground: Option<Color>,
    /// Drawn in place of the terminal's default background.
    pub background: Option<Color>,
    /// Drawn in place of the terminal's 16 ANSI colors.
    pub palette: Option<[Color; 16]>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Claude's orange, from the 256-color palette so it shows without truecolor.
            accent: Color::Indexed(173),
            on_accent: Color::Black,
            dim: Color::DarkGray,
            // Assumes a dark terminal.
            selection: Color::Indexed(236),
            starting: Color::DarkGray,
            idle: Color::Green,
            // Not your turn, as working is; the glyph says it is in the background.
            asleep: Color::Yellow,
            working: Color::Yellow,
            waiting: Color::Magenta,
            error: Color::Red,
            warning: Color::Yellow,
            leader_pending: Color::Cyan,
            foreground: None,
            background: None,
            palette: None,
        }
    }
}

impl Theme {
    /// Reads a theme file. Claude's colors are checked too, since Claude skips a bad one quietly.
    pub fn parse(text: &str) -> color_eyre::Result<Self> {
        let file: File = serde_json::from_str(text)?;
        if let Some(base) = &file.base
            && !BASES.contains(&base.as_str())
        {
            bail!("base {base:?} is none of Claude's: {}", BASES.join(", "));
        }
        for (key, color) in &file.overrides {
            parse_color(color).wrap_err_with(|| format!("overrides.{key}"))?;
        }
        let ccs = &file.ccs;
        let default = Self::default();
        // The section's color, else the Claude color of `key`, else ccs's own.
        let pick = |name: &str, own: Option<&str>, key: &str, fallback: Color| match own
            .or(file.overrides.get(key).map(String::as_str))
        {
            Some(text) => parse_color(text).wrap_err_with(|| format!("ccs.{name}")),
            None => Ok(fallback),
        };
        let terminal = |name: &str, own: Option<&str>| {
            own.map(|text| parse_color(text).wrap_err_with(|| format!("ccs.{name}")))
                .transpose()
        };
        Ok(Self {
            accent: pick("accent", ccs.accent.as_deref(), "claude", default.accent)?,
            on_accent: pick(
                "onAccent",
                ccs.on_accent.as_deref(),
                "inverseText",
                default.on_accent,
            )?,
            dim: pick("dim", ccs.dim.as_deref(), "inactive", default.dim)?,
            selection: pick(
                "selection",
                ccs.selection.as_deref(),
                "userMessageBackground",
                default.selection,
            )?,
            starting: pick(
                "starting",
                ccs.starting.as_deref(),
                "inactive",
                default.starting,
            )?,
            idle: pick("idle", ccs.idle.as_deref(), "success", default.idle)?,
            asleep: pick("asleep", ccs.asleep.as_deref(), "warning", default.asleep)?,
            working: pick(
                "working",
                ccs.working.as_deref(),
                "warning",
                default.working,
            )?,
            waiting: pick(
                "waiting",
                ccs.waiting.as_deref(),
                "permission",
                default.waiting,
            )?,
            error: pick("error", ccs.error.as_deref(), "error", default.error)?,
            warning: pick(
                "warning",
                ccs.warning.as_deref(),
                "warning",
                default.warning,
            )?,
            leader_pending: pick(
                "leaderPending",
                ccs.leader_pending.as_deref(),
                "suggestion",
                default.leader_pending,
            )?,
            foreground: terminal("foreground", ccs.foreground.as_deref())?,
            background: terminal("background", ccs.background.as_deref())?,
            palette: ccs.palette.as_deref().map(palette).transpose()?,
        })
    }

    /// Draws the theme's terminal colors in place of the terminal's own in every cell of
    /// `buffer`: its default text color and background, and its 16 ANSI colors.
    pub fn paint(&self, buffer: &mut Buffer) {
        if self.foreground.is_none() && self.background.is_none() && self.palette.is_none() {
            return;
        }
        for cell in &mut buffer.content {
            cell.fg = self.terminal_color(cell.fg, self.foreground);
            cell.bg = self.terminal_color(cell.bg, self.background);
        }
    }

    /// `color` as the theme draws it, `default` being its default color.
    fn terminal_color(&self, color: Color, default: Option<Color>) -> Color {
        if color == Color::Reset {
            return default.unwrap_or(color);
        }
        match (ansi_index(color), &self.palette) {
            (Some(index), Some(palette)) => palette[index],
            _ => color,
        }
    }
}

/// The palette's 16 colors.
fn palette(colors: &[String]) -> color_eyre::Result<[Color; 16]> {
    let colors = colors
        .iter()
        .enumerate()
        .map(|(i, text)| parse_color(text).wrap_err_with(|| format!("ccs.palette[{i}]")))
        .collect::<color_eyre::Result<Vec<_>>>()?;
    <[Color; 16]>::try_from(colors)
        .map_err(|colors| eyre!("ccs.palette has {} colors, not 16", colors.len()))
}

/// Which of the 16 ANSI colors `color` is, if it is one.
fn ansi_index(color: Color) -> Option<usize> {
    match color {
        Color::Indexed(index) if index < 16 => Some(usize::from(index)),
        _ => ANSI.iter().position(|&(_, ansi)| ansi == color),
    }
}

/// A color as a Claude theme writes it: `#rrggbb`, `#rgb`, `rgb(r,g,b)`, `ansi256(n)` or
/// `ansi:<name>`.
pub fn parse_color(text: &str) -> color_eyre::Result<Color> {
    let invalid = || {
        eyre!("{text:?} is not a color: use #rrggbb, #rgb, rgb(r,g,b), ansi256(n) or ansi:<name>")
    };
    if let Some(hex) = text.strip_prefix('#') {
        let digits = hex
            .chars()
            .map(|c| c.to_digit(16).and_then(|d| u8::try_from(d).ok()))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(invalid)?;
        return match digits[..] {
            [r1, r2, g1, g2, b1, b2] => Ok(Color::Rgb(r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2)),
            [r, g, b] => Ok(Color::Rgb(r * 17, g * 17, b * 17)),
            _ => Err(invalid()),
        };
    }
    if let Some(rest) = text.strip_prefix("rgb(").and_then(|t| t.strip_suffix(')')) {
        let parts = rest
            .split(',')
            .map(|part| part.trim().parse::<u8>().ok())
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(invalid)?;
        return match parts[..] {
            [r, g, b] => Ok(Color::Rgb(r, g, b)),
            _ => Err(invalid()),
        };
    }
    if let Some(index) = text
        .strip_prefix("ansi256(")
        .and_then(|t| t.strip_suffix(')'))
    {
        return index.parse().map(Color::Indexed).map_err(|_| invalid());
    }
    if let Some(name) = text.strip_prefix("ansi:") {
        return ANSI
            .iter()
            .find(|&&(ansi, _)| ansi == name)
            .map(|&(_, color)| color)
            .ok_or_else(invalid);
    }
    Err(invalid())
}

/// The theme `name` and its file's text: `<dir>/<name>.json`, else the built-in theme of that
/// name.
pub fn load(name: &str, dir: &Path) -> color_eyre::Result<(Theme, String)> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("{name:?} is not a theme name: use letters, digits, - and _");
    }
    let path = dir.join(format!("{name}.json"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let (_, text) = BUILT_IN
                .iter()
                .find(|&&(built_in, _)| built_in == name)
                .ok_or_eyre(format!(
                    "no theme {name:?}: {} is missing, and the built-in themes are {}",
                    path.display(),
                    BUILT_IN
                        .iter()
                        .map(|&(name, _)| name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?;
            (*text).to_owned()
        }
        Err(err) => return Err(err).wrap_err_with(|| format!("reading {}", path.display())),
    };
    let theme = Theme::parse(&text).wrap_err_with(|| format!("theme {name:?}"))?;
    Ok((theme, text))
}

/// The theme the screen is drawn in.
static THEME: OnceLock<Theme> = OnceLock::new();

/// Makes `theme` the one the screen is drawn in, before anything is drawn.
pub fn install(theme: Theme) {
    let _ = THEME.set(theme);
}

/// The theme the screen is drawn in: ccs's own colors until one is installed.
pub fn get() -> &'static Theme {
    THEME.get_or_init(Theme::default)
}

/// The plugin that gives every Claude the theme, removed when this is dropped.
pub struct ClaudeTheme {
    dir: PathBuf,
    name: String,
}

impl ClaudeTheme {
    /// Writes the plugin to `dir`, with the theme file `text` as theme `name`.
    pub fn write(dir: PathBuf, name: &str, text: &str) -> io::Result<Self> {
        let manifest = dir.join(".claude-plugin");
        let themes = dir.join("themes");
        std::fs::create_dir_all(&manifest)?;
        std::fs::create_dir_all(&themes)?;
        let plugin = serde_json::json!({
            "name": PLUGIN,
            "description": "The theme claude-screen (ccs) draws its Claude sessions in",
        });
        std::fs::write(manifest.join("plugin.json"), plugin.to_string())?;
        std::fs::write(themes.join(format!("{name}.json")), text)?;
        Ok(Self {
            dir,
            name: name.to_owned(),
        })
    }

    /// The plugin's directory, for `claude --plugin-dir`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The `theme` setting that picks it, for `claude --settings`.
    pub fn setting(&self) -> String {
        format!("custom:{PLUGIN}:{}", self.name)
    }
}

impl Drop for ClaudeTheme {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A rounded box with a title, for the finder, the name box and the leader panel.
pub fn popup<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    let theme = get();
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.dim))
        .title(title.into().fg(theme.accent).bold())
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;

    /// The keys of Claude Code 2.1.302's base themes, the same in all of them. Claude skips an
    /// override of any other key without a word.
    const CLAUDE_KEYS: &[&str] = &[
        "autoAccept",
        "autoAcceptShimmer",
        "skill",
        "bashBorder",
        "claude",
        "claudeShimmer",
        "claudeBlue_FOR_SYSTEM_SPINNER",
        "claudeBlueShimmer_FOR_SYSTEM_SPINNER",
        "permission",
        "permissionShimmer",
        "planMode",
        "ide",
        "promptBorder",
        "promptBorderShimmer",
        "text",
        "inverseText",
        "inactive",
        "inactiveShimmer",
        "subtle",
        "suggestion",
        "remember",
        "background",
        "success",
        "error",
        "warning",
        "merged",
        "warningShimmer",
        "diffAdded",
        "diffRemoved",
        "diffAddedDimmed",
        "diffRemovedDimmed",
        "diffAddedWord",
        "diffRemovedWord",
        "red_FOR_SUBAGENTS_ONLY",
        "blue_FOR_SUBAGENTS_ONLY",
        "green_FOR_SUBAGENTS_ONLY",
        "yellow_FOR_SUBAGENTS_ONLY",
        "purple_FOR_SUBAGENTS_ONLY",
        "orange_FOR_SUBAGENTS_ONLY",
        "pink_FOR_SUBAGENTS_ONLY",
        "cyan_FOR_SUBAGENTS_ONLY",
        "professionalBlue",
        "chromeYellow",
        "clawd_body",
        "clawd_background",
        "userMessageBackground",
        "userMessageBackgroundHover",
        "composerSidebarBackground",
        "selectionBg",
        "bashMessageBackgroundColor",
        "memoryBackgroundColor",
        "rate_limit_fill",
        "rate_limit_empty",
        "fastMode",
        "fastModeShimmer",
        "effortUltra",
        "briefLabelYou",
        "briefLabelClaude",
        "rainbow_red",
        "rainbow_orange",
        "rainbow_yellow",
        "rainbow_green",
        "rainbow_blue",
        "rainbow_indigo",
        "rainbow_violet",
        "rainbow_red_shimmer",
        "rainbow_orange_shimmer",
        "rainbow_yellow_shimmer",
        "rainbow_green_shimmer",
        "rainbow_blue_shimmer",
        "rainbow_indigo_shimmer",
        "rainbow_violet_shimmer",
    ];

    #[test]
    fn parses_every_color_claude_does() {
        for (text, color) in [
            ("#fe8019", Color::Rgb(0xfe, 0x80, 0x19)),
            ("#FE8019", Color::Rgb(0xfe, 0x80, 0x19)),
            ("#f80", Color::Rgb(0xff, 0x88, 0x00)),
            ("rgb(215,119,87)", Color::Rgb(215, 119, 87)),
            ("rgb(55, 55, 55)", Color::Rgb(55, 55, 55)),
            ("ansi256(173)", Color::Indexed(173)),
            ("ansi:red", Color::Red),
            ("ansi:white", Color::Gray),
            ("ansi:blackBright", Color::DarkGray),
            ("ansi:whiteBright", Color::White),
        ] {
            assert_eq!(parse_color(text).unwrap(), color, "{text}");
        }
        for (i, &(name, color)) in ANSI.iter().enumerate() {
            assert_eq!(parse_color(&format!("ansi:{name}")).unwrap(), color);
            assert_eq!(ansi_index(color), Some(i));
        }
        for text in [
            "",
            "red",
            "#ff80",
            "#gg8019",
            "rgb(1,2)",
            "rgb(256,0,0)",
            "ansi256(300)",
            "ansi:orange",
        ] {
            assert!(parse_color(text).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn a_claude_theme_colors_ccs_too() {
        let theme = Theme::parse(
            r##"{ "name": "Mine", "base": "dark", "author": "someone",
                  "overrides": { "claude": "#112233", "success": "ansi:greenBright" } }"##,
        )
        .unwrap();
        assert_eq!(theme.accent, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(theme.idle, Color::LightGreen);
        // What the file doesn't say stays ccs's own, and the terminal keeps its colors.
        let default = Theme::default();
        assert_eq!(theme.dim, default.dim);
        assert_eq!(
            (theme.foreground, theme.background, theme.palette),
            (None, None, None)
        );

        // The ccs section comes first.
        let theme = Theme::parse(
            r##"{ "overrides": { "claude": "#112233" }, "ccs": { "accent": "#445566" } }"##,
        )
        .unwrap();
        assert_eq!(theme.accent, Color::Rgb(0x44, 0x55, 0x66));
    }

    #[test]
    fn rejects_bad_themes() {
        for text in [
            "[]",
            r#"{ "base": "midnight" }"#,
            r#"{ "overrides": { "claude": "orange" } }"#,
            r##"{ "ccs": { "acent": "#112233" } }"##,
            r##"{ "ccs": { "dim": "#1234" } }"##,
            r##"{ "ccs": { "palette": ["#000000"] } }"##,
        ] {
            assert!(Theme::parse(text).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn built_in_themes_parse_and_override_only_claude_keys() {
        for &(name, text) in BUILT_IN {
            let theme = Theme::parse(text).unwrap_or_else(|err| panic!("{name}: {err:?}"));
            assert!(
                theme.palette.is_some() && theme.background.is_some(),
                "{name}"
            );
            let file: File = serde_json::from_str(text).unwrap();
            for key in file.overrides.keys() {
                assert!(CLAUDE_KEYS.contains(&key.as_str()), "{name}: {key}");
            }
        }
    }

    /// A directory of its own under the system's temporary one.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ccs-theme-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_theme_file_comes_before_a_built_in_theme() {
        let dir = scratch("load");
        let (theme, text) = load("gruvbox-dark", &dir).unwrap();
        assert_eq!(theme.background, Some(Color::Rgb(0x28, 0x28, 0x28)));
        assert_eq!(text, BUILT_IN[0].1);

        let mine = r##"{ "overrides": { "claude": "#112233" } }"##;
        std::fs::write(dir.join("gruvbox-dark.json"), mine).unwrap();
        let (theme, text) = load("gruvbox-dark", &dir).unwrap();
        assert_eq!(theme.accent, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(text, mine);

        let err = load("solarized", &dir).unwrap_err().to_string();
        assert!(err.contains("gruvbox-light, tokyonight-night"), "{err}");
        assert!(load("../gruvbox-dark", &dir).is_err());
        assert!(load("custom:x", &dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn paints_the_terminal_colors_and_leaves_the_others() {
        let mut palette = [Color::Rgb(0, 0, 0); 16];
        palette[1] = Color::Rgb(1, 0, 0);
        palette[9] = Color::Rgb(9, 0, 0);
        let theme = Theme {
            foreground: Some(Color::Rgb(0xeb, 0xdb, 0xb2)),
            background: Some(Color::Rgb(0x28, 0x28, 0x28)),
            palette: Some(palette),
            ..Theme::default()
        };
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));
        let colors = [
            (Color::Reset, Color::Reset),
            (Color::Red, Color::Indexed(9)),
            (Color::Indexed(1), Color::LightRed),
            (Color::Rgb(7, 7, 7), Color::Indexed(173)),
            (Color::Indexed(16), Color::Reset),
        ];
        for (cell, (fg, bg)) in buffer.content.iter_mut().zip(colors) {
            cell.fg = fg;
            cell.bg = bg;
        }
        theme.paint(&mut buffer);
        let painted: Vec<(Color, Color)> = buffer.content.iter().map(|c| (c.fg, c.bg)).collect();
        assert_eq!(
            painted,
            [
                (Color::Rgb(0xeb, 0xdb, 0xb2), Color::Rgb(0x28, 0x28, 0x28)),
                (Color::Rgb(1, 0, 0), Color::Rgb(9, 0, 0)),
                (Color::Rgb(1, 0, 0), Color::Rgb(9, 0, 0)),
                (Color::Rgb(7, 7, 7), Color::Indexed(173)),
                (Color::Indexed(16), Color::Rgb(0x28, 0x28, 0x28)),
            ]
        );
    }

    #[test]
    fn the_claude_plugin_holds_the_theme_until_dropped() {
        let dir = scratch("plugin").join("ccs-1-theme");
        let plugin = ClaudeTheme::write(dir.clone(), "mine", "{}").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(".claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"], PLUGIN);
        assert_eq!(
            std::fs::read_to_string(dir.join("themes/mine.json")).unwrap(),
            "{}"
        );
        assert_eq!(plugin.dir(), dir);
        assert_eq!(plugin.setting(), "custom:ccs-theme:mine");
        drop(plugin);
        assert!(!dir.exists());
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }
}
