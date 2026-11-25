//! The popup: two rows of switches (keyboard, mode), the board, a legend and the caption of
//! the pressed key. Tab switches the mode, Shift-Tab the keyboard; any other key, typed or
//! clicked, is read in the current mode and shown. Esc closes.
//!
//! Each key is drawn as a filled block with half-block top and bottom edges, so it reads as two
//! rows tall with a gap around it; the board scales to the popup in whole cells per key-unit.

use std::cell::RefCell;
use std::io;

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use tx_client::palette::{
    ACCENT, BACKGROUND, COMMENT, DIM_FG, FG, ON_ACCENT, SUBTLE_FG, SURFACE, SURFACE_RAISED,
    SURFACE_SUNKEN, WARN, palette,
};

use crate::bindings::{Bindings, Owner};
use crate::keyboard::{
    self, Cap, Mode, Platform, WIDTH_IN_QUARTERS, in_mode, keystroke, tmux_key_name,
};

/// What a click lands on.
#[derive(Clone)]
enum Target {
    Key(String),
    Platform(Platform),
}

struct App {
    bindings: Bindings,
    platform: Platform,
    caps: Vec<Cap>,
    mode: Mode,
    selected: Option<String>,
    /// Where the last frame drew something clickable.
    targets: RefCell<Vec<(Rect, Target)>>,
}

pub fn run(bindings: Bindings) -> io::Result<()> {
    let platform = Platform::native();
    let mut app = App {
        bindings,
        platform,
        caps: keyboard::keyboard(platform),
        mode: Mode::Base,
        selected: None,
        targets: RefCell::new(Vec::new()),
    };
    ratatui::run(|terminal| {
        ratatui::crossterm::execute!(io::stdout(), EnableMouseCapture)?;
        let result = event_loop(terminal, &mut app);
        ratatui::crossterm::execute!(io::stdout(), DisableMouseCapture)?;
        result
    })
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        app.targets.borrow_mut().clear();
        terminal.draw(|frame| draw(frame, app))?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Tab if key.modifiers.is_empty() => app.mode = app.mode.next(),
                KeyCode::BackTab => app.switch_platform(app.platform.next()),
                _ => {
                    if let Some(name) = tmux_key_name(key) {
                        app.selected = Some(in_mode(&name, app.mode, &app.caps));
                    }
                }
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = Position {
                    x: mouse.column,
                    y: mouse.row,
                };
                let target = app
                    .targets
                    .borrow()
                    .iter()
                    .find(|(area, _)| area.contains(position))
                    .map(|(_, target)| target.clone());
                match target {
                    Some(Target::Key(name)) => app.selected = Some(name),
                    Some(Target::Platform(platform)) => app.switch_platform(platform),
                    None => {}
                }
            }
            _ => {}
        }
    }
}

impl App {
    fn switch_platform(&mut self, platform: Platform) {
        self.platform = platform;
        self.caps = keyboard::keyboard(platform);
    }

    fn keystroke(&self, key: &str) -> String {
        keystroke(key, &self.caps, self.platform)
    }
}

/// Fill and symbol colour of a key that stands for `key`.
fn key_colors(app: &App, key: &str) -> (Color, Color) {
    if app.selected.as_deref() == Some(key) {
        return (WARN, BACKGROUND);
    }
    match app.bindings.keys.get(key).map(|binding| &binding.owner) {
        Some(Owner::Tx(_)) => (ACCENT, ON_ACCENT),
        Some(Owner::Tmux) => (SURFACE_RAISED, FG),
        None => (SURFACE, SUBTLE_FG),
    }
}

fn style(foreground: Color) -> Style {
    Style::new().fg(palette(foreground))
}

fn pill(foreground: Color, background: Color) -> Style {
    Style::new()
        .fg(palette(foreground))
        .bg(palette(background))
        .add_modifier(Modifier::BOLD)
}

const CAPTION_HEIGHT: u16 = 2;

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(Style::new().bg(palette(BACKGROUND))),
        area,
    );
    let units = f32::from(WIDTH_IN_QUARTERS) / 4.0;
    let columns_per_unit = (f32::from(area.width.saturating_sub(4)) / units)
        .floor()
        .clamp(3.0, 7.0);
    let column = |quarters: u16| (f32::from(quarters) * columns_per_unit / 4.0).round() as u16;
    let board_width = column(WIDTH_IN_QUARTERS);
    let board_height = 5 * 3;
    let content_height = 4 + board_height + 1 + 2 + CAPTION_HEIGHT;
    let top = area.y + area.height.saturating_sub(content_height) / 2;
    let left = area.x + area.width.saturating_sub(board_width) / 2;
    let row = |y: u16, height: u16| {
        Rect {
            x: left,
            y,
            width: board_width,
            height,
        }
        .intersection(area)
    };

    draw_switches(frame, app, row(top, 1), row(top + 2, 1));
    let board_top = top + 4;
    for cap in &app.caps {
        let x = left + column(cap.start);
        let width = column(cap.start + cap.width) - column(cap.start) - 1;
        draw_cap(
            frame,
            app,
            cap,
            Rect {
                x,
                y: board_top + cap.row * 3,
                width,
                height: 3,
            },
        );
    }
    let legend_y = board_top + board_height + 1;
    frame.render_widget(legend(app), row(legend_y, 1));
    frame.render_widget(
        Paragraph::new(caption(app)),
        row(legend_y + 2, CAPTION_HEIGHT),
    );
}

/// The keyboard row (Shift-Tab) above the mode row (Tab), each with its key at the right.
fn draw_switches(frame: &mut Frame, app: &App, platform_row: Rect, mode_row: Rect) {
    let mut platforms = Vec::new();
    let mut x = platform_row.x;
    for platform in [Platform::Mac, Platform::Windows] {
        let label = format!(" {} ", platform.name());
        let width = label.chars().count() as u16;
        let label_style = if platform == app.platform {
            pill(FG, SURFACE_RAISED)
        } else {
            style(SUBTLE_FG)
        };
        app.targets.borrow_mut().push((
            Rect {
                x,
                y: platform_row.y,
                width,
                height: 1,
            },
            Target::Platform(platform),
        ));
        x += width + 2;
        platforms.push(Span::styled(label, label_style));
        platforms.push(Span::raw("  "));
    }
    frame.render_widget(Line::from(platforms), platform_row);

    let mut modes = Vec::new();
    for mode in Mode::ALL {
        let count = app
            .bindings
            .keys
            .keys()
            .filter(|key| Mode::of(key, &app.caps) == mode)
            .count();
        let (label_style, count_style) = if mode == app.mode {
            let active = pill(FG, SURFACE_RAISED).add_modifier(Modifier::UNDERLINED);
            (active, active)
        } else {
            (style(SUBTLE_FG), style(COMMENT))
        };
        modes.push(Span::styled(
            format!(" {} ", mode.name(app.platform)),
            label_style,
        ));
        modes.push(Span::styled(format!("{count} "), count_style));
        modes.push(Span::raw(" "));
    }
    frame.render_widget(Line::from(modes), mode_row);

    let mac = app.platform == Platform::Mac;
    for (row, hint) in [
        (
            platform_row,
            if mac {
                " ⇧ shift ⇥ tab "
            } else {
                " Shift+Tab "
            },
        ),
        (mode_row, if mac { " ⇥ tab " } else { " Tab " }),
    ] {
        let width = hint.chars().count() as u16;
        let area = Rect {
            x: row.right().saturating_sub(width),
            width,
            ..row
        };
        frame.render_widget(Span::styled(hint, pill(FG, SURFACE_RAISED)), area);
    }
}

fn draw_cap(frame: &mut Frame, app: &App, cap: &Cap, area: Rect) {
    let screen = frame.area();
    let key = cap.key(app.mode);
    let (fill, symbol_color) = match &key {
        Some((name, _)) => key_colors(app, name),
        None => (SURFACE_SUNKEN, COMMENT),
    };
    if let Some((name, _)) = &key {
        app.targets
            .borrow_mut()
            .push((area, Target::Key(name.clone())));
    }
    if area.bottom() > screen.bottom() {
        return;
    }
    let buffer = frame.buffer_mut();
    for x in area.left()..area.right().min(screen.right()) {
        buffer[(x, area.y)].set_symbol("▄").set_fg(palette(fill));
        buffer[(x, area.y + 1)]
            .set_symbol(" ")
            .set_bg(palette(fill));
        buffer[(x, area.y + 2)]
            .set_symbol("▀")
            .set_fg(palette(fill));
    }
    let symbol = key.as_ref().map_or(cap.symbol, |(_, symbol)| symbol);
    let symbol_x = area.x + area.width.saturating_sub(symbol.chars().count() as u16) / 2;
    let weight = if matches!(fill, SURFACE | SURFACE_SUNKEN) {
        Modifier::empty()
    } else {
        Modifier::BOLD
    };
    let symbol_style = Style::new()
        .fg(palette(symbol_color))
        .bg(palette(fill))
        .add_modifier(weight);
    buffer.set_string(symbol_x, area.y + 1, symbol, symbol_style);
    // In the base mode a dot at the right says the shifted key is bound too.
    if app.mode != Mode::Base || area.width < 3 {
        return;
    }
    let Some((shifted, _)) = cap.key(Mode::Shift) else {
        return;
    };
    let Some(binding) = app.bindings.keys.get(&shifted) else {
        return;
    };
    let dot = match (&binding.owner, fill) {
        (_, ACCENT | WARN) => BACKGROUND,
        (Owner::Tx(_), _) => ACCENT,
        (Owner::Tmux, _) => DIM_FG,
    };
    buffer.set_string(
        area.right() - 1,
        area.y + 1,
        "•",
        Style::new().fg(palette(dot)).bg(palette(fill)),
    );
}

fn legend(app: &App) -> Line<'static> {
    let mut spans = vec![
        Span::styled("■ ", style(ACCENT)),
        Span::styled("tx      ", style(DIM_FG)),
        Span::styled("■ ", style(SURFACE_RAISED)),
        Span::styled("tmux      ", style(DIM_FG)),
        Span::styled("■ ", style(SURFACE)),
        Span::styled("free", style(DIM_FG)),
    ];
    if app.mode == Mode::Base {
        let shift = if app.platform == Platform::Mac {
            "⇧"
        } else {
            "Shift+"
        };
        spans.push(Span::styled("      •", style(ACCENT)));
        spans.push(Span::styled("•", style(DIM_FG)));
        spans.push(Span::styled(
            format!(" {shift}key is also bound"),
            style(DIM_FG),
        ));
    }
    Line::from(spans)
}

fn caption(app: &App) -> Vec<Line<'static>> {
    let Some(key) = &app.selected else {
        return vec![Line::styled(
            "press a key to see what it does  ·  esc closes",
            style(COMMENT),
        )];
    };
    let keycap = pill(FG, SURFACE_RAISED);
    let mut spans = vec![
        Span::styled(format!(" {} ", app.keystroke(&app.bindings.prefix)), keycap),
        Span::raw(" "),
        Span::styled(format!(" {} ", app.keystroke(key)), keycap),
        Span::raw("   "),
    ];
    match app.bindings.keys.get(key) {
        None => spans.push(Span::styled("free — nothing bound here", style(SUBTLE_FG))),
        Some(binding) => {
            spans.push(Span::styled(binding.description(), style(FG)));
            let (owner, owner_color) = match &binding.owner {
                Owner::Tx(plugin) => (format!("tx · {plugin}"), ACCENT),
                Owner::Tmux => ("tmux".to_owned(), SUBTLE_FG),
            };
            spans.push(Span::styled(format!("   {owner}"), style(owner_color)));
        }
    }
    let mut lines = vec![Line::from(spans)];
    if key.contains("M-") && app.platform == Platform::Mac {
        lines.push(Line::styled(
            "⌥ works when the terminal sends Option as Meta, or press esc then the key",
            style(COMMENT),
        ));
    }
    lines
}
