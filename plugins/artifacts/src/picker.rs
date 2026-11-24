//! The prefix+a picker, ported from main's `artifact_picker.rs`: a list in a popup over the pane,
//! drawn like the session picker's rows.
//!
//! Pure list logic ([`Picker`]), keys in and the chosen action out, around tx-client's editor
//! terminal layer. A row's key (`1`…`9`, then choose-tree's `M-a`…`M-z`, `M-A`…`M-Z`) moves the
//! cursor to that row, as j/k and the arrows move it by one. Then, as on a timeline lane, the
//! picks are tx-client's ways of showing a session ([`Show`]): a (or Enter) attaches the cursor's
//! row into a pane the operator picks next (attach-to), p opens it in a popup. Esc, q or Ctrl-C
//! close. A list taller than the popup scrolls with the cursor.

use std::io;

use tx_client::editor::{Decoder, Key, Terminal, read_key, write_all};
use tx_client::render::{ACCENT_ANSI, BOLD, DIM_FG_ANSI, RESET, SELECTION_BG};
use tx_client::show::Show;

pub const HINT: &str = "key go to row   a attach to a pane   p popup   Esc close";
/// Screen lines that are not list: the header, a blank line and the hint.
pub const CHROME_ROWS: usize = 3;

/// The key that picks row `index` (0-based), as choose-tree labels them: `1`…`9`, then `M-a`…
/// `M-z`, then `M-A`…`M-Z`; `None` past those.
pub fn row_key(index: usize) -> Option<Key> {
    match index {
        0..=8 => Some(Key::Char((b'1' + index as u8) as char)),
        9..=34 => Some(Key::Alt((b'a' + (index - 9) as u8) as char)),
        35..=60 => Some(Key::Alt((b'A' + (index - 35) as u8) as char)),
        _ => None,
    }
}

/// The row a key picks: the inverse of [`row_key`].
fn row_index(key: Key) -> Option<usize> {
    match key {
        Key::Char(digit @ '1'..='9') => Some(digit as usize - '1' as usize),
        Key::Alt(letter @ 'a'..='z') => Some(9 + letter as usize - 'a' as usize),
        Key::Alt(letter @ 'A'..='Z') => Some(35 + letter as usize - 'A' as usize),
        _ => None,
    }
}

/// `row_key`'s label for the row's left edge: `(1)`, `(M-a)`.
pub fn row_label(index: usize) -> String {
    match row_key(index) {
        Some(Key::Char(digit)) => format!("({digit})"),
        Some(Key::Alt(letter)) => format!("(M-{letter})"),
        _ => String::new(),
    }
}

/// A pick: how to show which row.
pub type Pick = (Show, usize);

/// One line of the list: a section title (not selectable) or a fully styled row (SGR, no
/// background). Rows are numbered in order across sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Section(String),
    Row(String),
}

impl Item {
    pub fn text(&self) -> &str {
        match self {
            Self::Section(text) | Self::Row(text) => text,
        }
    }
}

/// The list state: `items` in display order, `header` the dim column line above them, the
/// cursor counted over the rows only.
pub struct Picker<'a> {
    header: &'a str,
    items: &'a [Item],
    rows: usize,
    cursor: usize,
}

impl<'a> Picker<'a> {
    pub fn new(header: &'a str, items: &'a [Item]) -> Self {
        Self {
            header,
            items,
            rows: items
                .iter()
                .filter(|item| matches!(item, Item::Row(_)))
                .count(),
            cursor: 0,
        }
    }

    /// `Some(Some(pick))` chooses, `Some(None)` cancels, `None` keeps going.
    pub fn handle(&mut self, key: Key) -> Option<Option<Pick>> {
        match key {
            Key::Up | Key::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            Key::Down | Key::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.rows.saturating_sub(1));
            }
            Key::Char('a') | Key::Enter | Key::Char('\r') | Key::Char('\n') => {
                return Some(self.row().map(|index| (Show::AttachTo, index)));
            }
            Key::Char('p') => return Some(self.row().map(|index| (Show::Popup, index))),
            Key::Char('1'..='9') | Key::Alt(_) => {
                if let Some(index) = row_index(key).filter(|index| *index < self.rows) {
                    self.cursor = index;
                }
            }
            Key::Char('\x1b' | 'q' | '\x03') => return Some(None),
            _ => {}
        }
        None
    }

    fn row(&self) -> Option<usize> {
        (self.rows > 0).then_some(self.cursor)
    }

    /// The position in `items` of the cursor's row.
    fn cursor_item(&self) -> usize {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, Item::Row(_)))
            .nth(self.cursor)
            .map_or(0, |(position, _)| position)
    }

    /// The screen: the header, the items that fit between it and the hint (scrolled just far
    /// enough to show the cursor's row, on the selection background), the hint last.
    pub fn render(&self, height: usize, width: usize) -> String {
        let mut out = String::from("\x1b[0m\x1b[H\x1b[2J");
        let mut put = |screen_row: usize, text: &str| {
            if screen_row < height {
                out += &format!("\x1b[{};1H{text}{RESET}", screen_row + 1);
            }
        };
        put(0, &format!("{DIM_FG_ANSI}{}", self.header));
        let visible = height.saturating_sub(CHROME_ROWS).max(1);
        let first = (self.cursor_item() + 1).saturating_sub(visible);
        let mut row = self.items[..first]
            .iter()
            .filter(|item| matches!(item, Item::Row(_)))
            .count();
        for (offset, item) in self.items[first..].iter().take(visible).enumerate() {
            let line = match item {
                Item::Section(title) => format!("{ACCENT_ANSI}{BOLD}{title}"),
                Item::Row(line) if row == self.cursor => {
                    let fill = width.saturating_sub(visible_width(line));
                    format!("\x1b[48;5;{SELECTION_BG}m{line}{}", " ".repeat(fill))
                }
                Item::Row(line) => line.clone(),
            };
            if matches!(item, Item::Row(_)) {
                row += 1;
            }
            put(1 + offset, &line);
        }
        if height >= CHROME_ROWS {
            put(height - 1, &format!("{DIM_FG_ANSI}{HINT}"));
        }
        out
    }
}

/// Columns a styled line occupies: SGR sequences (`ESC [ … m`) take none.
pub fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\x1b' && characters.peek() == Some(&'[') {
            for inner in characters.by_ref() {
                if inner == 'm' {
                    break;
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

/// Run the picker on the controlling terminal; `None` on Esc / q / Ctrl-C.
pub fn pick(header: &str, items: &[Item]) -> io::Result<Option<Pick>> {
    let _terminal = Terminal::open()?;
    let mut picker = Picker::new(header, items);
    let mut decoder = Decoder::default();
    loop {
        let (height, width) = Terminal::size();
        write_all(picker.render(height, width).as_bytes())?;
        let key = read_key(&mut decoder, (height, width))?;
        if let Some(chosen) = picker.handle(key) {
            return Ok(chosen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_to_a_row_a_attaches_to_p_pops_up() {
        let items = vec![
            Item::Section("X".to_owned()),
            Item::Row("a".to_owned()),
            Item::Section("Y".to_owned()),
            Item::Row("b".to_owned()),
            Item::Row("c".to_owned()),
        ];
        let mut picker = Picker::new("H", &items);
        assert_eq!(picker.handle(Key::Down), None);
        assert_eq!(picker.handle(Key::Enter), Some(Some((Show::AttachTo, 1))));
        assert_eq!(picker.handle(Key::Char('3')), None);
        assert_eq!(
            picker.handle(Key::Char('a')),
            Some(Some((Show::AttachTo, 2)))
        );
        assert_eq!(picker.handle(Key::Char('1')), None);
        assert_eq!(picker.handle(Key::Char('p')), Some(Some((Show::Popup, 0))));
        assert_eq!(picker.handle(Key::Char('0')), None);
        assert_eq!(picker.handle(Key::Char('7')), None);
        assert_eq!(picker.handle(Key::Char('f')), None);
        assert_eq!(picker.handle(Key::Char('l')), None);
        assert_eq!(picker.handle(Key::Char('q')), Some(None));
    }

    #[test]
    fn rows_past_nine_take_meta_letters() {
        assert_eq!(row_key(0), Some(Key::Char('1')));
        assert_eq!(row_key(8), Some(Key::Char('9')));
        assert_eq!(row_key(9), Some(Key::Alt('a')));
        assert_eq!(row_key(34), Some(Key::Alt('z')));
        assert_eq!(row_key(35), Some(Key::Alt('A')));
        assert_eq!(row_key(60), Some(Key::Alt('Z')));
        assert_eq!(row_key(61), None);
        for index in 0..61 {
            assert_eq!(row_index(row_key(index).unwrap()), Some(index));
        }
        assert_eq!(row_label(9), "(M-a)");
        let items: Vec<Item> = (0..12).map(|i| Item::Row(i.to_string())).collect();
        let mut picker = Picker::new("H", &items);
        assert_eq!(picker.handle(Key::Alt('c')), None);
        assert_eq!(picker.handle(Key::Alt('d')), None);
        assert_eq!(picker.handle(Key::Char('p')), Some(Some((Show::Popup, 11))));
    }

    #[test]
    fn a_list_taller_than_the_screen_scrolls_with_the_cursor() {
        let mut items = vec![Item::Section("S".to_owned())];
        items.extend((0..10).map(|i| Item::Row(format!("row{i}"))));
        let mut picker = Picker::new("H", &items);
        // 7 lines: header, 4 of list, a blank, the hint.
        let screen = picker.render(7, 20);
        assert!(screen.contains("S") && screen.contains("row2") && !screen.contains("row3"));
        assert!(screen.contains(HINT));
        for _ in 0..5 {
            picker.handle(Key::Down);
        }
        let screen = picker.render(7, 20);
        assert!(screen.contains("row2") && screen.contains("row5"));
        assert!(!screen.contains("row1") && !screen.contains("row6"));
        assert!(screen.contains(HINT));
    }

    #[test]
    fn empty_list_chooses_nothing() {
        let items = vec![Item::Section("X".to_owned())];
        let mut picker = Picker::new("H", &items);
        assert_eq!(picker.handle(Key::Enter), Some(None));
    }

    #[test]
    fn visible_width_skips_sgr() {
        assert_eq!(visible_width("\x1b[38;5;1m[x]\x1b[39m y"), 5);
    }
}
