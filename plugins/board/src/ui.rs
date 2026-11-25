//! Drawing the board: a header, the three columns, the detail drawer of the selected card and a
//! footer of keys (or the query being typed, or the notice); the label picker over it all.
//! Labels wear GitHub's own colours; everything else goes through the tx palette.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use tx_client::palette::indexed;
use tx_client::theme::{ColorDepth, Role, active};
use tx_core::LlmEngineState;

use super::app::{App, Picker};
use super::github::{Failure, Issue, age, rgb_of};
use super::model::{Card, CardAgent, ColumnKind};

/// Each card takes two rows: the number and title, then labels and agents.
pub const CARD_ROWS: u16 = 2;
const KEYS: &str =
    "j/k h/l move  / filter  L labels  s start agent  Enter show agent  ? all keys  q quit";

fn accent() -> Style {
    active().style(Role::Accent)
}

fn dim() -> Style {
    active().style(Role::Muted)
}

fn text() -> Style {
    active().style(Role::Text)
}

fn info() -> Style {
    active().style(Role::Accent)
}

/// A label in its GitHub colour; the theme's signal colour when it has none.
pub fn label_style(color: &str) -> Style {
    let theme = active();
    let Some((red, green, blue)) = rgb_of(color) else {
        return theme.style(Role::Signal);
    };
    let rgb = Color::Rgb(red, green, blue);
    match theme.depth {
        ColorDepth::Truecolor => Style::new().fg(rgb),
        ColorDepth::Ansi256 => Style::new().fg(indexed(rgb)),
        ColorDepth::Ansi16 => theme.style(Role::Text),
        ColorDepth::None => Style::new(),
    }
}

pub fn state_style(state: LlmEngineState, waiting: bool) -> Style {
    let theme = active();
    if waiting {
        return theme.style(Role::Signal);
    }
    match state {
        LlmEngineState::Working => theme.style(Role::Working),
        LlmEngineState::Starting | LlmEngineState::Idle | LlmEngineState::Ended => {
            theme.style(Role::Muted)
        }
    }
}

fn state_mark(state: LlmEngineState, waiting: bool) -> &'static str {
    let glyphs = active().glyphs;
    if waiting {
        return glyphs.needs_you;
    }
    match state {
        LlmEngineState::Working => glyphs.working,
        LlmEngineState::Starting => glyphs.starting,
        LlmEngineState::Idle => glyphs.idle,
        LlmEngineState::Ended => glyphs.ended,
    }
}

/// `text` cut to `width` cells, with `…` when it was longer.
pub fn truncate(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut cut: String = text.chars().take(width - 1).collect();
    cut.push('…');
    cut
}

pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();
    if area.height < 6 || area.width < 20 {
        return;
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(area);
    let drawer_rows = (body.height * 45 / 100)
        .max(5)
        .min(body.height.saturating_sub(4));
    let [columns, drawer] =
        Layout::vertical([Constraint::Min(4), Constraint::Length(drawer_rows)]).areas(body);
    let visible = usize::from(columns.height.saturating_sub(2) / CARD_ROWS);
    app.keep_cursor_visible(visible);

    frame.render_widget(header_line(app, header.width), header);
    if let Some(problem) = &app.problem {
        draw_problem(problem, columns, frame);
    } else {
        let thirds = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(columns);
        for (index, kind) in ColumnKind::ALL.into_iter().enumerate() {
            draw_column(app, kind, thirds[index], frame);
        }
    }
    draw_drawer(app, drawer, frame);
    frame.render_widget(footer_line(app, footer.width), footer);
    if let Some(picker) = app.picker.as_mut() {
        draw_picker(picker, body, frame);
    }
}

fn header_line(app: &App, width: u16) -> Line<'static> {
    let fetched = match (app.refreshing, app.fetched_at) {
        (true, _) => "refreshing…".to_owned(),
        (false, Some(at)) => format!("refreshed {}", ago(app.now - at)),
        (false, None) => "not fetched".to_owned(),
    };
    let mut spans = vec![
        Span::styled(
            format!(" {} ", tx_client::theme::MARK),
            accent().add_modifier(Modifier::BOLD),
        ),
        Span::styled("board  ", text().add_modifier(Modifier::BOLD)),
        Span::styled(app.board.repository.slug(), text()),
        Span::styled(format!("   {fetched}"), dim()),
    ];
    if !app.query.is_empty() {
        let room = usize::from(width).saturating_sub(
            spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>()
                + 3,
        );
        let hidden = format!("  ({} hidden, Esc clears)", app.board.hidden);
        let query = truncate(
            &app.query.text,
            room.saturating_sub(hidden.len() + "filter: ".len()),
        );
        spans.push(Span::styled("   filter: ", dim()));
        spans.push(Span::styled(query, info()));
        spans.push(Span::styled(hidden, dim()));
    }
    Line::from(spans)
}

fn ago(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    if seconds < 60 {
        format!("{seconds}s ago")
    } else {
        format!("{}m ago", seconds / 60)
    }
}

fn draw_column(app: &App, kind: ColumnKind, area: Rect, frame: &mut Frame) {
    let theme = active();
    let index = kind as usize;
    let cards = app.column_of(kind);
    let is_active = app.column == index;
    let border = if is_active {
        accent()
    } else {
        theme.style(Role::Rule)
    };
    let title = tx_client::theme::titled(&format!("{} {}", kind.title(), cards.len()));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border)
        .title(Span::styled(
            title,
            if is_active {
                accent().add_modifier(Modifier::BOLD)
            } else {
                dim()
            },
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 4 {
        return;
    }
    let visible = usize::from(inner.height / CARD_ROWS);
    let first = app.scroll[index];
    let selection = theme.selection_style();
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (row, card) in cards.iter().enumerate().skip(first).take(visible) {
        let selected = is_active && row == app.cursor[index];
        let mut head = card_head(card, usize::from(inner.width));
        let mut tail = card_tail(card, usize::from(inner.width));
        if selected {
            for line in [&mut head, &mut tail] {
                line.style = selection;
            }
        }
        lines.push(head);
        lines.push(tail);
    }
    if cards.len() > first + visible {
        let more = cards.len() - first - visible;
        if let Some(last) = lines.last_mut() {
            *last = Line::from(Span::styled(format!("  … {more} more"), dim()));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn card_head(card: &Card, width: usize) -> Line<'static> {
    let number = format!("#{} ", card.issue.number);
    let title = truncate(&card.issue.title, width.saturating_sub(number.len() + 1));
    Line::from(vec![
        Span::styled(" ", text()),
        Span::styled(number, text().add_modifier(Modifier::BOLD)),
        Span::styled(title, text()),
    ])
}

fn card_tail(card: &Card, width: usize) -> Line<'static> {
    let mut spans = vec![Span::styled("   ", text())];
    let mut used = 3;
    for label in &card.issue.labels {
        let word = format!("{} ", label.name);
        if used + word.chars().count() > width {
            break;
        }
        used += word.chars().count();
        spans.push(Span::styled(word, label_style(&label.color)));
    }
    if let Some(agent) = card.agents.first() {
        let word = format!("{} {}", state_mark(agent.state, agent.waiting), agent.name);
        let word = truncate(&word, width.saturating_sub(used));
        spans.push(Span::styled(word, state_style(agent.state, agent.waiting)));
    } else if let Some(assignee) = card.issue.assignees.first() {
        let word = truncate(&format!("@{}", assignee.login), width.saturating_sub(used));
        spans.push(Span::styled(word, dim()));
    }
    Line::from(spans)
}

fn draw_drawer(app: &App, area: Rect, frame: &mut Frame) {
    let theme = active();
    let border = theme.style(Role::Rule);
    let (title, lines) = match (app.help, app.selected()) {
        (true, _) => ("keys".to_owned(), help_lines()),
        (false, Some(card)) => (
            format!("#{} {}", card.issue.number, card.issue.title),
            detail_lines(card, app.now),
        ),
        (false, None) => (
            "board".to_owned(),
            vec![Line::from(Span::styled(
                if app.query.is_empty() {
                    "no issue in this column (h/l to move, n files one)"
                } else {
                    "no issue in this column matches the filter (Esc clears it)"
                },
                dim(),
            ))],
        ),
    };
    let clipped = truncate(&title, usize::from(area.width.saturating_sub(8)));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border)
        .title(Span::styled(
            tx_client::theme::titled(&clipped),
            text().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0)),
        inner,
    );
}

fn detail_lines(card: &Card, now: f64) -> Vec<Line<'static>> {
    let issue: &Issue = &card.issue;
    let mut first = vec![Span::styled(" ", text())];
    for label in &issue.labels {
        first.push(Span::styled(
            format!("{} ", label.name),
            label_style(&label.color),
        ));
    }
    if let Some(milestone) = &issue.milestone {
        first.push(Span::styled(format!("◆ {} ", milestone.title), accent()));
    }
    let who = if issue.assignees.is_empty() {
        String::new()
    } else {
        format!(
            "  assigned {}",
            issue
                .assignees
                .iter()
                .map(|user| format!("@{}", user.login))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    first.push(Span::styled(
        format!(
            "  {} by @{}, opened {} ago, updated {} ago{who}",
            issue.state.to_lowercase(),
            issue.author.login,
            age(&issue.created_at, now),
            age(&issue.updated_at, now)
        ),
        dim(),
    ));
    let mut lines = vec![Line::from(first)];
    let mut agents = vec![Span::styled(" agents: ", dim())];
    if card.agents.is_empty() {
        agents.push(Span::styled("none (s starts one)", dim()));
    }
    for (index, agent) in card.agents.iter().enumerate() {
        if index > 0 {
            agents.push(Span::styled(", ", dim()));
        }
        agents.push(agent_span(agent));
    }
    lines.push(Line::from(agents));
    lines.push(Line::default());
    for line in issue.body.lines() {
        lines.push(Line::from(Span::styled(format!(" {line}"), text())));
    }
    lines
}

fn agent_span(agent: &CardAgent) -> Span<'static> {
    let label = if agent.waiting {
        "needs you"
    } else {
        agent.state.as_str()
    };
    Span::styled(
        format!(
            "{} {} {label}",
            state_mark(agent.state, agent.waiting),
            agent.name
        ),
        state_style(agent.state, agent.waiting),
    )
}

fn help_lines() -> Vec<Line<'static>> {
    [
        ("j/k  h/l", "move between cards and columns; g/G first and last card"),
        ("J/K", "scroll the issue body"),
        (
            "/",
            "filter in GitHub's search language: label:bug -label:wip assignee:me no:assignee author:ada milestone:0.2 is:open \"free text\"; Enter keeps it, Esc clears it",
        ),
        ("L", "the issue's labels: space ticks, Enter applies, Esc cancels"),
        ("s", "start a developer agent on the issue (tagged issue-<n>), in this checkout"),
        (
            "Enter  a  p",
            "show the agent working on it: focus its pane, attach it into a pane, popup",
        ),
        ("o", "open the issue in the browser"),
        ("c  n", "comment on the issue; file a new issue (gh opens your editor)"),
        ("x", "close the issue, or reopen a closed one"),
        ("m", "assign yourself, or unassign"),
        ("r", "fetch again now (every 60 s otherwise; agents every 3 s)"),
        ("?  q", "this list; quit (the popup's resident stays warm)"),
    ]
    .into_iter()
    .map(|(keys, what)| {
        Line::from(vec![
            Span::styled(format!(" {keys:<12}"), accent()),
            Span::styled(what.to_owned(), text()),
        ])
    })
    .collect()
}

fn footer_line(app: &App, width: u16) -> Line<'static> {
    let width = usize::from(width);
    if app.searching {
        return Line::from(vec![
            Span::styled(" / ", accent().add_modifier(Modifier::BOLD)),
            Span::styled(app.query.text.clone(), text()),
            Span::styled("█", accent()),
            Span::styled(
                "   label:  assignee:  no:assignee  author:  milestone:  is:open  -not  \"text\"   Enter keeps  Esc clears",
                dim(),
            ),
        ]);
    }
    match &app.notice {
        Some(notice) => Line::from(Span::styled(
            truncate(&format!(" {notice}"), width),
            active().style(Role::Signal),
        )),
        None => Line::from(Span::styled(truncate(&format!(" {KEYS}"), width), dim())),
    }
}

/// What stands where the columns would be while `gh` cannot be used: what is wrong and the
/// one command that fixes it.
fn draw_problem(problem: &Failure, area: Rect, frame: &mut Frame) {
    let theme = active();
    let (title, lines): (&str, Vec<(String, Style)>) = match problem {
        Failure::NotInstalled => (
            "GitHub CLI is not installed",
            vec![
                (
                    "The board reads and edits issues through gh, GitHub's command line, which is not on PATH."
                        .to_owned(),
                    text(),
                ),
                (String::new(), text()),
                ("  brew install gh        https://cli.github.com".to_owned(), accent()),
                ("  gh auth login          then sign in once".to_owned(), accent()),
                (String::new(), text()),
                ("r retries once it is there.".to_owned(), dim()),
            ],
        ),
        Failure::NotLoggedIn(said) => (
            "gh is not signed in",
            vec![
                (
                    "The board uses your GitHub login through gh; gh has none yet.".to_owned(),
                    text(),
                ),
                (String::new(), text()),
                ("  gh auth login          in any shell".to_owned(), accent()),
                (String::new(), text()),
                (format!("gh said: {said}"), dim()),
                ("r retries once you are signed in.".to_owned(), dim()),
            ],
        ),
        Failure::Failed(reason) => ("GitHub did not answer", vec![(reason.clone(), text())]),
    };
    let signal = theme.style(Role::Signal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(signal)
        .title(Span::styled(
            tx_client::theme::titled(title),
            signal.add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let top = inner.height.saturating_sub(lines.len() as u16) / 2;
    let mut body: Vec<Line<'static>> = (0..top).map(|_| Line::default()).collect();
    body.extend(
        lines
            .into_iter()
            .map(|(line, style)| Line::from(Span::styled(format!("   {line}"), style))),
    );
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), inner);
}

/// The label picker, centred over `area`: one row per label of the repository.
fn draw_picker(picker: &mut Picker, area: Rect, frame: &mut Frame) {
    let longest = picker
        .choices
        .iter()
        .map(|choice| choice.label.name.chars().count() + choice.label.description.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let width = (longest + 12).clamp(40, area.width.saturating_sub(4).max(20));
    let rows = picker.choices.len() as u16 + 3;
    let height = rows.clamp(5, area.height.saturating_sub(2).max(5));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    let theme = active();
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(accent())
        .title(Span::styled(
            tx_client::theme::titled(&format!("labels of #{}", picker.number)),
            accent().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let visible = usize::from(inner.height.saturating_sub(1));
    picker.keep_cursor_visible(visible);
    let name_width = picker
        .choices
        .iter()
        .map(|choice| choice.label.name.chars().count())
        .max()
        .unwrap_or(0);
    let selection = theme.selection_style();
    let mut lines: Vec<Line<'static>> = picker
        .choices
        .iter()
        .enumerate()
        .skip(picker.scroll)
        .take(visible)
        .map(|(index, choice)| {
            let tick = if choice.checked { "[x] " } else { "[ ] " };
            let name = format!("{:<name_width$}", choice.label.name);
            let description = truncate(
                &format!("  {}", choice.label.description),
                usize::from(inner.width).saturating_sub(name_width + 6),
            );
            let mut line = Line::from(vec![
                Span::styled(" ", text()),
                Span::styled(tick, if choice.checked { text() } else { dim() }),
                Span::styled("● ", label_style(&choice.label.color)),
                Span::styled(name, text()),
                Span::styled(description, dim()),
            ]);
            if index == picker.cursor {
                line.style = selection;
            }
            line
        })
        .collect();
    while lines.len() < visible {
        lines.push(Line::default());
    }
    lines.push(Line::from(Span::styled(
        " space ticks  Enter applies  Esc cancels",
        dim(),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The frame as plain text, trailing blanks trimmed: what `--snapshot` prints.
pub fn buffer_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut lines = Vec::with_capacity(usize::from(area.height));
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_the_width() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("abcd", 0), "");
    }

    #[test]
    fn a_missing_or_signed_out_gh_is_explained_where_the_columns_were() {
        use crate::github::Repository;
        use crate::model::Board;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let frame = |problem: Failure| {
            let mut app = App::new(Board::empty(Repository::parse("o/n").unwrap()), 0.0);
            app.problem = Some(problem);
            let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
            terminal.draw(|frame| render(&mut app, frame)).unwrap();
            buffer_text(terminal.backend().buffer())
        };
        let missing = frame(Failure::NotInstalled);
        assert!(missing.contains("GitHub CLI is not installed"), "{missing}");
        assert!(missing.contains("brew install gh"), "{missing}");
        assert!(!missing.contains("Backlog"), "{missing}");
        let signed_out = frame(Failure::NotLoggedIn("run gh auth login".to_owned()));
        assert!(signed_out.contains("gh is not signed in"), "{signed_out}");
        assert!(
            signed_out.contains("gh said: run gh auth login"),
            "{signed_out}"
        );
    }

    #[test]
    fn labels_wear_their_github_colour() {
        let theme = active();
        let expected = match theme.depth {
            ColorDepth::Truecolor => Some(Color::Rgb(0xd7, 0x3a, 0x4a)),
            ColorDepth::Ansi256 => Some(indexed(Color::Rgb(0xd7, 0x3a, 0x4a))),
            ColorDepth::Ansi16 => theme.color(Role::Text),
            ColorDepth::None => None,
        };
        assert_eq!(label_style("d73a4a").fg, expected);
        assert_eq!(label_style("").fg, theme.style(Role::Signal).fg);
    }
}
