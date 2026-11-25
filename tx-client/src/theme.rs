//! The tx look and its theme: the mark, one glyph per agent state, semantic colour roles, and
//! one resolved theme handed to ratatui, tmux, fzf and nvim.
//!
//! Resolution order:
//! 1. Colour allowance: non-empty `NO_COLOR`, `TERM=dumb` or `TX_COLORS=none` yields
//!    [`ColorDepth::None`], where every resolved [`Style`] omits `fg` and `bg` completely
//!    (avoiding `crossterm 0.29`'s `CSI ; m` reset after modifiers) and keeps only attributes.
//! 2. Background: the `terminal` theme has fills only on a background the terminal reported
//!    (tmux's `client_theme`, recorded for the screens started later); a theme file may replace
//!    roles for dark or light only.
//! 3. Colour depth: `TX_COLORS` (`truecolor` | `256` | `16` | `none`), legacy `TX_HOME_COLORS=256`,
//!    `COLORTERM` (`truecolor` | `24bit`), or `TERM`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use ratatui_core::style::{Color, Modifier, Style};
use serde::Deserialize;
use tx_core::host::user_config::{self, GlyphTier, ThemeSettings};

use crate::render::tag_hash;

/// The mark at text size: the name, in letters.
pub const MARK: &str = "tx";

/// The mark as a logo: `tx` in pixels, two pixel rows to a cell (`#` lit, `.` clear). Drawn with
/// half-blocks, which is what the home's backdrops are made of, so it sits in a scene instead of
/// on it.
pub const LOGO_PIXELS: &[&str] = &[
    "..##................",
    "..##................",
    "########..##......##",
    "########..###....###",
    "..##.......###..###.",
    "..##........######..",
    "..##.........####...",
    "..##........######..",
    "..##..##...###..###.",
    "..######..###....###",
    "...####...##......##",
    "....................",
];

/// Terminal background polarity, reported by tmux's `#{client_theme}` or overridden by config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    Dark,
    Light,
    Unknown,
}

impl Polarity {
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "dark" => Self::Dark,
            "light" => Self::Light,
            _ => Self::Unknown,
        }
    }

    /// The background the terminal last reported, as `tx _theme-apply dark|light` recorded it
    /// under `data_dir`; `Unknown` before any terminal has answered.
    pub fn recorded(data_dir: &Path) -> Self {
        std::fs::read_to_string(data_dir.join(POLARITY_FILE))
            .map_or(Self::Unknown, |text| Self::parse(&text))
    }

    /// Records a known background under `data_dir` for the screens started after this.
    pub fn record(self, data_dir: &Path) -> std::io::Result<()> {
        let name = match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Unknown => return Ok(()),
        };
        let path = data_dir.join(POLARITY_FILE);
        std::fs::create_dir_all(path.parent().expect("the file is under plugins/ide"))?;
        std::fs::write(path, name)
    }
}

/// Where the terminal's last reported background is kept, relative to the data directory. tmux
/// knows it only once a client has attached and its terminal has answered; a screen that starts
/// before that, or outside tmux, reads it from here. It is the `ide` plugin's own data: `tx update`
/// prunes `<data>/conf`, where the generated theme files are.
pub const POLARITY_FILE: &str = "plugins/ide/terminal-background";

/// Colour depth available to a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    Truecolor,
    Ansi256,
    Ansi16,
    None,
}

impl ColorDepth {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" | "rgb" => Some(Self::Truecolor),
            "256" | "ansi256" => Some(Self::Ansi256),
            "16" | "ansi16" | "ansi" => Some(Self::Ansi16),
            "none" | "0" | "off" => Some(Self::None),
            _ => None,
        }
    }

    /// Detects the active terminal's colour depth from environment variables.
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
            return Self::None;
        }
        if let Ok(explicit) = std::env::var("TX_COLORS")
            && let Some(depth) = Self::parse(&explicit)
        {
            return depth;
        }
        if std::env::var("TX_HOME_COLORS").is_ok_and(|value| value == "256") {
            return Self::Ansi256;
        }
        if let Ok(term) = std::env::var("TERM") {
            let lowered = term.to_ascii_lowercase();
            if lowered == "dumb" || lowered == "vt100" {
                return Self::None;
            }
            if lowered.contains("direct") {
                return Self::Truecolor;
            }
            if lowered.contains("16color") || lowered == "xterm" {
                return Self::Ansi16;
            }
        }
        if let Ok(colorterm) = std::env::var("COLORTERM") {
            let lowered = colorterm.to_ascii_lowercase();
            if lowered.contains("truecolor") || lowered.contains("24bit") {
                return Self::Truecolor;
            }
        }
        Self::Truecolor
    }
}

/// The lines of every tx popup: one light rounded border, the terminal's own weight.
pub const POPUP_LINES: &str = "rounded";

/// A title as it sits in a border: a space each side, nothing else.
pub fn titled(title: &str) -> String {
    format!(" {} ", title.trim())
}

/// A popup's title, as a tmux format: the mark in the accent, then what the popup is. The
/// plugins' scripts and key fragments write the same string.
pub fn popup_title(name: &str) -> String {
    format!("#[#{{@tx-accent-style}}] #{{@tx-mark}}#[default] · {name} ")
}

/// Unified glyph alphabet: one shape per state across every plugin, independent of colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub working: &'static str,
    pub starting: &'static str,
    pub needs_you: &'static str,
    pub idle: &'static str,
    pub ended: &'static str,
    pub failed: &'static str,
    pub operator: &'static str,
    pub separator: &'static str,
    pub empty: &'static str,
    pub prompt: &'static str,
    pub ellipsis: &'static str,
    pub arrow_right: &'static str,
}

pub const UNICODE_GLYPHS: Glyphs = Glyphs {
    working: "●",
    starting: "◌",
    needs_you: "▲",
    idle: "○",
    ended: "×",
    failed: "×",
    operator: "◇",
    separator: "·",
    empty: "—",
    prompt: "›",
    ellipsis: "…",
    arrow_right: "→",
};

pub const ASCII_GLYPHS: Glyphs = Glyphs {
    working: "*",
    starting: "~",
    needs_you: "!",
    idle: "o",
    ended: "x",
    failed: "X",
    operator: "@",
    separator: ".",
    empty: "-",
    prompt: ">",
    ellipsis: "...",
    arrow_right: "->",
};

impl Glyphs {
    pub const fn for_tier(tier: GlyphTier) -> Self {
        match tier {
            GlyphTier::Unicode => UNICODE_GLYPHS,
            GlyphTier::Ascii => ASCII_GLYPHS,
        }
    }
}

/// Explicit 16-colour ANSI slot; never relies on Euclidean RGB downsampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ansi16 {
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

impl Ansi16 {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "default" | "reset" | "none" => Some(Self::Default),
            "black" => Some(Self::Black),
            "red" => Some(Self::Red),
            "green" => Some(Self::Green),
            "yellow" => Some(Self::Yellow),
            "blue" => Some(Self::Blue),
            "magenta" => Some(Self::Magenta),
            "cyan" => Some(Self::Cyan),
            "white" | "gray" | "grey" => Some(Self::White),
            "bright-black" | "darkgray" | "darkgrey" => Some(Self::BrightBlack),
            "bright-red" => Some(Self::BrightRed),
            "bright-green" => Some(Self::BrightGreen),
            "bright-yellow" => Some(Self::BrightYellow),
            "bright-blue" => Some(Self::BrightBlue),
            "bright-magenta" => Some(Self::BrightMagenta),
            "bright-cyan" => Some(Self::BrightCyan),
            "bright-white" => Some(Self::BrightWhite),
            _ => None,
        }
    }

    fn ratatui_color(self) -> Option<Color> {
        match self {
            Self::Default => None,
            Self::Black => Some(Color::Black),
            Self::Red => Some(Color::Red),
            Self::Green => Some(Color::Green),
            Self::Yellow => Some(Color::Yellow),
            Self::Blue => Some(Color::Blue),
            Self::Magenta => Some(Color::Magenta),
            Self::Cyan => Some(Color::Cyan),
            Self::White => Some(Color::Gray),
            Self::BrightBlack => Some(Color::DarkGray),
            Self::BrightRed => Some(Color::LightRed),
            Self::BrightGreen => Some(Color::LightGreen),
            Self::BrightYellow => Some(Color::LightYellow),
            Self::BrightBlue => Some(Color::LightBlue),
            Self::BrightMagenta => Some(Color::LightMagenta),
            Self::BrightCyan => Some(Color::LightCyan),
            Self::BrightWhite => Some(Color::White),
        }
    }

    /// Its number among the terminal's sixteen; `None` for the default colour.
    fn index(self) -> Option<u8> {
        match self {
            Self::Default => None,
            Self::Black => Some(0),
            Self::Red => Some(1),
            Self::Green => Some(2),
            Self::Yellow => Some(3),
            Self::Blue => Some(4),
            Self::Magenta => Some(5),
            Self::Cyan => Some(6),
            Self::White => Some(7),
            Self::BrightBlack => Some(8),
            Self::BrightRed => Some(9),
            Self::BrightGreen => Some(10),
            Self::BrightYellow => Some(11),
            Self::BrightBlue => Some(12),
            Self::BrightMagenta => Some(13),
            Self::BrightCyan => Some(14),
            Self::BrightWhite => Some(15),
        }
    }

    fn tmux_name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Black => "black",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Magenta => "magenta",
            Self::Cyan => "cyan",
            Self::White => "white",
            Self::BrightBlack => "brightblack",
            Self::BrightRed => "brightred",
            Self::BrightGreen => "brightgreen",
            Self::BrightYellow => "brightyellow",
            Self::BrightBlue => "brightblue",
            Self::BrightMagenta => "brightmagenta",
            Self::BrightCyan => "brightcyan",
            Self::BrightWhite => "brightwhite",
        }
    }

    fn ansi_fg_code(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Black => Some("30"),
            Self::Red => Some("31"),
            Self::Green => Some("32"),
            Self::Yellow => Some("33"),
            Self::Blue => Some("34"),
            Self::Magenta => Some("35"),
            Self::Cyan => Some("36"),
            Self::White => Some("37"),
            Self::BrightBlack => Some("90"),
            Self::BrightRed => Some("91"),
            Self::BrightGreen => Some("92"),
            Self::BrightYellow => Some("93"),
            Self::BrightBlue => Some("94"),
            Self::BrightMagenta => Some("95"),
            Self::BrightCyan => Some("96"),
            Self::BrightWhite => Some("97"),
        }
    }

    fn ansi_bg_code(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Black => Some("40"),
            Self::Red => Some("41"),
            Self::Green => Some("42"),
            Self::Yellow => Some("43"),
            Self::Blue => Some("44"),
            Self::Magenta => Some("45"),
            Self::Cyan => Some("46"),
            Self::White => Some("47"),
            Self::BrightBlack => Some("100"),
            Self::BrightRed => Some("101"),
            Self::BrightGreen => Some("102"),
            Self::BrightYellow => Some("103"),
            Self::BrightBlue => Some("104"),
            Self::BrightMagenta => Some("105"),
            Self::BrightCyan => Some("106"),
            Self::BrightWhite => Some("107"),
        }
    }
}

/// One colour role specified across all three colour depths plus structural modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleSpec {
    pub rgb: Option<(u8, u8, u8)>,
    pub ansi256: Option<u8>,
    pub ansi16: Ansi16,
    pub bold: bool,
    pub dim: bool,
    pub reversed: bool,
    /// When true, in 16-colour mode render as reverse video (e.g. `signal` on light backgrounds
    /// where ANSI yellow is unreadable).
    pub reverse_on_16: bool,
    /// When true, under `NO_COLOR` render as reverse video so state/selection stays distinct.
    pub reverse_on_no_color: bool,
}

impl RoleSpec {
    const fn default_role() -> Self {
        Self {
            rgb: None,
            ansi256: None,
            ansi16: Ansi16::Default,
            bold: false,
            dim: false,
            reversed: false,
            reverse_on_16: false,
            reverse_on_no_color: false,
        }
    }

    const fn with_rgb(
        rgb: (u8, u8, u8),
        ansi256: u8,
        ansi16: Ansi16,
        bold: bool,
        dim: bool,
    ) -> Self {
        Self {
            rgb: Some(rgb),
            ansi256: Some(ansi256),
            ansi16,
            bold,
            dim,
            reversed: false,
            reverse_on_16: false,
            reverse_on_no_color: false,
        }
    }

    fn modifiers(self, depth: ColorDepth) -> Modifier {
        let mut modifier = Modifier::empty();
        if self.bold {
            modifier |= Modifier::BOLD;
        }
        if self.dim {
            modifier |= Modifier::DIM;
        }
        if self.reversed
            || (depth == ColorDepth::Ansi16 && self.reverse_on_16)
            || (depth == ColorDepth::None && self.reverse_on_no_color)
        {
            modifier |= Modifier::REVERSED;
        }
        modifier
    }

    /// Resolved foreground `Color` at `depth`; `None` means omit colour entirely (`NO_COLOR` or
    /// terminal default), never setting a crossterm `SetForegroundColor`.
    pub fn color(self, depth: ColorDepth) -> Option<Color> {
        match depth {
            ColorDepth::None => None,
            ColorDepth::Truecolor => self
                .rgb
                .map(|(red, green, blue)| Color::Rgb(red, green, blue))
                .or_else(|| self.ansi256.map(Color::Indexed))
                .or_else(|| self.ansi16.ratatui_color()),
            ColorDepth::Ansi256 => self
                .ansi256
                .map(Color::Indexed)
                .or_else(|| self.rgb.map(|(r, g, b)| rgb_to_256(r, g, b)))
                .or_else(|| self.ansi16.ratatui_color()),
            ColorDepth::Ansi16 => {
                if self.reverse_on_16 {
                    None
                } else {
                    self.ansi16.ratatui_color()
                }
            }
        }
    }

    pub fn style(self, depth: ColorDepth) -> Style {
        let mut style = Style::new();
        if let Some(color) = self.color(depth) {
            style = style.fg(color);
        }
        let modifier = self.modifiers(depth);
        if !modifier.is_empty() {
            style = style.add_modifier(modifier);
        }
        style
    }

    /// SGR foreground escape at `depth`, or empty string when terminal default / `NO_COLOR`.
    pub fn ansi_fg(self, depth: ColorDepth) -> String {
        match depth {
            ColorDepth::None => String::new(),
            ColorDepth::Truecolor => match self.rgb {
                Some((red, green, blue)) => format!("\x1b[38;2;{red};{green};{blue}m"),
                None => self
                    .ansi256
                    .map(|index| format!("\x1b[38;5;{index}m"))
                    .or_else(|| {
                        self.ansi16
                            .ansi_fg_code()
                            .map(|code| format!("\x1b[{code}m"))
                    })
                    .unwrap_or_default(),
            },
            ColorDepth::Ansi256 => match self.ansi256 {
                Some(index) => format!("\x1b[38;5;{index}m"),
                None => match self.rgb {
                    Some((red, green, blue)) => {
                        format!("\x1b[38;5;{}m", rgb_to_256_index(red, green, blue))
                    }
                    None => self
                        .ansi16
                        .ansi_fg_code()
                        .map(|code| format!("\x1b[{code}m"))
                        .unwrap_or_default(),
                },
            },
            ColorDepth::Ansi16 => {
                if self.reverse_on_16 {
                    String::new()
                } else {
                    self.ansi16
                        .ansi_fg_code()
                        .map(|code| format!("\x1b[{code}m"))
                        .unwrap_or_default()
                }
            }
        }
    }

    /// SGR background escape at `depth`, or empty string when default / `NO_COLOR`.
    pub fn ansi_bg(self, depth: ColorDepth) -> String {
        match depth {
            ColorDepth::None => String::new(),
            ColorDepth::Truecolor => match self.rgb {
                Some((red, green, blue)) => format!("\x1b[48;2;{red};{green};{blue}m"),
                None => self
                    .ansi256
                    .map(|index| format!("\x1b[48;5;{index}m"))
                    .or_else(|| {
                        self.ansi16
                            .ansi_bg_code()
                            .map(|code| format!("\x1b[{code}m"))
                    })
                    .unwrap_or_default(),
            },
            ColorDepth::Ansi256 => match self.ansi256 {
                Some(index) => format!("\x1b[48;5;{index}m"),
                None => match self.rgb {
                    Some((red, green, blue)) => {
                        format!("\x1b[48;5;{}m", rgb_to_256_index(red, green, blue))
                    }
                    None => self
                        .ansi16
                        .ansi_bg_code()
                        .map(|code| format!("\x1b[{code}m"))
                        .unwrap_or_default(),
                },
            },
            ColorDepth::Ansi16 => self
                .ansi16
                .ansi_bg_code()
                .map(|code| format!("\x1b[{code}m"))
                .unwrap_or_default(),
        }
    }

    /// tmux colour value (`#rrggbb`, `colourN`, ANSI name, or `default`).
    pub fn tmux_color(self, depth: ColorDepth) -> String {
        match depth {
            ColorDepth::None => "default".to_owned(),
            ColorDepth::Truecolor => match self.rgb {
                Some((red, green, blue)) => format!("#{red:02x}{green:02x}{blue:02x}"),
                None => self
                    .ansi256
                    .map(|index| format!("colour{index}"))
                    .unwrap_or_else(|| self.ansi16.tmux_name().to_owned()),
            },
            ColorDepth::Ansi256 => match self.ansi256 {
                Some(index) => format!("colour{index}"),
                None => match self.rgb {
                    Some((red, green, blue)) => {
                        format!("colour{}", rgb_to_256_index(red, green, blue))
                    }
                    None => self.ansi16.tmux_name().to_owned(),
                },
            },
            ColorDepth::Ansi16 => {
                if self.reverse_on_16 {
                    "default".to_owned()
                } else {
                    self.ansi16.tmux_name().to_owned()
                }
            }
        }
    }

    /// tmux style expression (`fg=...,bold,dim,reverse`).
    pub fn tmux_fg_style(self, depth: ColorDepth) -> String {
        let mut parts = vec![format!("fg={}", self.tmux_color(depth))];
        if self.bold {
            parts.push("bold".to_owned());
        }
        if self.dim {
            parts.push("dim".to_owned());
        }
        if self.reversed
            || (depth == ColorDepth::Ansi16 && self.reverse_on_16)
            || (depth == ColorDepth::None && self.reverse_on_no_color)
        {
            parts.push("reverse".to_owned());
        }
        parts.join(",")
    }

    /// Hex string `#rrggbb` when an RGB value exists.
    pub fn hex(self) -> Option<String> {
        self.rgb
            .map(|(red, green, blue)| format!("#{red:02x}{green:02x}{blue:02x}"))
    }
}

/// Semantic colour roles shared by every tx surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Primary text.
    Text,
    /// Secondary text, hints, inactive labels, idle/ended agents.
    Muted,
    /// Inactive frame lines, status rule filler, inactive borders.
    Rule,
    /// Active pane border and label.
    RuleActive,
    /// The single colour allowed on the frame: needs you, owed attention, attach-to.
    Signal,
    /// Selection and focus inside tx's own TUI screens (never on the guest-pane frame).
    Accent,
    /// Text drawn on top of an `Accent` fill.
    OnAccent,
    /// Working state inside tx's own screens.
    Working,
    /// Informational edges, message traffic, filter queries.
    Info,
    /// Failed state and destructive prompts.
    Failed,
    /// Surface background (popups, bar mode).
    Surface,
    /// Highlighted row background in lists/tables.
    Selection,
}

/// A resolved theme: concrete roles, glyph tier, status line style and colour depth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub glyph_tier: GlyphTier,
    pub glyphs: Glyphs,
    pub depth: ColorDepth,
    pub polarity: Polarity,
    pub text: RoleSpec,
    pub muted: RoleSpec,
    pub rule: RoleSpec,
    pub rule_active: RoleSpec,
    pub signal: RoleSpec,
    pub accent: RoleSpec,
    pub on_accent: RoleSpec,
    pub working: RoleSpec,
    pub info: RoleSpec,
    pub failed: RoleSpec,
    pub surface: RoleSpec,
    pub selection: RoleSpec,
    pub tag_palette: [u8; 12],
}

const TAG_CUBE_DARK: [u8; 12] = [210, 167, 215, 149, 80, 73, 37, 117, 38, 141, 198, 140];
const TAG_CUBE_LIGHT: [u8; 12] = [131, 124, 130, 64, 30, 29, 23, 25, 24, 97, 126, 96];

/// The roles a theme may set, as `config.yaml` and a theme file name them.
pub const ROLE_NAMES: &str = "text, muted, rule, rule_active, signal, accent, on_accent, working, \
                              info, failed, surface, selection";

/// Why a theme could not be resolved. Reported by `tx update`; a screen that meets one draws
/// with the shipped theme instead.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    #[error("theme: unknown role `{0}` (the roles are {ROLE_NAMES})")]
    UnknownRole(String),
    #[error(
        "theme: `{value}` for `{role}` is neither #rrggbb nor one of the terminal's sixteen \
         colour names"
    )]
    BadColor { role: String, value: String },
    #[error("theme: {path}: {reason}")]
    File { path: String, reason: String },
}

/// What differs in tmux between a theme with fills and one without.
struct TmuxLayout {
    lines: &'static str,
    border: String,
    border_active: String,
    header: String,
    header_active: String,
    attention: String,
    window_current: String,
    /// The active pane's border while attach-to picks a pane.
    picking: String,
}

/// A theme of the user's, `<config>/themes/<name>.yaml`: a shipped theme (`base`) with some
/// roles replaced, for both backgrounds (`colors`) or for one (`dark`, `light`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    #[serde(default = "automatic_name")]
    base: String,
    #[serde(default)]
    colors: BTreeMap<String, String>,
    #[serde(default)]
    dark: BTreeMap<String, String>,
    #[serde(default)]
    light: BTreeMap<String, String>,
}

fn automatic_name() -> String {
    "terminal".to_owned()
}

impl Theme {
    /// The default look: every role is the terminal's own foreground, one of its sixteen colours
    /// or an attribute, so tx wears whatever theme the terminal has. A known background adds two
    /// quiet fills, taken from the neutral end of the 256-colour palette: `surface` (the status
    /// bar, a pane's header, the home's card) and `selection` (the selected row, the active
    /// header). With an unknown background there are no fills: reverse video and lines instead.
    pub fn terminal(polarity: Polarity, depth: ColorDepth) -> Self {
        let named = |ansi16: Ansi16| RoleSpec {
            ansi16,
            ..RoleSpec::default_role()
        };
        let fill = |dark: (Ansi16, u8), light: (Ansi16, u8)| {
            let (ansi16, ansi256) = match polarity {
                Polarity::Dark => dark,
                Polarity::Light => light,
                Polarity::Unknown => return RoleSpec::default_role(),
            };
            RoleSpec {
                ansi256: Some(ansi256),
                ansi16,
                ..RoleSpec::default_role()
            }
        };
        Self {
            name: "terminal".to_owned(),
            glyph_tier: GlyphTier::Unicode,
            glyphs: UNICODE_GLYPHS,
            depth,
            polarity,
            text: RoleSpec::default_role(),
            muted: RoleSpec {
                dim: true,
                ..RoleSpec::default_role()
            },
            rule: match polarity {
                Polarity::Unknown => RoleSpec {
                    dim: true,
                    ..RoleSpec::default_role()
                },
                Polarity::Dark | Polarity::Light => named(Ansi16::BrightBlack),
            },
            rule_active: named(Ansi16::Blue),
            // ANSI yellow is the least readable of the sixteen on a light background.
            signal: match polarity {
                Polarity::Dark => RoleSpec {
                    bold: true,
                    reverse_on_no_color: true,
                    ..named(Ansi16::Yellow)
                },
                Polarity::Light | Polarity::Unknown => RoleSpec {
                    bold: true,
                    reversed: true,
                    reverse_on_no_color: true,
                    ..RoleSpec::default_role()
                },
            },
            accent: named(Ansi16::Blue),
            on_accent: match polarity {
                Polarity::Dark => named(Ansi16::Black),
                Polarity::Light => named(Ansi16::BrightWhite),
                Polarity::Unknown => RoleSpec::default_role(),
            },
            working: named(Ansi16::Green),
            info: named(Ansi16::Cyan),
            failed: RoleSpec {
                reverse_on_no_color: true,
                ..named(Ansi16::Red)
            },
            surface: fill((Ansi16::Black, 0), (Ansi16::White, 7)),
            selection: RoleSpec {
                reversed: polarity == Polarity::Unknown,
                reverse_on_16: true,
                reverse_on_no_color: true,
                ..fill((Ansi16::Default, 236), (Ansi16::Default, 254))
            },
            tag_palette: match polarity {
                Polarity::Light => TAG_CUBE_LIGHT,
                Polarity::Dark | Polarity::Unknown => TAG_CUBE_DARK,
            },
        }
    }

    /// Previous default palette (`tokyo-night`), preserved as a first-class shipped preset.
    pub fn tokyo_night(depth: ColorDepth) -> Self {
        Self {
            name: "tokyo-night".to_owned(),
            glyph_tier: GlyphTier::Unicode,
            glyphs: UNICODE_GLYPHS,
            depth,
            polarity: Polarity::Dark,
            text: RoleSpec::with_rgb((192, 202, 245), 189, Ansi16::Default, false, false),
            muted: RoleSpec::with_rgb((169, 177, 214), 146, Ansi16::Default, false, true),
            rule: RoleSpec::with_rgb((59, 66, 97), 239, Ansi16::Default, false, true),
            rule_active: RoleSpec::with_rgb((122, 162, 247), 111, Ansi16::Blue, true, false),
            signal: RoleSpec {
                rgb: Some((224, 175, 104)),
                ansi256: Some(179),
                ansi16: Ansi16::Yellow,
                bold: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            accent: RoleSpec {
                rgb: Some((122, 162, 247)),
                ansi256: Some(111),
                ansi16: Ansi16::Blue,
                ..RoleSpec::default_role()
            },
            on_accent: RoleSpec::with_rgb((26, 27, 38), 234, Ansi16::Black, false, false),
            working: RoleSpec::with_rgb((158, 206, 106), 149, Ansi16::Green, false, false),
            info: RoleSpec::with_rgb((125, 207, 255), 117, Ansi16::Cyan, false, false),
            failed: RoleSpec {
                rgb: Some((247, 118, 142)),
                ansi256: Some(210),
                ansi16: Ansi16::Red,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            surface: RoleSpec::with_rgb((21, 22, 30), 234, Ansi16::Black, false, false),
            selection: RoleSpec {
                rgb: Some((48, 48, 48)),
                ansi256: Some(236),
                ansi16: Ansi16::Default,
                reverse_on_16: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            tag_palette: TAG_CUBE_DARK,
        }
    }

    /// Resolves `settings` against `<config_dir>/themes/`, `polarity` and `depth`. A theme that
    /// names an unknown role, a colour that is neither `#rrggbb` nor one of the terminal's
    /// sixteen, or a theme file that is missing or malformed, is an error: `tx update` reports
    /// it instead of applying part of a theme.
    pub fn resolve(
        settings: &ThemeSettings,
        config_dir: &Path,
        polarity: Polarity,
        depth: ColorDepth,
    ) -> Result<Self, ThemeError> {
        let mut theme = Self::from_name(&settings.name, config_dir, polarity, depth)?;
        theme.glyph_tier = settings.glyphs;
        theme.glyphs = Glyphs::for_tier(settings.glyphs);
        theme.apply_overrides(&settings.colors)?;
        Ok(theme)
    }

    /// Loads `config.yaml` under `config_dir` and resolves the theme.
    pub fn load(
        config_dir: &Path,
        polarity: Polarity,
        depth: ColorDepth,
    ) -> Result<Self, ThemeError> {
        let settings = user_config::load(&config_dir.join(user_config::FILE_NAME))
            .map(|config| config.theme)
            .unwrap_or_default();
        Self::resolve(&settings, config_dir, polarity, depth)
    }

    /// Loads the theme for a specific `TxCore` using its resolved config directory.
    pub fn for_core(
        core: &tx_core::TxCore,
        polarity: Polarity,
        depth: ColorDepth,
    ) -> Result<Self, ThemeError> {
        Self::load(&core.config().directories.config, polarity, depth)
    }

    fn from_name(
        name: &str,
        config_dir: &Path,
        polarity: Polarity,
        depth: ColorDepth,
    ) -> Result<Self, ThemeError> {
        match name {
            "terminal" => Ok(Self::terminal(polarity, depth)),
            "tokyo-night" => Ok(Self::tokyo_night(depth)),
            custom => {
                let path = config_dir.join("themes").join(format!("{custom}.yaml"));
                let failed = |reason: String| ThemeError::File {
                    path: path.display().to_string(),
                    reason,
                };
                let text =
                    std::fs::read_to_string(&path).map_err(|error| failed(error.to_string()))?;
                let file: ThemeFile =
                    serde_norway::from_str(&text).map_err(|error| failed(error.to_string()))?;
                let mut theme = Self::from_name(&file.base, config_dir, polarity, depth)?;
                theme.name = custom.to_owned();
                theme.apply_overrides(&file.colors)?;
                match polarity {
                    Polarity::Dark => theme.apply_overrides(&file.dark)?,
                    Polarity::Light => theme.apply_overrides(&file.light)?,
                    Polarity::Unknown => {}
                }
                Ok(theme)
            }
        }
    }

    fn apply_overrides(&mut self, overrides: &BTreeMap<String, String>) -> Result<(), ThemeError> {
        for (role, value) in overrides {
            let slot = self
                .role_mut(role)
                .ok_or_else(|| ThemeError::UnknownRole(role.clone()))?;
            *slot = parse_color_override(value, *slot).ok_or_else(|| ThemeError::BadColor {
                role: role.clone(),
                value: value.clone(),
            })?;
        }
        Ok(())
    }

    fn role_mut(&mut self, name: &str) -> Option<&mut RoleSpec> {
        match name {
            "text" => Some(&mut self.text),
            "muted" => Some(&mut self.muted),
            "rule" => Some(&mut self.rule),
            "rule_active" => Some(&mut self.rule_active),
            "signal" => Some(&mut self.signal),
            "accent" => Some(&mut self.accent),
            "on_accent" => Some(&mut self.on_accent),
            "working" => Some(&mut self.working),
            "info" => Some(&mut self.info),
            "failed" => Some(&mut self.failed),
            "surface" => Some(&mut self.surface),
            "selection" => Some(&mut self.selection),
            _ => None,
        }
    }

    pub fn spec(&self, role: Role) -> RoleSpec {
        match role {
            Role::Text => self.text,
            Role::Muted => self.muted,
            Role::Rule => self.rule,
            Role::RuleActive => self.rule_active,
            Role::Signal => self.signal,
            Role::Accent => self.accent,
            Role::OnAccent => self.on_accent,
            Role::Working => self.working,
            Role::Info => self.info,
            Role::Failed => self.failed,
            Role::Surface => self.surface,
            Role::Selection => self.selection,
        }
    }

    pub fn style(&self, role: Role) -> Style {
        self.spec(role).style(self.depth)
    }

    pub fn bold(&self, role: Role) -> Style {
        self.style(role).add_modifier(Modifier::BOLD)
    }

    pub fn color(&self, role: Role) -> Option<Color> {
        self.spec(role).color(self.depth)
    }

    /// The selected row of a list or a table: the theme's quiet fill, or reverse video where
    /// there is none (an unknown background, sixteen colours, no colour).
    pub fn selection_style(&self) -> Style {
        match self.color(Role::Selection) {
            Some(background) if !self.selection.reversed => Style::new().bg(background),
            _ => Style::new().add_modifier(Modifier::REVERSED),
        }
    }

    /// The mark as a chip, and any text on an accent fill: reverse video where the theme has no
    /// colour for one of the two.
    pub fn on_accent_badge(&self) -> Style {
        match (self.color(Role::Accent), self.color(Role::OnAccent)) {
            (Some(background), Some(foreground)) if self.depth != ColorDepth::Ansi16 => {
                Style::new()
                    .fg(foreground)
                    .bg(background)
                    .add_modifier(Modifier::BOLD)
            }
            _ => Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        }
    }

    /// Whether the theme has its two fills at this depth: bars and chips where it does, lines
    /// and reverse video where it does not.
    pub fn has_fills(&self) -> bool {
        !self.selection.reversed
            && self.color(Role::Surface).is_some()
            && self.color(Role::Selection).is_some()
    }

    /// Maps an arbitrary `Color` (such as a GitHub label or backdrop pixel) down to `self.depth`.
    /// Under `ColorDepth::None`, returns `None` so callers omit colour cleanly.
    pub fn map_color(&self, color: Color) -> Option<Color> {
        match self.depth {
            ColorDepth::None => None,
            ColorDepth::Truecolor => Some(color),
            ColorDepth::Ansi256 => match color {
                Color::Rgb(red, green, blue) => Some(rgb_to_256(red, green, blue)),
                other => Some(other),
            },
            ColorDepth::Ansi16 => match color {
                Color::Rgb(red, green, blue) => Some(rgb_to_16(red, green, blue)),
                Color::Indexed(_) => None,
                other => Some(other),
            },
        }
    }

    pub fn ansi_fg(&self, role: Role) -> String {
        self.spec(role).ansi_fg(self.depth)
    }

    pub fn ansi_bg(&self, role: Role) -> String {
        self.spec(role).ansi_bg(self.depth)
    }

    pub fn tmux_color(&self, role: Role) -> String {
        self.spec(role).tmux_color(self.depth)
    }

    pub fn tmux_fg_style(&self, role: Role) -> String {
        self.spec(role).tmux_fg_style(self.depth)
    }

    /// Deterministic hash-assigned tag colour (`None` under `NO_COLOR`).
    pub fn tag_color(&self, tag: &str) -> Option<Color> {
        match self.depth {
            ColorDepth::None => None,
            ColorDepth::Ansi16 => {
                const ANSI_TAGS: [Color; 6] = [
                    Color::Cyan,
                    Color::Green,
                    Color::Magenta,
                    Color::Blue,
                    Color::Yellow,
                    Color::Red,
                ];
                let index = (tag_hash(tag) % ANSI_TAGS.len() as u64) as usize;
                Some(ANSI_TAGS[index])
            }
            ColorDepth::Truecolor | ColorDepth::Ansi256 => {
                let index = (tag_hash(tag) % self.tag_palette.len() as u64) as usize;
                Some(Color::Indexed(self.tag_palette[index]))
            }
        }
    }

    pub fn tag_ansi(&self, tag: &str) -> String {
        match self.tag_color(tag) {
            Some(Color::Indexed(index)) => format!("\x1b[38;5;{index}m"),
            Some(Color::Cyan) => "\x1b[36m".to_owned(),
            Some(Color::Green) => "\x1b[32m".to_owned(),
            Some(Color::Magenta) => "\x1b[35m".to_owned(),
            Some(Color::Blue) => "\x1b[34m".to_owned(),
            Some(Color::Yellow) => "\x1b[33m".to_owned(),
            Some(Color::Red) => "\x1b[31m".to_owned(),
            _ => String::new(),
        }
    }

    pub fn tag_tmux(&self, tag: &str) -> String {
        match self.tag_color(tag) {
            Some(Color::Indexed(index)) => format!("colour{index}"),
            Some(Color::Cyan) => "cyan".to_owned(),
            Some(Color::Green) => "green".to_owned(),
            Some(Color::Magenta) => "magenta".to_owned(),
            Some(Color::Blue) => "blue".to_owned(),
            Some(Color::Yellow) => "yellow".to_owned(),
            Some(Color::Red) => "red".to_owned(),
            _ => "default".to_owned(),
        }
    }

    /// fzf's `--color=…` argument for this theme, or `--no-color`.
    pub fn fzf_color_arg(&self, with_surface_bg: bool) -> String {
        if self.depth == ColorDepth::None {
            return "--no-color".to_owned();
        }
        let color = |role: Role| fzf_value(self.spec(role), self.depth);
        let muted = format!("{}:dim", color(Role::Muted));
        let accent = color(Role::Accent);
        let mut parts = vec![
            format!("fg:{muted}"),
            format!("header:{muted}"),
            format!("footer:{muted}"),
            format!("prompt:{accent}"),
            format!("query:{}", color(Role::Text)),
            format!("pointer:{accent}"),
            format!("hl:{accent}"),
            format!("hl+:{accent}"),
        ];
        if self.has_fills() {
            parts.push(format!("fg+:{}:regular", color(Role::Text)));
            parts.push(format!("bg+:{}:regular", color(Role::Selection)));
            if with_surface_bg {
                parts.push(format!("bg:{}", color(Role::Surface)));
            }
        } else {
            parts.push("fg+:-1:reverse".to_owned());
        }
        format!("--color={}", parts.join(","))
    }

    /// Generates `<data>/conf/theme.tmux`: every tmux visual option, in one place. tmux's own
    /// `status-left`, window formats and `status-right` stay slots, so the user and the plugins
    /// (`attention.tmux`) fill them freely; the styles they refer to are the `@tx-*` options.
    pub fn tmux_conf(&self) -> String {
        let muted = self.tmux_fg_style(Role::Muted);
        let signal = self.tmux_fg_style(Role::Signal);
        let accent = format!("{},bold", self.tmux_fg_style(Role::Accent));
        let surface = self.tmux_color(Role::Surface);
        let selection = self.tmux_color(Role::Selection);
        let chip = match (self.color(Role::Accent), self.color(Role::OnAccent)) {
            (Some(_), Some(_)) if self.depth != ColorDepth::Ansi16 => format!(
                "fg={},bg={},bold",
                self.tmux_color(Role::OnAccent),
                self.tmux_color(Role::Accent)
            ),
            _ => "fg=default,bold,reverse".to_owned(),
        };
        // With fills a pane's border is a bar: a header row and a blank divider. Without them
        // it is a light line.
        let layout = if self.has_fills() {
            TmuxLayout {
                lines: "spaces",
                border: format!("bg={surface}"),
                border_active: format!("bg={selection}"),
                header: format!("fg=default,bg={surface}"),
                header_active: format!("fg=default,bg={selection}"),
                attention: format!("{signal},bg={selection}"),
                window_current: format!("fg=default,bg={selection},bold"),
                picking: format!(
                    "fg={},bg={},bold",
                    self.tmux_color(Role::OnAccent),
                    self.tmux_color(Role::Signal)
                ),
            }
        } else {
            TmuxLayout {
                lines: "single",
                border: self.tmux_fg_style(Role::Rule),
                border_active: self.tmux_fg_style(Role::RuleActive),
                header: muted.clone(),
                header_active: "fg=default,bold".to_owned(),
                attention: signal.clone(),
                window_current: "fg=default,bold,reverse".to_owned(),
                picking: signal.clone(),
            }
        };
        format!(
            "# Generated by `tx update` / `tx _theme-apply` (theme: {name}).\n\
             set -g @tx-theme \"{name}\"\n\
             set -g @tx-mark \"{MARK}\"\n\
             set -g @tx-chip-style \"{chip}\"\n\
             set -g @tx-accent-style \"{accent}\"\n\
             set -g @tx-muted-style \"{muted}\"\n\
             set -g @tx-signal-style \"{signal}\"\n\
             set -g @tx-working-style \"{working}\"\n\
             set -g @tx-attention-style \"{attention}\"\n\
             set -g @tx-header-style \"{header}\"\n\
             set -g @tx-header-active-style \"{header_active}\"\n\
             set -g @tx-picking-style \"{picking}\"\n\
             set -g status-style \"fg=default,bg={surface}\"\n\
             set -gu status-format\n\
             setw -g window-status-format \" #I #W \"\n\
             setw -g window-status-current-format \" #I #W \"\n\
             setw -g window-status-separator \"\"\n\
             setw -g window-status-style \"{muted}\"\n\
             setw -g window-status-current-style \"{window_current}\"\n\
             set -g pane-border-lines {lines}\n\
             set -g pane-border-indicators off\n\
             set -g pane-border-style \"{border}\"\n\
             set -g pane-active-border-style \"{border_active}\"\n\
             set -g popup-border-lines {POPUP_LINES}\n\
             set -g popup-style \"fg=default,bg=default\"\n\
             set -g popup-border-style \"{popup_border}\"\n\
             set -g set-titles on\n\
             set -g set-titles-string \"#{{@tx-mark}} · #S\"\n",
            name = self.name,
            lines = layout.lines,
            border = layout.border,
            border_active = layout.border_active,
            header = layout.header,
            header_active = layout.header_active,
            attention = layout.attention,
            window_current = layout.window_current,
            picking = layout.picking,
            popup_border = self.tmux_fg_style(Role::RuleActive),
            working = self.tmux_fg_style(Role::Working),
        )
    }

    /// Generates `<data>/conf/theme.lua` for the nvim forms (`plugins/spawn/nvim/tx-new-agent.lua`):
    /// each role as highlight attributes, 24-bit (`fg`, `bg`) where the theme has them and the
    /// terminal's palette (`ctermfg`, `ctermbg`) otherwise. `nil` is the terminal's default.
    pub fn nvim_lua(&self) -> String {
        let attributes = |role: Role, ground: &str| {
            let spec = self.spec(role);
            if self.depth == ColorDepth::None {
                return "nil".to_owned();
            }
            match (spec.hex(), spec.ansi256.or(spec.ansi16.index())) {
                (Some(hex), _) => format!("{{ {ground} = \"{hex}\" }}"),
                (None, Some(index)) => format!("{{ cterm{ground} = {index} }}"),
                (None, None) => "nil".to_owned(),
            }
        };
        format!(
            "-- Generated by `tx update` (theme: {name}).\n\
             return {{\n\
             \x20 name = \"{name}\",\n\
             \x20 truecolor = {truecolor},\n\
             \x20 text = {text},\n\
             \x20 muted = {muted},\n\
             \x20 rule = {rule},\n\
             \x20 accent = {accent},\n\
             \x20 signal = {signal},\n\
             \x20 selection = {selection},\n\
             }}\n",
            name = self.name,
            truecolor = self.text.rgb.is_some() || self.accent.rgb.is_some(),
            text = attributes(Role::Text, "fg"),
            muted = attributes(Role::Muted, "fg"),
            rule = attributes(Role::Rule, "fg"),
            accent = attributes(Role::Accent, "fg"),
            signal = attributes(Role::Signal, "fg"),
            selection = attributes(Role::Selection, "bg"),
        )
    }
}

/// A role's colour as fzf writes one: `#rrggbb`, a palette number, or `-1` for the default.
fn fzf_value(spec: RoleSpec, depth: ColorDepth) -> String {
    let palette = || spec.ansi256.or(spec.ansi16.index());
    let value = match depth {
        ColorDepth::Truecolor => spec
            .hex()
            .or_else(|| palette().map(|index| index.to_string())),
        ColorDepth::Ansi256 => spec
            .ansi256
            .or_else(|| {
                spec.rgb
                    .map(|(red, green, blue)| rgb_to_256_index(red, green, blue))
            })
            .or(spec.ansi16.index())
            .map(|index| index.to_string()),
        ColorDepth::Ansi16 | ColorDepth::None => spec.ansi16.index().map(|index| index.to_string()),
    };
    value.unwrap_or_else(|| "-1".to_owned())
}

fn parse_color_override(raw: &str, base: RoleSpec) -> Option<RoleSpec> {
    let trimmed = raw.trim();
    if let Some((red, green, blue)) = parse_hex_rgb(trimmed) {
        return Some(RoleSpec {
            rgb: Some((red, green, blue)),
            ansi256: Some(rgb_to_256_index(red, green, blue)),
            ansi16: rgb_to_ansi16(red, green, blue),
            reverse_on_16: false,
            ..base
        });
    }
    if let Some(ansi16) = Ansi16::parse(trimmed) {
        return Some(RoleSpec {
            rgb: None,
            ansi256: None,
            ansi16,
            reverse_on_16: false,
            ..base
        });
    }
    None
}

pub fn parse_hex_rgb(text: &str) -> Option<(u8, u8, u8)> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((red, green, blue))
}

pub fn rgb_to_256_index(red: u8, green: u8, blue: u8) -> u8 {
    let level = |channel: u8| (u16::from(channel) * 5 + 127) / 255;
    let index = 16 + 36 * level(red) + 6 * level(green) + level(blue);
    u8::try_from(index).expect("cube indices are at most 231")
}

pub fn rgb_to_256(red: u8, green: u8, blue: u8) -> Color {
    Color::Indexed(rgb_to_256_index(red, green, blue))
}

fn rgb_to_ansi16(red: u8, green: u8, blue: u8) -> Ansi16 {
    let high_r = red >= 128;
    let high_g = green >= 128;
    let high_b = blue >= 128;
    match (high_r, high_g, high_b) {
        (false, false, false) => Ansi16::Black,
        (true, false, false) => Ansi16::Red,
        (false, true, false) => Ansi16::Green,
        (true, true, false) => Ansi16::Yellow,
        (false, false, true) => Ansi16::Blue,
        (true, false, true) => Ansi16::Magenta,
        (false, true, true) => Ansi16::Cyan,
        (true, true, true) => Ansi16::White,
    }
}

fn rgb_to_16(red: u8, green: u8, blue: u8) -> Color {
    rgb_to_ansi16(red, green, blue)
        .ratatui_color()
        .unwrap_or(Color::Reset)
}

/// Process-wide resolved theme, loaded once: the configuration the environment names, the
/// terminal's background (`TX_THEME_POLARITY`, else the recorded one), the detected colour depth.
pub fn active() -> &'static Theme {
    static ACTIVE: OnceLock<Theme> = OnceLock::new();
    ACTIVE.get_or_init(load_from_env)
}

/// Loads the active theme from the environment without caching. An unknown background resolves
/// `auto` to the `terminal` theme, never to a guessed dark one.
pub fn load_from_env() -> Theme {
    let depth = ColorDepth::detect();
    let Ok(configuration) =
        tx_core::host::setup::core_config(&tx_core::host::args::Global::default())
    else {
        return Theme::terminal(Polarity::Unknown, depth);
    };
    let directories = configuration.directories;
    let polarity = std::env::var("TX_THEME_POLARITY").map_or_else(
        |_| Polarity::recorded(&directories.data),
        |value| Polarity::parse(&value),
    );
    Theme::load(&directories.config, polarity, depth)
        .unwrap_or_else(|_| Theme::terminal(polarity, depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLES: [Role; 12] = [
        Role::Text,
        Role::Muted,
        Role::Rule,
        Role::RuleActive,
        Role::Signal,
        Role::Accent,
        Role::OnAccent,
        Role::Working,
        Role::Info,
        Role::Failed,
        Role::Surface,
        Role::Selection,
    ];

    fn resolved(settings: &ThemeSettings, config_dir: &Path, polarity: Polarity) -> Theme {
        Theme::resolve(settings, config_dir, polarity, ColorDepth::Truecolor).unwrap()
    }

    #[test]
    fn no_color_never_sets_fg_or_bg_and_preserves_modifiers() {
        for theme in [
            Theme::terminal(Polarity::Dark, ColorDepth::None),
            Theme::terminal(Polarity::Light, ColorDepth::None),
            Theme::terminal(Polarity::Unknown, ColorDepth::None),
            Theme::tokyo_night(ColorDepth::None),
        ] {
            for role in ROLES {
                let style = theme.style(role);
                assert_eq!(style.fg, None, "{}: {role:?}", theme.name);
                assert_eq!(style.bg, None, "{}: {role:?}", theme.name);
                assert_eq!(theme.ansi_fg(role), "", "{}: {role:?}", theme.name);
                assert_eq!(theme.ansi_bg(role), "", "{}: {role:?}", theme.name);
            }
            let signal = theme.style(Role::Signal).add_modifier;
            assert!(signal.contains(Modifier::BOLD | Modifier::REVERSED));
            assert_eq!(
                theme.selection_style(),
                Style::new().add_modifier(Modifier::REVERSED)
            );
            assert!(!theme.has_fills());
            assert_eq!(theme.fzf_color_arg(true), "--no-color");
        }
    }

    #[test]
    fn the_terminal_theme_uses_the_terminals_own_colours_and_fills_only_a_known_background() {
        let dark = Theme::terminal(Polarity::Dark, ColorDepth::Truecolor);
        assert_eq!(dark.color(Role::Accent), Some(Color::Blue));
        assert_eq!(dark.color(Role::Signal), Some(Color::Yellow));
        assert_eq!(dark.color(Role::Working), Some(Color::Green));
        assert_eq!(dark.selection_style(), Style::new().bg(Color::Indexed(236)));
        assert!(dark.has_fills());

        // ANSI yellow on a light background is unreadable: reverse video carries the signal.
        let light = Theme::terminal(Polarity::Light, ColorDepth::Truecolor);
        assert_eq!(light.color(Role::Signal), None);
        let signal = light.style(Role::Signal).add_modifier;
        assert!(signal.contains(Modifier::BOLD | Modifier::REVERSED));
        assert_eq!(
            light.selection_style(),
            Style::new().bg(Color::Indexed(254))
        );

        let unknown = Theme::terminal(Polarity::Unknown, ColorDepth::Truecolor);
        assert!(!unknown.has_fills());
        assert_eq!(
            unknown.selection_style(),
            Style::new().add_modifier(Modifier::REVERSED)
        );
        // Sixteen colours have no quiet fill either.
        assert!(!Theme::terminal(Polarity::Dark, ColorDepth::Ansi16).has_fills());
    }

    #[test]
    fn a_reported_background_is_recorded_and_an_unknown_one_is_not() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(Polarity::recorded(data.path()), Polarity::Unknown);
        Polarity::Light.record(data.path()).unwrap();
        assert_eq!(Polarity::recorded(data.path()), Polarity::Light);
        Polarity::Unknown.record(data.path()).unwrap();
        assert_eq!(Polarity::recorded(data.path()), Polarity::Light);
    }

    #[test]
    fn the_logo_is_a_rectangle_of_whole_cells() {
        assert_eq!(LOGO_PIXELS.len() % 2, 0);
        let width = LOGO_PIXELS[0].len();
        assert!(LOGO_PIXELS.iter().all(|row| row.len() == width));
        assert!(LOGO_PIXELS.len() / 2 <= 7 && width <= 48);
    }

    #[test]
    fn custom_theme_file_and_inline_overrides_apply() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("themes")).unwrap();
        std::fs::write(
            dir.path().join("themes/mine.yaml"),
            "base: tokyo-night\ncolors:\n  working: green\ndark:\n  accent: \"#ff75b5\"\nlight:\n  accent: \"#a01060\"\n",
        )
        .unwrap();
        let mut settings = ThemeSettings {
            name: "mine".to_owned(),
            ..ThemeSettings::default()
        };
        settings
            .colors
            .insert("signal".to_owned(), "#ffb000".to_owned());
        let theme = resolved(&settings, dir.path(), Polarity::Dark);
        assert_eq!(theme.name, "mine");
        assert_eq!(theme.color(Role::Accent), Some(Color::Rgb(255, 117, 181)));
        assert_eq!(theme.color(Role::Signal), Some(Color::Rgb(255, 176, 0)));
        assert_eq!(theme.color(Role::Working), Some(Color::Green));
        let light = resolved(&settings, dir.path(), Polarity::Light);
        assert_eq!(light.color(Role::Accent), Some(Color::Rgb(160, 16, 96)));
    }

    #[test]
    fn a_theme_that_cannot_be_applied_whole_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let resolve = |settings: &ThemeSettings| {
            Theme::resolve(settings, dir.path(), Polarity::Dark, ColorDepth::Truecolor)
        };
        let with_color = |role: &str, value: &str| {
            let mut settings = ThemeSettings::default();
            settings.colors.insert(role.to_owned(), value.to_owned());
            settings
        };
        assert_eq!(
            resolve(&with_color("acent", "#32b8ac")),
            Err(ThemeError::UnknownRole("acent".to_owned()))
        );
        let error = resolve(&with_color("accent", "teal")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "theme: `teal` for `accent` is neither #rrggbb nor one of the terminal's sixteen \
             colour names"
        );
        let missing = ThemeSettings {
            name: "nowhere".to_owned(),
            ..ThemeSettings::default()
        };
        let error = resolve(&missing).unwrap_err().to_string();
        assert!(error.contains("themes/nowhere.yaml"), "{error}");
        std::fs::create_dir_all(dir.path().join("themes")).unwrap();
        std::fs::write(dir.path().join("themes/nowhere.yaml"), "colours: {}\n").unwrap();
        let error = resolve(&missing).unwrap_err().to_string();
        assert!(error.contains("unknown field `colours`"), "{error}");
    }

    #[test]
    fn tmux_takes_bars_with_fills_and_lines_without() {
        let bars = Theme::terminal(Polarity::Dark, ColorDepth::Truecolor).tmux_conf();
        assert!(bars.contains("set -g @tx-mark \"tx\""), "{bars}");
        assert!(
            bars.contains("set -g @tx-chip-style \"fg=black,bg=blue,bold\""),
            "{bars}"
        );
        assert!(
            bars.contains("set -g status-style \"fg=default,bg=colour0\""),
            "{bars}"
        );
        assert!(bars.contains("set -g pane-border-lines spaces"), "{bars}");
        assert!(
            bars.contains("set -g pane-active-border-style \"bg=colour236\""),
            "{bars}"
        );
        assert!(bars.contains("set -g popup-border-lines rounded"), "{bars}");
        assert!(bars.contains("set -gu status-format"), "{bars}");

        let lines = Theme::terminal(Polarity::Unknown, ColorDepth::Truecolor).tmux_conf();
        assert!(lines.contains("set -g pane-border-lines single"), "{lines}");
        assert!(
            lines.contains("set -g status-style \"fg=default,bg=default\""),
            "{lines}"
        );
        assert!(
            lines.contains("set -g @tx-chip-style \"fg=default,bold,reverse\""),
            "{lines}"
        );
    }
}
