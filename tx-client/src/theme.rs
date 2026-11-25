//! The tx visual identity and theme engine (Junction): mark, state alphabet, three-weight border
//! grammar, semantic colour roles and single-source distribution to ratatui, tmux, fzf and nvim.
//!
//! Resolution order:
//! 1. Colour allowance: non-empty `NO_COLOR`, `TERM=dumb` or `TX_COLORS=none` yields
//!    [`ColorDepth::None`], where every resolved [`Style`] omits `fg` and `bg` completely
//!    (avoiding `crossterm 0.29`'s `CSI ; m` reset after modifiers) and keeps only attributes.
//! 2. Polarity: explicit preset (`tx-dark`, `tx-light`, `terminal`, `tokyo-night`) or a user file
//!    under `<config>/themes/<name>.yaml`; `auto` selects `tx-dark` on dark terminals, `tx-light`
//!    on light terminals, and `terminal` when the background polarity is unknown.
//! 3. Colour depth: `TX_COLORS` (`truecolor` | `256` | `16` | `none`), legacy `TX_HOME_COLORS=256`,
//!    `COLORTERM` (`truecolor` | `24bit`), or `TERM`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use ratatui_core::style::{Color, Modifier, Style};
use serde::Deserialize;
use tx_core::host::user_config::{self, GlyphTier, StatusStyle, ThemeSettings};

use crate::render::tag_hash;

/// Canonical two-cell Junction sigil (`U+254B`, `U+2573`).
pub const MARK_UNICODE: &str = "╋╳";
pub const MARK_ASCII: &str = "tx";
/// Tight contextual one-cell sigil (`U+254B`), used only where `╋╳` is already on screen.
pub const MARK_CELL_UNICODE: &str = "╋";
pub const MARK_CELL_ASCII: &str = "t";

/// Four-row box-drawn Junction `tx` banner (9×4, fits `home-backdrop`'s <= 7×48 contract).
pub const BANNER_UNICODE: &[&str] = &[" ╻       ", "━╋━━  ╲ ╱", " ┃     ╳ ", " ┗━━  ╱ ╲"];

/// Four-row ASCII fallback banner (9×4).
pub const BANNER_ASCII: &[&str] = &[" |       ", "-+--  \\ /", " |     X ", " +--  / \\"];

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

/// The three-weight border grammar: heavy = tx's frame, light = content, rounded = operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderRole {
    Frame,
    Content,
    Operator,
}

impl BorderRole {
    /// tmux `display-popup -b` / `pane-border-lines` name for this role.
    pub fn tmux_lines(self) -> &'static str {
        match self {
            Self::Frame => "heavy",
            Self::Content => "single",
            Self::Operator => "rounded",
        }
    }
}

/// Unified glyph alphabet: one shape per state across every plugin, independent of colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub mark: &'static str,
    pub mark_cell: &'static str,
    pub working: &'static str,
    pub starting: [&'static str; 4],
    pub working_bar: [&'static str; 4],
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
    /// The tees a title sits between on a heavy line.
    pub inset_left: &'static str,
    pub inset_right: &'static str,
    /// The same on a light or rounded line.
    pub inset_light_left: &'static str,
    pub inset_light_right: &'static str,
    pub rule_heavy: &'static str,
}

pub const UNICODE_GLYPHS: Glyphs = Glyphs {
    mark: MARK_UNICODE,
    mark_cell: MARK_CELL_UNICODE,
    working: "●",
    starting: ["▖", "▘", "▝", "▗"],
    working_bar: ["━───", "─━──", "──━─", "───━"],
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
    inset_left: "┫",
    inset_right: "┣",
    inset_light_left: "┤",
    inset_light_right: "├",
    rule_heavy: "━",
};

pub const ASCII_GLYPHS: Glyphs = Glyphs {
    mark: MARK_ASCII,
    mark_cell: MARK_CELL_ASCII,
    working: "*",
    starting: ["-", "\\", "|", "/"],
    working_bar: ["=---", "-=--", "--=-", "---="],
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
    inset_left: "[",
    inset_right: "]",
    inset_light_left: "[",
    inset_light_right: "]",
    rule_heavy: "-",
};

impl Glyphs {
    pub const fn for_tier(tier: GlyphTier) -> Self {
        match tier {
            GlyphTier::Unicode => UNICODE_GLYPHS,
            GlyphTier::Ascii => ASCII_GLYPHS,
        }
    }

    pub const fn banner(tier: GlyphTier) -> &'static [&'static str] {
        match tier {
            GlyphTier::Unicode => BANNER_UNICODE,
            GlyphTier::Ascii => BANNER_ASCII,
        }
    }

    /// The tees that match `role`'s line weight: heavy for the frame, light otherwise.
    pub fn inset(self, role: BorderRole) -> (&'static str, &'static str) {
        match role {
            BorderRole::Frame => (self.inset_left, self.inset_right),
            BorderRole::Content | BorderRole::Operator => {
                (self.inset_light_left, self.inset_light_right)
            }
        }
    }

    /// A title set into a border of `role`: `┫ title ┣` on a heavy line, `┤ title ├` on a light
    /// or rounded one, `[ title ]` in ASCII.
    pub fn inset_title(self, role: BorderRole, title: &str) -> String {
        let (left, right) = self.inset(role);
        format!("{left} {} {right}", title.trim())
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
    pub status_style: StatusStyle,
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
    "auto".to_owned()
}

impl Theme {
    /// Shipped dark Junction theme: achromatic frame with amber `signal`, restrained Verdigris
    /// Teal accent inside tx's own TUI screens.
    pub fn tx_dark(depth: ColorDepth) -> Self {
        Self {
            name: "tx-dark".to_owned(),
            status_style: StatusStyle::Rule,
            glyph_tier: GlyphTier::Unicode,
            glyphs: UNICODE_GLYPHS,
            depth,
            polarity: Polarity::Dark,
            text: RoleSpec::default_role(),
            muted: RoleSpec {
                dim: true,
                ..RoleSpec::default_role()
            },
            rule: RoleSpec {
                rgb: Some((74, 83, 98)),
                ansi256: Some(240),
                ansi16: Ansi16::Default,
                dim: true,
                ..RoleSpec::default_role()
            },
            rule_active: RoleSpec {
                bold: true,
                ..RoleSpec::default_role()
            },
            signal: RoleSpec {
                rgb: Some((224, 175, 104)),
                ansi256: Some(179),
                ansi16: Ansi16::Yellow,
                bold: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            accent: RoleSpec {
                rgb: Some((50, 184, 172)),
                ansi256: Some(73),
                ansi16: Ansi16::Cyan,
                ..RoleSpec::default_role()
            },
            on_accent: RoleSpec::with_rgb((14, 19, 22), 233, Ansi16::Black, false, false),
            working: RoleSpec::with_rgb((98, 195, 112), 71, Ansi16::Green, false, false),
            info: RoleSpec::with_rgb((94, 194, 230), 74, Ansi16::Cyan, false, false),
            failed: RoleSpec {
                rgb: Some((229, 107, 111)),
                ansi256: Some(167),
                ansi16: Ansi16::Red,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            surface: RoleSpec::with_rgb((18, 21, 25), 234, Ansi16::Black, false, false),
            selection: RoleSpec {
                rgb: Some((38, 46, 56)),
                ansi256: Some(236),
                ansi16: Ansi16::Default,
                reverse_on_16: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            tag_palette: TAG_CUBE_DARK,
        }
    }

    /// Shipped light Junction theme: high-contrast ochre `signal` (reverse in 16-colour mode so
    /// ANSI yellow never vanishes on white) and deep Verdigris Teal inside tx's own screens.
    pub fn tx_light(depth: ColorDepth) -> Self {
        Self {
            name: "tx-light".to_owned(),
            status_style: StatusStyle::Rule,
            glyph_tier: GlyphTier::Unicode,
            glyphs: UNICODE_GLYPHS,
            depth,
            polarity: Polarity::Light,
            text: RoleSpec::default_role(),
            muted: RoleSpec {
                dim: true,
                ..RoleSpec::default_role()
            },
            rule: RoleSpec {
                rgb: Some((158, 168, 180)),
                ansi256: Some(247),
                ansi16: Ansi16::Default,
                dim: true,
                ..RoleSpec::default_role()
            },
            rule_active: RoleSpec {
                bold: true,
                ..RoleSpec::default_role()
            },
            signal: RoleSpec {
                rgb: Some((154, 91, 0)),
                ansi256: Some(94),
                ansi16: Ansi16::Default,
                bold: true,
                reverse_on_16: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            accent: RoleSpec {
                rgb: Some((13, 122, 114)),
                ansi256: Some(30),
                ansi16: Ansi16::Cyan,
                ..RoleSpec::default_role()
            },
            on_accent: RoleSpec::with_rgb((247, 250, 250), 231, Ansi16::White, false, false),
            working: RoleSpec::with_rgb((31, 122, 56), 28, Ansi16::Green, false, false),
            info: RoleSpec::with_rgb((14, 107, 148), 24, Ansi16::Cyan, false, false),
            failed: RoleSpec {
                rgb: Some((194, 59, 66)),
                ansi256: Some(131),
                ansi16: Ansi16::Red,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            surface: RoleSpec::with_rgb((232, 236, 239), 254, Ansi16::White, false, false),
            selection: RoleSpec {
                rgb: Some((220, 227, 234)),
                ansi256: Some(253),
                ansi16: Ansi16::Default,
                reverse_on_16: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            tag_palette: TAG_CUBE_LIGHT,
        }
    }

    /// Pure terminal-inheriting theme: uses only the terminal's default foreground, bold, dim,
    /// reverse, and safe ANSI slots. Used when explicitly chosen or when background polarity is
    /// unknown under `auto`.
    pub fn terminal(depth: ColorDepth) -> Self {
        Self {
            name: "terminal".to_owned(),
            status_style: StatusStyle::Rule,
            glyph_tier: GlyphTier::Unicode,
            glyphs: UNICODE_GLYPHS,
            depth,
            polarity: Polarity::Unknown,
            text: RoleSpec::default_role(),
            muted: RoleSpec {
                dim: true,
                ..RoleSpec::default_role()
            },
            rule: RoleSpec {
                dim: true,
                ..RoleSpec::default_role()
            },
            rule_active: RoleSpec {
                bold: true,
                ..RoleSpec::default_role()
            },
            signal: RoleSpec {
                ansi16: Ansi16::Default,
                bold: true,
                reversed: true,
                reverse_on_16: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            accent: RoleSpec {
                ansi16: Ansi16::Cyan,
                bold: true,
                ..RoleSpec::default_role()
            },
            on_accent: RoleSpec::default_role(),
            working: RoleSpec {
                ansi16: Ansi16::Green,
                ..RoleSpec::default_role()
            },
            info: RoleSpec {
                ansi16: Ansi16::Cyan,
                ..RoleSpec::default_role()
            },
            failed: RoleSpec {
                ansi16: Ansi16::Red,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            surface: RoleSpec::default_role(),
            selection: RoleSpec {
                reversed: true,
                reverse_on_16: true,
                reverse_on_no_color: true,
                ..RoleSpec::default_role()
            },
            tag_palette: TAG_CUBE_DARK,
        }
    }

    /// Previous default palette (`tokyo-night`), preserved as a first-class shipped preset.
    pub fn tokyo_night(depth: ColorDepth) -> Self {
        Self {
            name: "tokyo-night".to_owned(),
            status_style: StatusStyle::Rule,
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
        theme.status_style = settings.status_style;
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

    /// What `auto` means: the designed theme for a known background, the terminal's own colours
    /// for an unknown one.
    pub fn automatic(polarity: Polarity, depth: ColorDepth) -> Self {
        match polarity {
            Polarity::Dark => Self::tx_dark(depth),
            Polarity::Light => Self::tx_light(depth),
            Polarity::Unknown => Self::terminal(depth),
        }
    }

    fn from_name(
        name: &str,
        config_dir: &Path,
        polarity: Polarity,
        depth: ColorDepth,
    ) -> Result<Self, ThemeError> {
        match name {
            "auto" => Ok(Self::automatic(polarity, depth)),
            "tx-dark" => Ok(Self::tx_dark(depth)),
            "tx-light" => Ok(Self::tx_light(depth)),
            "terminal" => Ok(Self::terminal(depth)),
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

    /// Row selection style for list/table TUIs. Over an animated backdrop, uses a solid accent
    /// bar; otherwise uses the theme's subtle selection background (or reverse video under
    /// `terminal`, 16-colour or `NO_COLOR`).
    pub fn selection_style(&self, over_backdrop: bool) -> Style {
        if self.depth == ColorDepth::None || self.depth == ColorDepth::Ansi16 {
            return Style::new().add_modifier(Modifier::REVERSED);
        }
        if over_backdrop {
            if let Some(background) = self.color(Role::Accent) {
                let mut style = Style::new().bg(background);
                if let Some(foreground) = self.color(Role::OnAccent) {
                    style = style.fg(foreground);
                }
                return style;
            }
            return Style::new().add_modifier(Modifier::REVERSED);
        }
        if self.selection.reversed {
            return Style::new().add_modifier(Modifier::REVERSED);
        }
        match self.color(Role::Selection) {
            Some(background) => Style::new().bg(background),
            None => Style::new().add_modifier(Modifier::REVERSED),
        }
    }

    /// Text drawn on an accent-filled badge (e.g. graph sidebar title).
    pub fn on_accent_badge(&self) -> Style {
        if self.depth == ColorDepth::None || self.depth == ColorDepth::Ansi16 {
            return Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED);
        }
        match (self.color(Role::Accent), self.color(Role::OnAccent)) {
            (Some(background), Some(foreground)) => Style::new()
                .fg(foreground)
                .bg(background)
                .add_modifier(Modifier::BOLD),
            (Some(background), None) => Style::new().bg(background).add_modifier(Modifier::BOLD),
            _ => Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        }
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

    /// fzf `--color=...` or `--no-color` argument matching the active theme.
    pub fn fzf_color_arg(&self, with_surface_bg: bool) -> String {
        if self.depth == ColorDepth::None {
            return "--no-color".to_owned();
        }
        let accent = fzf_value(self.spec(Role::Accent), self.depth);
        let text = fzf_value(self.spec(Role::Text), self.depth);
        let muted = fzf_value(self.spec(Role::Muted), self.depth);
        let selection = fzf_value(self.spec(Role::Selection), self.depth);
        let mut parts = Vec::new();
        if with_surface_bg && let Some(surface) = fzf_value(self.spec(Role::Surface), self.depth) {
            parts.push(format!("bg:{surface}"));
        }
        if let Some(color) = muted.as_deref() {
            parts.push(format!("fg:{color}"));
            parts.push(format!("header:{color}"));
            parts.push(format!("footer:{color}"));
            parts.push(format!("prompt:{color}"));
        }
        if let Some(color) = text.as_deref() {
            parts.push(format!("query:{color}"));
            parts.push(format!("fg+:{color}:regular"));
        }
        if let Some(color) = accent.as_deref() {
            parts.push(format!("pointer:{color}"));
            parts.push(format!("hl:{color}"));
            parts.push(format!("hl+:{color}"));
        }
        if let Some(color) = selection.as_deref() {
            parts.push(format!("bg+:{color}:regular"));
        }
        if parts.is_empty() {
            "--color=16".to_owned()
        } else {
            format!("--color={}", parts.join(","))
        }
    }

    /// Generates `<data>/conf/theme.tmux` defining every tmux visual option in one place.
    ///
    /// Keeps native `status-left`, `window-status-format`, `window-status-current-format` and
    /// `status-right` intact so users and plugins (`attention.tmux`) customize segments freely.
    pub fn tmux_conf(&self) -> String {
        let glyphs = self.glyphs;
        let mark = glyphs.mark;
        let rule_style = self.tmux_fg_style(Role::Rule);
        let rule_active_style = self.tmux_fg_style(Role::RuleActive);
        let signal_style = self.tmux_fg_style(Role::Signal);
        let muted_style = self.tmux_fg_style(Role::Muted);
        let text_style = self.tmux_fg_style(Role::Text);
        let surface_bg = self.tmux_color(Role::Surface);
        let status_bg = match self.status_style {
            StatusStyle::Rule => "default".to_owned(),
            StatusStyle::Bar => surface_bg.clone(),
        };
        let status_style = format!("fg={},bg={status_bg}", self.tmux_color(Role::Text));
        let popup_style = format!("fg={},bg={surface_bg}", self.tmux_color(Role::Text));
        let popup_border_style =
            format!("fg={},bg={surface_bg}", self.tmux_color(Role::RuleActive));
        let operator_border_style =
            format!("fg={},bg={surface_bg}", self.tmux_color(Role::RuleActive));

        let active_window_format = format!("{} #I:#W {}", glyphs.inset_left, glyphs.inset_right);
        let window_current_style = if self.depth == ColorDepth::None {
            "bold".to_owned()
        } else {
            rule_active_style.clone()
        };

        let status_format_lines = match self.status_style {
            StatusStyle::Rule => {
                let rule_char = glyphs.rule_heavy;
                // tmux's own window list, with its styles and its clickable ranges.
                let windows = "#{W:#[range=window|#{window_index} #{E:window-status-style}]#{T:window-status-format}#[norange default],#[range=window|#{window_index} #{E:window-status-current-style}]#{T:window-status-current-format}#[norange default]}";
                let pulse =
                    "#{?@tx-pulse,#[#{@tx-rule-active-style}]#{@tx-pulse}#[#{@tx-rule-style}],}";
                let fill = format!(
                    "#{{R:{rule_char},#{{e|-:#{{e|-:#{{e|-:#{{e|-:#{{client_width}},#{{w:#{{T:status-left}}}}}},#{{w:#{{E:@tx-windows}}}}}},#{{w:#{{@tx-pulse}}}}}},#{{w:#{{T:status-right}}}}}}}}"
                );
                format!(
                    "set -g @tx-windows \"{windows}\"\n\
                     set -g status-format[0] \"#[align=left range=left #{{E:status-left-style}}]#{{T:status-left}}#[norange default]#{{E:@tx-windows}}#[#{{@tx-rule-style}}]{pulse}{fill}#[default range=right #{{E:status-right-style}}]#{{T:status-right}}#[norange default]\"\n"
                )
            }
            StatusStyle::Bar => "set -gu status-format\n".to_owned(),
        };

        format!(
            "# Generated by `tx update` / `tx _theme-apply` (theme: {name}).\n\
             set -g @tx-theme \"{name}\"\n\
             set -g @tx-mark \"{mark}\"\n\
             set -g @tx-inset-left \"{inset_left}\"\n\
             set -g @tx-inset-right \"{inset_right}\"\n\
             set -g @tx-operator-inset-left \"{operator_inset_left}\"\n\
             set -g @tx-operator-inset-right \"{operator_inset_right}\"\n\
             set -g @tx-rule-style \"{rule_style}\"\n\
             set -g @tx-rule-active-style \"{rule_active_style}\"\n\
             set -g @tx-signal-style \"{signal_style}\"\n\
             set -g @tx-muted-style \"{muted_style}\"\n\
             set -g @tx-text-style \"{text_style}\"\n\
             set -g @tx-surface-bg \"{surface_bg}\"\n\
             set -g @tx-popup-border-style \"{popup_border_style}\"\n\
             set -g @tx-popup-operator-border-style \"{operator_border_style}\"\n\
             set -g status-style \"{status_style}\"\n\
             setw -g window-status-format \" #I:#W \"\n\
             setw -g window-status-current-format \"{active_window_format}\"\n\
             setw -g window-status-separator \"\"\n\
             setw -g window-status-style \"{muted_style}\"\n\
             setw -g window-status-current-style \"{window_current_style}\"\n\
             set -g pane-border-lines {frame_lines}\n\
             set -g pane-border-indicators both\n\
             set -g pane-border-style \"{rule_style}\"\n\
             set -g pane-active-border-style \"{rule_active_style}\"\n\
             set -g popup-border-lines {frame_lines}\n\
             set -g popup-style \"{popup_style}\"\n\
             set -g popup-border-style \"{popup_border_style}\"\n\
             set -g set-titles on\n\
             set -g set-titles-string \"#{{@tx-mark}} #S\"\n\
             {status_format_lines}",
            name = self.name,
            inset_left = glyphs.inset_left,
            inset_right = glyphs.inset_right,
            operator_inset_left = glyphs.inset_light_left,
            operator_inset_right = glyphs.inset_light_right,
            frame_lines = BorderRole::Frame.tmux_lines(),
        )
    }

    /// Generates `<data>/conf/theme.lua` for Neovim forms (`plugins/spawn/nvim/tx-new-agent.lua`).
    pub fn nvim_lua(&self) -> String {
        let lua_hex = |role: Role| match (self.depth, self.spec(role).hex()) {
            (ColorDepth::None, _) => "nil".to_owned(),
            (_, Some(hex)) => format!("\"{hex}\""),
            (_, None) => "nil".to_owned(),
        };
        format!(
            "-- Generated by `tx update` (theme: {name}).\n\
             return {{\n\
             \x20 name = \"{name}\",\n\
             \x20 mark = \"{mark}\",\n\
             \x20 accent = {accent},\n\
             \x20 signal = {signal},\n\
             \x20 fg = {fg},\n\
             \x20 muted = {muted},\n\
             \x20 rule = {rule},\n\
             \x20 selection = {selection},\n\
             \x20 working = {working},\n\
             \x20 failed = {failed},\n\
             }}\n",
            name = self.name,
            mark = self.glyphs.mark,
            accent = lua_hex(Role::Accent),
            signal = lua_hex(Role::Signal),
            fg = lua_hex(Role::Text),
            muted = lua_hex(Role::Muted),
            rule = lua_hex(Role::Rule),
            selection = lua_hex(Role::Selection),
            working = lua_hex(Role::Working),
            failed = lua_hex(Role::Failed),
        )
    }
}

fn fzf_value(spec: RoleSpec, depth: ColorDepth) -> Option<String> {
    match depth {
        ColorDepth::None => None,
        ColorDepth::Truecolor => spec
            .hex()
            .or_else(|| spec.ansi256.map(|index| index.to_string())),
        ColorDepth::Ansi256 => spec.ansi256.map(|index| index.to_string()).or_else(|| {
            spec.rgb
                .map(|(r, g, b)| rgb_to_256_index(r, g, b).to_string())
        }),
        ColorDepth::Ansi16 => {
            if spec.reverse_on_16 || spec.ansi16 == Ansi16::Default {
                None
            } else {
                Some(spec.ansi16.tmux_name().to_owned())
            }
        }
    }
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
        return Theme::terminal(depth);
    };
    let directories = configuration.directories;
    let polarity = std::env::var("TX_THEME_POLARITY").map_or_else(
        |_| Polarity::recorded(&directories.data),
        |value| Polarity::parse(&value),
    );
    Theme::load(&directories.config, polarity, depth)
        .unwrap_or_else(|_| Theme::automatic(polarity, depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_never_sets_fg_or_bg_and_preserves_modifiers() {
        for theme in [
            Theme::tx_dark(ColorDepth::None),
            Theme::tx_light(ColorDepth::None),
            Theme::terminal(ColorDepth::None),
            Theme::tokyo_night(ColorDepth::None),
        ] {
            for role in [
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
            ] {
                let style = theme.style(role);
                assert_eq!(style.fg, None, "{}: {role:?}", theme.name);
                assert_eq!(style.bg, None, "{}: {role:?}", theme.name);
                assert_eq!(theme.ansi_fg(role), "", "{}: {role:?}", theme.name);
                assert_eq!(theme.ansi_bg(role), "", "{}: {role:?}", theme.name);
            }
            assert!(
                theme
                    .style(Role::Signal)
                    .add_modifier
                    .contains(Modifier::BOLD | Modifier::REVERSED)
            );
            assert!(
                theme
                    .style(Role::RuleActive)
                    .add_modifier
                    .contains(Modifier::BOLD)
            );
            assert_eq!(theme.fzf_color_arg(true), "--no-color");
        }
    }

    fn resolved(settings: &ThemeSettings, config_dir: &Path, polarity: Polarity) -> Theme {
        Theme::resolve(settings, config_dir, polarity, ColorDepth::Truecolor).unwrap()
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
    fn auto_falls_back_to_terminal_when_polarity_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let settings = ThemeSettings::default();
        assert_eq!(
            resolved(&settings, dir.path(), Polarity::Dark).name,
            "tx-dark"
        );
        assert_eq!(
            resolved(&settings, dir.path(), Polarity::Light).name,
            "tx-light"
        );
        assert_eq!(
            resolved(&settings, dir.path(), Polarity::Unknown).name,
            "terminal"
        );
    }

    #[test]
    fn light_signal_uses_reverse_on_16_colors_not_yellow() {
        let light = Theme::tx_light(ColorDepth::Ansi16);
        let style = light.style(Role::Signal);
        assert_eq!(style.fg, None);
        assert!(
            style
                .add_modifier
                .contains(Modifier::REVERSED | Modifier::BOLD)
        );
        assert_eq!(light.tmux_fg_style(Role::Signal), "fg=default,bold,reverse");
    }

    #[test]
    fn banner_fits_home_backdrop_contract_with_equal_width_rows() {
        for banner in [BANNER_UNICODE, BANNER_ASCII] {
            assert!(banner.len() <= 7);
            let width = banner[0].chars().count();
            assert!(width <= 48);
            assert!(banner.iter().all(|row| row.chars().count() == width));
        }
    }

    #[test]
    fn custom_theme_file_and_inline_overrides_apply() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("themes")).unwrap();
        std::fs::write(
            dir.path().join("themes/mine.yaml"),
            "base: tx-dark\ncolors:\n  working: green\ndark:\n  accent: \"#ff75b5\"\nlight:\n  accent: \"#a01060\"\n",
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
    fn tmux_conf_keeps_slots_and_respects_rule_and_bar_modes() {
        let rule_conf = Theme::tx_dark(ColorDepth::Truecolor).tmux_conf();
        assert!(rule_conf.contains("set -g @tx-mark \"╋╳\""));
        assert!(rule_conf.contains("set -g status-format[0]"));
        assert!(rule_conf.contains("#{T:status-left}"));
        assert!(rule_conf.contains("#{T:status-right}"));
        assert!(rule_conf.contains("set -g popup-border-lines heavy"));
        assert!(rule_conf.contains("set -g set-titles-string \"#{@tx-mark} #S\""));

        let mut bar_theme = Theme::tx_dark(ColorDepth::Truecolor);
        bar_theme.status_style = StatusStyle::Bar;
        let bar_conf = bar_theme.tmux_conf();
        assert!(bar_conf.contains("set -gu status-format"));
    }
}
