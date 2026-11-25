//! The drawn keyboard: where each key sits, the tmux key it stands for in each mode, and how to
//! press a tmux key on a Mac or a Windows keyboard.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which keyboard is drawn and named: its bottom row, the mode names and the keystrokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
}

impl Platform {
    /// The keyboard tx runs on; tx targets macOS.
    pub fn native() -> Platform {
        if cfg!(target_os = "macos") { Platform::Mac } else { Platform::Windows }
    }

    pub fn next(self) -> Platform {
        match self {
            Platform::Mac => Platform::Windows,
            Platform::Windows => Platform::Mac,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Platform::Mac => "mac",
            Platform::Windows => "windows",
        }
    }
}

/// The modifier held with every key on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Base,
    Shift,
    Control,
    Alt,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Base, Mode::Shift, Mode::Control, Mode::Alt];

    pub fn name(self, platform: Platform) -> &'static str {
        match (self, platform) {
            (Mode::Base, _) => "base",
            (Mode::Shift, Platform::Mac) => "⇧ shift",
            (Mode::Shift, Platform::Windows) => "Shift",
            (Mode::Control, Platform::Mac) => "⌃ control",
            (Mode::Control, Platform::Windows) => "Ctrl",
            (Mode::Alt, Platform::Mac) => "⌥ option",
            (Mode::Alt, Platform::Windows) => "Alt",
        }
    }

    pub fn next(self) -> Mode {
        let index = Mode::ALL.iter().position(|mode| *mode == self).expect("every mode is listed");
        Mode::ALL[(index + 1) % Mode::ALL.len()]
    }

    /// The mode a tmux key name belongs to.
    pub fn of(key: &str, caps: &[Cap]) -> Mode {
        if key.starts_with("C-") {
            Mode::Control
        } else if key.starts_with("M-") {
            Mode::Alt
        } else if key.starts_with("S-") || caps.iter().any(|cap| cap.shifted == Some(key)) {
            Mode::Shift
        } else {
            Mode::Base
        }
    }
}

/// One key: its row and its span in quarter key-units, the symbol it shows, and the tmux keys it
/// stands for unshifted and shifted. `base` is `None` for keys tmux cannot bind alone (⇧, fn, ⌘).
pub struct Cap {
    pub row: u16,
    pub start: u16,
    pub width: u16,
    pub symbol: &'static str,
    pub base: Option<&'static str>,
    pub shifted: Option<&'static str>,
}

/// The board's width in quarter key-units: a 15-unit main block, a half-unit gap and three
/// arrow keys.
pub const WIDTH_IN_QUARTERS: u16 = 74;
const ARROWS_START: u16 = 62;

type Key = (u16, &'static str, Option<&'static str>, Option<&'static str>);

/// A US ANSI board with the arrow keys, its bottom row the `platform`'s.
pub fn keyboard(platform: Platform) -> Vec<Cap> {
    let pair = |base: &'static str, shifted: &'static str| -> Key { (4, base, Some(base), Some(shifted)) };
    let pairs = |keys: &[(&'static str, &'static str)]| keys.iter().map(|&(base, shifted)| pair(base, shifted)).collect::<Vec<_>>();
    let inert = |width: u16, symbol: &'static str| -> Key { (width, symbol, None, None) };
    let rows: [Vec<Key>; 5] = [
        [
            pairs(&[("`", "~"), ("1", "!"), ("2", "@"), ("3", "#"), ("4", "$"), ("5", "%"), ("6", "^")]),
            pairs(&[("7", "&"), ("8", "*"), ("9", "("), ("0", ")"), ("-", "_"), ("=", "+")]),
            vec![(8, "⌫", Some("BSpace"), None)],
        ]
        .concat(),
        [
            vec![(6, "⇥", Some("Tab"), Some("BTab"))],
            pairs(&[("q", "Q"), ("w", "W"), ("e", "E"), ("r", "R"), ("t", "T"), ("y", "Y")]),
            pairs(&[("u", "U"), ("i", "I"), ("o", "O"), ("p", "P"), ("[", "{"), ("]", "}")]),
            vec![(6, "\\", Some("\\"), Some("|"))],
        ]
        .concat(),
        [
            vec![inert(7, "⇪")],
            pairs(&[("a", "A"), ("s", "S"), ("d", "D"), ("f", "F"), ("g", "G"), ("h", "H")]),
            pairs(&[("j", "J"), ("k", "K"), ("l", "L"), (";", ":"), ("'", "\"")]),
            vec![(9, "⏎", Some("Enter"), None)],
        ]
        .concat(),
        [
            vec![inert(9, "⇧")],
            pairs(&[("z", "Z"), ("x", "X"), ("c", "C"), ("v", "V"), ("b", "B")]),
            pairs(&[("n", "N"), ("m", "M"), (",", "<"), (".", ">"), ("/", "?")]),
            vec![inert(11, "⇧")],
        ]
        .concat(),
        match platform {
            Platform::Mac => vec![
                inert(5, "fn"),
                inert(5, "⌃"),
                inert(5, "⌥"),
                inert(6, "⌘"),
                (28, "", Some("Space"), None),
                inert(6, "⌘"),
                inert(5, "⌥"),
            ],
            Platform::Windows => vec![
                inert(6, "Ctrl"),
                inert(5, "⊞"),
                inert(6, "Alt"),
                (26, "", Some("Space"), None),
                inert(6, "Alt"),
                inert(5, "⊞"),
                inert(6, "Ctrl"),
            ],
        },
    ];
    let mut caps = Vec::new();
    for (row, keys) in (0..).zip(rows) {
        let mut start = 0;
        for (width, symbol, base, shifted) in keys {
            caps.push(Cap { row, start, width, symbol, base, shifted });
            start += width;
        }
    }
    let arrows = [(3, 4, "↑", "Up"), (4, 0, "←", "Left"), (4, 4, "↓", "Down"), (4, 8, "→", "Right")];
    for (row, offset, symbol, key) in arrows {
        caps.push(Cap { row, start: ARROWS_START + offset, width: 4, symbol, base: Some(key), shifted: None });
    }
    caps
}

impl Cap {
    /// The tmux key this cap stands for in `mode`, and the symbol it shows there.
    pub fn key(&self, mode: Mode) -> Option<(String, &'static str)> {
        let base = self.base?;
        match mode {
            Mode::Base => Some((base.to_owned(), self.symbol)),
            Mode::Shift => match self.shifted {
                Some("BTab") => Some(("BTab".to_owned(), self.symbol)),
                Some(shifted) => Some((shifted.to_owned(), shifted)),
                None if matches!(base, "Up" | "Down" | "Left" | "Right" | "Space" | "Enter") => {
                    Some((format!("S-{base}"), self.symbol))
                }
                None => None,
            },
            Mode::Control => Some((format!("C-{base}"), self.symbol)),
            Mode::Alt => Some((format!("M-{base}"), self.symbol)),
        }
    }
}

/// A pressed key as a tmux key name (`e`, `?`, `Up`, `C-h`, …); `None` for keys tmux has no name
/// for here.
pub fn tmux_key_name(key: KeyEvent) -> Option<String> {
    let name = match key.code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(character) => character.to_string(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::Backspace => "BSpace".to_owned(),
        KeyCode::Delete => "DC".to_owned(),
        KeyCode::Insert => "IC".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PPage".to_owned(),
        KeyCode::PageDown => "NPage".to_owned(),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::F(number) => format!("F{number}"),
        _ => return None,
    };
    let mut name = name;
    if key.modifiers.contains(KeyModifiers::ALT) {
        name = format!("M-{name}");
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        name = format!("C-{name}");
    }
    Some(name)
}

/// A pressed key read in `mode`: the modifiers typed with it are dropped, so in the control mode
/// `h` is `C-h` and in the base mode `X` is `x`.
pub fn in_mode(key: &str, mode: Mode, caps: &[Cap]) -> String {
    let mut bare = key;
    while let Some(after) = bare.strip_prefix("C-").or(bare.strip_prefix("M-")).or(bare.strip_prefix("S-")) {
        bare = after;
    }
    let cap = caps.iter().find(|cap| cap.base == Some(bare) || cap.shifted == Some(bare));
    match cap.and_then(|cap| cap.key(mode)) {
        Some((name, _)) => name,
        None => match mode {
            Mode::Base => bare.to_owned(),
            Mode::Shift => format!("S-{bare}"),
            Mode::Control => format!("C-{bare}"),
            Mode::Alt => format!("M-{bare}"),
        },
    }
}

/// How to press a tmux key: on a Mac `C-a` is `⌃a`, `?` is `⇧/`, `M-Up` is `⌥↑` and `PPage`
/// is `fn↑`; on Windows they are `Ctrl+a`, `Shift+/`, `Alt+↑` and `PgUp`.
pub fn keystroke(key: &str, caps: &[Cap], platform: Platform) -> String {
    let (mut control, mut alt, mut shift) = (false, false, false);
    let mut rest = key;
    loop {
        if let Some(after) = rest.strip_prefix("C-") {
            control = true;
            rest = after;
        } else if let Some(after) = rest.strip_prefix("M-") {
            alt = true;
            rest = after;
        } else if let Some(after) = rest.strip_prefix("S-") {
            shift = true;
            rest = after;
        } else {
            break;
        }
    }
    let mac = platform == Platform::Mac;
    let shifted_cap = caps
        .iter()
        .find(|cap| cap.shifted == Some(rest) && cap.base.is_some_and(|base| base.chars().count() == 1));
    let label = if rest == "BTab" {
        shift = true;
        if mac { "⇥" } else { "Tab" }.to_owned()
    } else if let Some(cap) = shifted_cap {
        shift = true;
        cap.base.expect("found by its base").to_uppercase()
    } else {
        let (mac_label, windows_label) = match rest {
            "Up" => ("↑", "↑"),
            "Down" => ("↓", "↓"),
            "Left" => ("←", "←"),
            "Right" => ("→", "→"),
            "PPage" => ("fn↑", "PgUp"),
            "NPage" => ("fn↓", "PgDn"),
            "Home" => ("fn←", "Home"),
            "End" => ("fn→", "End"),
            "DC" => ("fn⌫", "Del"),
            "IC" => ("Insert", "Ins"),
            "BSpace" => ("⌫", "Backspace"),
            "Enter" => ("⏎", "Enter"),
            "Tab" => ("⇥", "Tab"),
            other => (other, other),
        };
        if mac { mac_label } else { windows_label }.to_owned()
    };
    let modifiers = [(control, "⌃", "Ctrl"), (alt, "⌥", "Alt"), (shift, "⇧", "Shift")];
    let held = modifiers.iter().filter(|(on, _, _)| *on);
    if mac {
        let glyphs: String = held.map(|(_, glyph, _)| *glyph).collect();
        format!("{glyphs}{label}")
    } else {
        let mut words: Vec<String> = held.map(|(_, _, word)| (*word).to_owned()).collect();
        words.push(label);
        words.join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystrokes_read_as_the_keys_to_press() {
        let caps = keyboard(Platform::Mac);
        assert_eq!(keystroke("C-a", &caps, Platform::Mac), "⌃a");
        assert_eq!(keystroke("?", &caps, Platform::Mac), "⇧/");
        assert_eq!(keystroke("M-Up", &caps, Platform::Mac), "⌥↑");
        assert_eq!(keystroke("PPage", &caps, Platform::Mac), "fn↑");
        assert_eq!(keystroke("X", &caps, Platform::Windows), "Shift+X");
        assert_eq!(keystroke("C-a", &caps, Platform::Windows), "Ctrl+a");
    }

    #[test]
    fn a_key_is_read_in_the_current_mode() {
        let caps = keyboard(Platform::Mac);
        assert_eq!(in_mode("h", Mode::Control, &caps), "C-h");
        assert_eq!(in_mode("X", Mode::Base, &caps), "x");
        assert_eq!(in_mode("x", Mode::Shift, &caps), "X");
        assert_eq!(in_mode("Up", Mode::Alt, &caps), "M-Up");
    }

    #[test]
    fn every_row_spans_the_main_block() {
        for platform in [Platform::Mac, Platform::Windows] {
            let caps = keyboard(platform);
            for row in 0..5 {
                let width: u16 = caps.iter().filter(|cap| cap.row == row && cap.start < ARROWS_START).map(|cap| cap.width).sum();
                assert_eq!(width, 60, "{platform:?} row {row}");
            }
        }
    }
}
