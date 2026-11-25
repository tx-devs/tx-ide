//! Drawing, after the "tx-ide visual identity" artifact: a header, the column labels, the
//! canvas (braille Bézier edges under heavy boxes), a legend, the selected node's detail line
//! and the key hints.
//!
//! The canvas is drawn whole into an off-screen buffer as tall as the layout, then the rows the
//! scroll shows are copied into the frame, so a box cut by the view's edge stays a box.

use std::f64::consts::TAU;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context, Line as Segment, Points};
use ratatui::widgets::{Block, BorderType, Clear, LineGauge, Paragraph, Sparkline, Widget};
use tx_chat::transcript::clock;

use crate::app::{App, Mode, PULSE_TRAVEL, Purpose};
use crate::layout;
use crate::model::{Edge, Graph, Node, NodeState};
use tx_client::theme::{Role, active};

/// Above this share of its window, a context gauge turns signal amber.
const CONTEXT_WARN: f64 = 0.8;
/// How far a sibling arc bulges right of its column.
const ARC_BULGE: f64 = 7.0;
const CURVE_STEPS: u16 = 48;
/// The "parent ended" stub's length in cells.
const STUB_LENGTH: f64 = 12.0;
/// The sidebar shows from this terminal width up, when the graph keeps full boxes beside it.
const SIDEBAR_FROM: u16 = 130;
const SIDEBAR_WIDTH: u16 = 36;
const KEYS: [(&str, &str); 14] = [
    ("tab / ⇧tab", "next / previous agent, column by column"),
    (
        "h j k l",
        "the nearest agent left, below, above, right; past the edge, scroll",
    ),
    ("wheel", "scroll"),
    ("enter", "jump to the agent's pane"),
    ("s", "send the agent a line, as you"),
    ("f", "fork the agent (asks for a name; blank: <name>-fork)"),
    ("a", "the agent's artifacts"),
    ("x", "stop the agent, after y"),
    ("r", "respawn an ended agent"),
    ("g", "next group: all → each group → all"),
    ("?", "this page"),
    (
        "q / esc",
        "quit (in the popup: close it; the graph stays warm)",
    ),
    ("", ""),
    (
        "legend",
        "● working  ○ idle  × ended  ▖ starting  ▲ needs you  ◇ you",
    ),
];
/// Seconds of one breath of a working node's border.
const BREATH_PERIOD: f64 = 2.4;
/// Dots behind a pulse's head, each a step further back along the edge.
const PULSE_TRAIL: u16 = 10;
const PULSE_TRAIL_STEP: f64 = 0.018;
/// The edge length, in cells, the trail step was tuned for.
const CURVE_REFERENCE_LENGTH: f64 = 40.0;
/// The box rows incoming edges arrive on, in order: the first inner line, the second, the top
/// border, the bottom border.
const ARRIVAL_ROWS: [u16; 4] = [1, 2, 0, 3];
/// How far left a badge moves per round of arrivals beyond the box's rows.
const BADGE_SHIFT: f64 = 6.0;
/// Cells from a node's right side to its fan's bus.
const BUS_OFFSET: f64 = 2.0;
/// Cells on, then off, in a dashed (lineage-only) edge.
const DASH: f64 = 1.5;

fn role(role: Role) -> Style {
    active().style(role)
}

fn dim() -> Style {
    role(Role::Muted)
}

fn bold(target: Role) -> Style {
    active().bold(target)
}

fn color_of(target: Role) -> Color {
    active().color(target).unwrap_or(Color::Reset)
}

fn style_of_color(color: Option<Color>) -> Style {
    match color {
        Some(color) => Style::new().fg(color),
        None => Style::new(),
    }
}

/// `from` moved `amount` (0..1) of the way to `to`, mapped through the active theme's depth.
fn mix(from: Option<Color>, to: Option<Color>, amount: f64) -> Option<Color> {
    let (Some(Color::Rgb(red, green, blue)), Some(Color::Rgb(to_red, to_green, to_blue))) =
        (from, to)
    else {
        return from;
    };
    let blend = |start: u8, end: u8| {
        (f64::from(start) + (f64::from(end) - f64::from(start)) * amount.clamp(0.0, 1.0)).round()
            as u8
    };
    active().map_color(Color::Rgb(
        blend(red, to_red),
        blend(green, to_green),
        blend(blue, to_blue),
    ))
}

/// `text` cut to `width` characters, the last one `…` when cut.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(width.saturating_sub(1)).collect();
    kept.push('…');
    kept
}

/// `1 message`, `3 messages`.
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// `4s`, `12m`, `3h`, `2d`.
pub fn age(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    match seconds {
        _ if seconds < 60.0 => format!("{}s", seconds as u64),
        _ if seconds < 3600.0 => format!("{}m", (seconds / 60.0) as u64),
        _ if seconds < 86400.0 => format!("{}h", (seconds / 3600.0) as u64),
        _ => format!("{}d", (seconds / 86400.0) as u64),
    }
}

/// The window as typed: `1h`, `30m`, `90s`, `2d`.
pub fn window_label(seconds: f64) -> String {
    let whole = |unit: f64| (seconds / unit).fract() == 0.0;
    match seconds {
        _ if whole(86400.0) => format!("{}d", seconds / 86400.0),
        _ if whole(3600.0) => format!("{}h", seconds / 3600.0),
        _ if whole(60.0) => format!("{}m", seconds / 60.0),
        _ => format!("{}s", seconds.round()),
    }
}

/// `1.2K`, `75K`, `1.0M`.
fn tokens(count: u64) -> String {
    match count {
        _ if count >= 1_000_000 => format!("{:.1}M", count as f64 / 1e6),
        _ if count >= 10_000 => format!("{}K", count / 1000),
        _ if count >= 1_000 => format!("{:.1}K", count as f64 / 1e3),
        _ => count.to_string(),
    }
}

/// The state's glyph at `clock`: the starting quadrant turns at 4 Hz; needs-you ▲ is steady.
fn glyph(state: NodeState) -> &'static str {
    let glyphs = active().glyphs;
    match state {
        NodeState::Operator => glyphs.operator,
        NodeState::Starting => glyphs.starting,
        NodeState::Working => glyphs.working,
        NodeState::Idle => glyphs.idle,
        NodeState::Ended => glyphs.ended,
        NodeState::NeedsYou => glyphs.needs_you,
    }
}

/// A working node's border at `clock`: working colour breathing down to 45% and back.
fn breathing(clock: f64) -> Option<Color> {
    let theme = active();
    let working = theme.color(Role::Working);
    let surface = theme.color(Role::Surface);
    let breath = 0.5 + 0.5 * (clock * TAU / BREATH_PERIOD).sin();
    mix(mix(working, surface, 0.55), working, breath)
}

fn state_role(state: NodeState) -> Role {
    match state {
        NodeState::Operator => Role::Text,
        NodeState::Working => Role::Working,
        NodeState::NeedsYou => Role::Signal,
        NodeState::Starting | NodeState::Idle | NodeState::Ended => Role::Muted,
    }
}

fn quiet(node: &Node) -> bool {
    matches!(node.state, NodeState::Idle | NodeState::Ended)
}

pub fn render(app: &mut App, frame: &mut Frame) {
    // The sidebar never squeezes the graph's boxes.
    let width = frame.area().width;
    let side_width = if width >= SIDEBAR_FROM
        && width > layout::full_width(app.graph.columns()) + SIDEBAR_WIDTH
    {
        SIDEBAR_WIDTH
    } else {
        0
    };
    let [header, _, middle, legend, detail, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(u16::from(side_width == 0)),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [main, _, side] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(u16::from(side_width > 0)),
        Constraint::Length(side_width),
    ])
    .areas(middle);
    let [labels, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(main);
    app.arrange(body);
    frame.render_widget(header_line(app), header);
    frame.render_widget(Paragraph::new(footer_line(app)), footer);
    if app.mode == Mode::Help {
        frame.render_widget(help_page(), middle);
        return;
    }
    if app.graph.nodes.is_empty() {
        let text = format!(
            "  no agents running or active in the last {}",
            window_label(app.window)
        );
        frame.render_widget(Paragraph::new(Line::styled(text, dim())), middle);
        return;
    }
    frame.render_widget(column_labels(app, labels.width), labels);
    draw_canvas(app, frame, body);
    if side_width > 0 {
        draw_sidebar(app, frame, side);
    }
    frame.render_widget(legend_line(), legend);
    frame.render_widget(detail_line(app), detail);
}

/// The notice, else the open prompt or confirmation, else the key hints.
fn footer_line(app: &App) -> Line<'static> {
    if let Some(notice) = &app.notice {
        return Line::styled(format!("  {notice}"), role(Role::Signal));
    }
    let name = app
        .graph
        .nodes
        .get(app.selected)
        .map_or(String::new(), |node| node.name.clone());
    match &app.mode {
        Mode::Prompt { purpose, text } => {
            let prompt_glyph = active().glyphs.prompt;
            let label = match purpose {
                Purpose::Send => format!("  send to {name} {prompt_glyph} "),
                Purpose::Fork => {
                    format!("  fork {name} as (blank: {name}-fork) {prompt_glyph} ")
                }
            };
            Line::from(vec![
                Span::styled(label, bold(Role::Accent)),
                Span::styled(text.clone(), role(Role::Text)),
                Span::styled("▏", role(Role::Accent)),
                Span::styled("   enter · esc cancels", dim()),
            ])
        }
        Mode::ConfirmStop => Line::from(vec![
            Span::styled(format!("  stop {name}? "), bold(Role::Signal)),
            Span::styled("y stops · any other key cancels", dim()),
        ]),
        Mode::Help => Line::styled("  any key closes", dim()),
        Mode::Browse => {
            let group = app.group().unwrap_or("all");
            Line::styled(
                format!(
                    "  enter jump · s send · f fork · a artifacts · x stop · g group: {group} · ? keys"
                ),
                dim(),
            )
        }
    }
}

/// Every key, as home's `?` page lists them.
fn help_page() -> Paragraph<'static> {
    let mut lines = vec![Line::styled("  keys", bold(Role::Accent)), Line::raw("")];
    for (key, meaning) in KEYS {
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<14}"), role(Role::Text)),
            Span::styled(meaning, dim()),
        ]));
    }
    Paragraph::new(lines)
}

fn header_line(app: &App) -> Paragraph<'static> {
    let graph = &app.graph;
    let glyphs = active().glyphs;
    let count = |state: NodeState| {
        graph
            .nodes
            .iter()
            .filter(|node| node.state == state)
            .count()
    };
    let agents = graph.nodes.iter().filter(|node| node.id.is_some()).count();
    let scope = match (app.group(), app.filter.tag.as_deref()) {
        (None, None) => "all agents".to_owned(),
        (Some(group), None) => group.to_owned(),
        (None, Some(tag)) => format!("tag {tag}"),
        (Some(group), Some(tag)) => format!("{group} · tag {tag}"),
    };
    let mut spans = vec![
        Span::styled(format!(" {}", app.brand), bold(Role::Accent)),
        Span::styled("  graph  ", dim()),
        Span::styled(scope, bold(Role::Text)),
        Span::styled(format!("   {} · ", counted(agents, "agent")), dim()),
        Span::styled(glyphs.working, role(Role::Working)),
        Span::styled(format!(" {} working · ", count(NodeState::Working)), dim()),
    ];
    let waiting = count(NodeState::NeedsYou);
    if waiting > 0 {
        spans.push(Span::styled(glyphs.needs_you, bold(Role::Signal)));
        spans.push(Span::styled(format!(" {waiting} needs you · "), dim()));
    }
    spans.push(Span::styled("━", role(Role::Info)));
    spans.push(Span::styled(
        format!(
            " {} / {}",
            counted(graph.traffic.len(), "message"),
            window_label(graph.window)
        ),
        dim(),
    ));
    Paragraph::new(Line::from(spans))
}

fn column_labels(app: &App, width: u16) -> Paragraph<'static> {
    let mut text = String::new();
    for (depth, x) in app.placement.column_x.iter().enumerate() {
        let label = if depth == 0 {
            "depth 0".to_owned()
        } else {
            depth.to_string()
        };
        let x = usize::from(*x);
        if text.chars().count() < x {
            text.push_str(&" ".repeat(x - text.chars().count()));
        }
        text.push_str(&label);
    }
    Paragraph::new(Line::styled(cut(&text, usize::from(width)), dim()))
}

fn legend_line() -> Paragraph<'static> {
    let swatch = |glyph: &'static str, target: Role, label: &'static str| {
        [
            Span::styled(glyph, bold(target)),
            Span::styled(label, dim()),
        ]
    };
    let mut spans = vec![Span::raw("  ")];
    spans.extend(swatch("╭╮", Role::Accent, " selected   "));
    spans.extend(swatch("╭╮", Role::Working, " working   "));
    spans.extend(swatch("╭╮", Role::Signal, " needs you   "));
    spans.extend(swatch("╭╮", Role::Rule, " idle   "));
    spans.extend(swatch("⠉⠒⠤", Role::Info, " messages   "));
    spans.extend(swatch("⠁⠂⠄", Role::Rule, " lineage only"));
    Paragraph::new(Line::from(spans))
}

/// The selected node as `label value` pairs.
fn detail_line(app: &App) -> Paragraph<'static> {
    let graph = &app.graph;
    let node = &graph.nodes[app.selected];
    let mut spans = vec![Span::styled(format!("  {}  ", node.name), bold(Role::Text))];
    let mut pair = |label: &str, value: String| {
        if spans.len() > 1 {
            spans.push(Span::styled(" · ", dim()));
        }
        spans.push(Span::styled(format!("{label} "), dim()));
        spans.push(Span::styled(value, role(Role::Text)));
    };
    if node.state == NodeState::Operator {
        pair("sent", sent_by(graph, app.selected).to_string());
        return Paragraph::new(Line::from(spans));
    }
    if let Some(engine) = node.engine {
        pair("engine", engine.as_str().to_owned());
    }
    if let Some(definition) = &node.definition {
        pair("definition", definition.clone());
    }
    pair("group", node.group.clone());
    let parent = parent_label(graph, node);
    pair("parent", parent);
    let children = graph
        .nodes
        .iter()
        .filter(|child| child.parent == Some(app.selected))
        .count();
    pair("children", children.to_string());
    let messages: u32 = graph
        .edges
        .iter()
        .filter(|edge| edge.from == app.selected || edge.to == app.selected)
        .map(Edge::messages)
        .sum();
    pair("messages", messages.to_string());
    if let Some(context) = node.context {
        let value = match context.ratio() {
            Some(ratio) => format!("{} ({:.0}%)", tokens(context.tokens), ratio * 100.0),
            None => tokens(context.tokens),
        };
        pair("ctx", value);
    }
    Paragraph::new(Line::from(spans))
}

/// The selected node in full: facts, messages per minute, the context window, recent messages.
fn draw_sidebar(app: &App, frame: &mut Frame, area: Rect) {
    let graph = &app.graph;
    let node = &graph.nodes[app.selected];
    let theme = active();
    let title =
        tx_client::theme::titled(&cut(&node.name, usize::from(area.width.saturating_sub(6))));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(bold(Role::Accent))
        .title(Line::styled(title, theme.on_accent_badge()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [
        facts,
        _,
        spark_label,
        spark,
        _,
        gauge_label,
        gauge,
        _,
        recent_label,
        recent,
    ] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    let fact = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!(" {label:<12}"), dim()),
            Span::styled(value, role(Role::Text)),
        ])
    };
    let lines = if node.state == NodeState::Operator {
        vec![
            fact("role", "operator".to_owned()),
            fact("sent", sent_by(graph, app.selected).to_string()),
        ]
    } else {
        let parent = parent_label(graph, node);
        let children = graph
            .nodes
            .iter()
            .filter(|child| child.parent == Some(app.selected))
            .count();
        let active = node.last_activity.map_or("—".to_owned(), |at| {
            format!("{} ago", age(graph.now - at))
        });
        vec![
            fact(
                "engine",
                node.engine
                    .as_ref()
                    .map_or("—", |engine| engine.as_str())
                    .to_owned(),
            ),
            fact(
                "definition",
                node.definition.clone().unwrap_or("—".to_owned()),
            ),
            fact("parent", parent),
            fact("children", children.to_string()),
            fact("group", node.group.clone()),
            fact("last active", active),
        ]
    };
    frame.render_widget(Paragraph::new(lines), facts);

    let spark = spark.inner(Margin::new(1, 0));
    let minutes = (graph.window / 60.0)
        .ceil()
        .clamp(1.0, f64::from(spark.width.max(1)));
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(" messages / min · last {minutes}m"),
            dim(),
        )),
        spark_label,
    );
    let mut counts = vec![0u64; minutes as usize];
    for traffic in graph
        .traffic
        .iter()
        .filter(|traffic| traffic.from == app.selected || traffic.to == app.selected)
    {
        let back = ((graph.now - traffic.at) / 60.0).floor();
        if (0.0..minutes).contains(&back) {
            counts[minutes as usize - 1 - back as usize] += 1;
        }
    }
    frame.render_widget(
        Sparkline::default().data(&counts).style(role(Role::Info)),
        spark,
    );

    frame.render_widget(
        Paragraph::new(Line::styled(" context window", dim())),
        gauge_label,
    );
    let gauge = gauge.inner(Margin::new(1, 0));
    match node.context.map(|context| (context, context.ratio())) {
        Some((_, Some(ratio))) => frame.render_widget(
            LineGauge::default()
                .ratio(ratio)
                .label(Span::styled(
                    format!("{:>3.0}%", ratio * 100.0),
                    role(Role::Text),
                ))
                .filled_symbol("━")
                .unfilled_symbol("─")
                .filled_style(bold(if ratio > CONTEXT_WARN {
                    Role::Signal
                } else {
                    Role::Working
                }))
                .unfilled_style(role(Role::Rule)),
            gauge,
        ),
        Some((context, None)) => frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{} tokens", tokens(context.tokens)),
                    role(Role::Text),
                ),
                Span::styled(" · window unknown", dim()),
            ])),
            gauge,
        ),
        None => frame.render_widget(Paragraph::new(Line::styled("—", dim())), gauge),
    }

    frame.render_widget(Paragraph::new(Line::styled(" recent", dim())), recent_label);
    let width = usize::from(recent.width.saturating_sub(12)) / 2;
    let lines: Vec<Line> = graph
        .traffic
        .iter()
        .rev()
        .filter(|traffic| traffic.from == app.selected || traffic.to == app.selected)
        .take(usize::from(recent.height))
        .map(|traffic| {
            Line::from(vec![
                Span::styled(format!(" {} ", clock::clock_label(traffic.at)), dim()),
                Span::styled(
                    cut(&graph.nodes[traffic.from].name, width),
                    role(Role::Text),
                ),
                Span::styled(" ━▸ ", role(Role::Info)),
                Span::styled(cut(&graph.nodes[traffic.to].name, width), role(Role::Text)),
            ])
        })
        .collect();
    let lines = if lines.is_empty() {
        vec![Line::styled(" none in the window", dim())]
    } else {
        lines
    };
    frame.render_widget(Paragraph::new(lines), recent);
}

/// The parent's name, `<name> (hidden)` when a filter hides it, `ended` when its record
/// is gone, `—` for a root.
fn parent_label(graph: &Graph, node: &Node) -> String {
    match (node.parent, &node.hidden_parent) {
        (Some(parent), _) => graph.nodes[parent].name.clone(),
        (None, Some(name)) => format!("{name} (hidden)"),
        (None, None) if node.parent_lost => "ended".to_owned(),
        (None, None) => "—".to_owned(),
    }
}

fn sent_by(graph: &Graph, index: usize) -> usize {
    graph
        .traffic
        .iter()
        .filter(|traffic| traffic.from == index)
        .count()
}

fn draw_canvas(app: &App, frame: &mut Frame, view: Rect) {
    let height = app.placement.height.max(view.height).max(1);
    let area = Rect::new(0, 0, view.width, height);
    let mut buffer = Buffer::empty(area);
    draw_edges(app, area, &mut buffer);
    for (index, node) in app.graph.nodes.iter().enumerate() {
        let rect = app.placement.boxes[index];
        let selected = index == app.selected;
        if app.placement.compact {
            draw_compact_node(node, rect, selected, &mut buffer);
        } else {
            draw_node(app, index, rect, selected, &mut buffer);
        }
    }
    let target = frame.buffer_mut();
    for row in 0..view.height {
        let source_row = row + app.scroll;
        if source_row >= height {
            break;
        }
        for column in 0..view.width {
            target[(view.x + column, view.y + row)] = buffer[(column, source_row)].clone();
        }
    }
}

/// A cell position to canvas coordinates: x right, y up from the bottom row.
fn point(area: Rect, column: f64, row: f64) -> (f64, f64) {
    (column, f64::from(area.height - 1) - row)
}

type Curve = [(f64, f64); 4];

fn bezier(curve: &Curve, t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let weights = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    let mut point = (0.0, 0.0);
    for (weight, control) in weights.iter().zip(curve) {
        point.0 += weight * control.0;
        point.1 += weight * control.1;
    }
    point
}

/// An edge's route on the canvas, as a polyline walked by length.
struct Path {
    points: Vec<(f64, f64)>,
    /// The length up to each point.
    lengths: Vec<f64>,
}

impl Path {
    fn new(points: Vec<(f64, f64)>) -> Self {
        let mut lengths = vec![0.0];
        for pair in points.windows(2) {
            let step = (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
            lengths.push(lengths.last().copied().unwrap_or(0.0) + step);
        }
        Self { points, lengths }
    }

    fn curve(curve: &Curve) -> Self {
        Self::new(
            (0..=CURVE_STEPS)
                .map(|step| bezier(curve, f64::from(step) / f64::from(CURVE_STEPS)))
                .collect(),
        )
    }

    fn length(&self) -> f64 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// The point `t` (0..1) of the way along.
    fn at(&self, t: f64) -> (f64, f64) {
        let target = t.clamp(0.0, 1.0) * self.length();
        let segment = self
            .lengths
            .windows(2)
            .position(|pair| target <= pair[1])
            .unwrap_or(self.points.len().saturating_sub(2));
        let (start, end) = (self.points[segment], self.points[segment + 1]);
        let span = self.lengths[segment + 1] - self.lengths[segment];
        let along = if span > 0.0 {
            (target - self.lengths[segment]) / span
        } else {
            0.0
        };
        (
            start.0 + (end.0 - start.0) * along,
            start.1 + (end.1 - start.1) * along,
        )
    }

    /// Dashed: on and off every [`DASH`] cells. Thick: drawn twice, a quarter cell apart.
    fn draw(&self, context: &mut Context, color: Color, dashed: bool, thick: bool) {
        let pieces = (self.length() / DASH * 2.0).ceil().max(1.0) as u32;
        let mut previous = self.at(0.0);
        for piece in 1..=pieces {
            let next = self.at(f64::from(piece) / f64::from(pieces));
            if !dashed || (piece / 2) % 2 == 0 {
                context.draw(&Segment::new(previous.0, previous.1, next.0, next.1, color));
                if thick {
                    context.draw(&Segment::new(
                        previous.0 + 0.5,
                        previous.1 + 0.25,
                        next.0 + 0.5,
                        next.1 + 0.25,
                        color,
                    ));
                }
            }
            previous = next;
        }
    }
}

/// The row an edge meets a box at: the first inner line, or the only line of a compact box.
fn anchor_row(rect: Rect) -> f64 {
    f64::from(rect.y + 1.min(rect.height - 1))
}

/// The route of an edge. Within one column: an arc out to the right and back. Across columns,
/// from the right side of `from` to just before `to`, leaving the cell before `to` for the
/// arrowhead: a curve, or with a `bus` column, straight out to the bus, along it to the
/// target's row and across, so a fan shares one trunk.
fn route(area: Rect, from: Rect, to: Rect, bus: Option<f64>, end_row: f64) -> Path {
    let start = point(area, f64::from(from.right()), anchor_row(from));
    if from.x == to.x {
        let end = point(area, f64::from(to.right()), anchor_row(to));
        return Path::curve(&[
            start,
            (start.0 + ARC_BULGE, start.1),
            (end.0 + ARC_BULGE, end.1),
            end,
        ]);
    }
    let end = point(area, f64::from(to.x) - 2.0, end_row);
    if let Some(bus) = bus {
        return Path::new(vec![start, (bus, start.1), (bus, end.1), end]);
    }
    let reach = (end.0 - start.0) * 0.55;
    Path::curve(&[
        start,
        (start.0 + reach, start.1),
        (end.0 - reach, end.1),
        end,
    ])
}

/// Where an edge comes into its target: the box row, and how far left of the arrowhead its
/// badge moves when the box has fewer rows than incoming edges.
#[derive(Clone, Copy)]
struct Arrival {
    row: f64,
    badge_shift: f64,
}

/// Each edge's route and arrival. A node with two or more edges into later columns sends them
/// along one bus just right of it. A node's incoming edges from earlier columns arrive on
/// different rows of its box (top to bottom by source), so their badges never overwrite each
/// other; past the box's rows they line up leftwards.
fn routes(app: &App, area: Rect) -> (Vec<Path>, Vec<Arrival>) {
    let graph = &app.graph;
    let boxes = &app.placement.boxes;
    let across = |edge: &Edge| boxes[edge.from].x != boxes[edge.to].x;
    let mut fan = vec![0usize; graph.nodes.len()];
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    for (index, edge) in graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| across(edge))
    {
        fan[edge.from] += 1;
        incoming[edge.to].push(index);
    }
    let mut arrivals = vec![
        Arrival {
            row: 0.0,
            badge_shift: 0.0
        };
        graph.edges.len()
    ];
    for (target, edges) in incoming.iter_mut().enumerate() {
        let rect = boxes[target];
        edges.sort_by_key(|index| boxes[graph.edges[*index].from].y);
        let rows: Vec<u16> = ARRIVAL_ROWS
            .into_iter()
            .filter(|row| *row < rect.height)
            .collect();
        for (slot, index) in edges.iter().enumerate() {
            arrivals[*index] = Arrival {
                row: f64::from(rect.y + rows[slot % rows.len()]),
                badge_shift: (slot / rows.len()) as f64 * BADGE_SHIFT,
            };
        }
    }
    let paths = graph
        .edges
        .iter()
        .zip(&arrivals)
        .map(|(edge, arrival)| {
            let from = boxes[edge.from];
            let bus =
                (across(edge) && fan[edge.from] > 1).then(|| f64::from(from.right()) + BUS_OFFSET);
            route(area, from, boxes[edge.to], bus, arrival.row)
        })
        .collect();
    (paths, arrivals)
}

fn draw_edges(app: &App, area: Rect, buffer: &mut Buffer) {
    let graph = &app.graph;
    let boxes = &app.placement.boxes;
    let (paths, arrivals) = routes(app, area);
    let theme = active();
    let busiest = graph
        .edges
        .iter()
        .map(Edge::messages)
        .max()
        .unwrap_or(0)
        .max(1);
    let touches = |edge: &Edge| edge.from == app.selected || edge.to == app.selected;
    let edge_colour = |edge: &Edge| {
        if edge.messages() == 0 {
            theme.color(Role::Rule)
        } else if touches(edge) {
            theme.color(Role::Info)
        } else {
            let strength = 0.35 + 0.65 * f64::from(edge.messages()) / f64::from(busiest);
            mix(
                theme.color(Role::Surface),
                theme.color(Role::Info),
                strength * 0.7,
            )
        }
    };
    // Quiet edges first, the selection's last, so a shared trunk shows its brightest use.
    let mut order: Vec<usize> = (0..graph.edges.len()).collect();
    order.sort_by_key(|index| {
        let edge = &graph.edges[*index];
        (touches(edge), edge.messages())
    });
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([0.0, f64::from(area.width.max(2) - 1)])
        .y_bounds([0.0, f64::from(area.height.max(2) - 1)])
        .paint(|context| {
            for index in &order {
                let edge = &graph.edges[*index];
                let quiet = edge.messages() == 0;
                paths[*index].draw(
                    context,
                    edge_colour(edge).unwrap_or(Color::Reset),
                    quiet,
                    touches(edge) && !quiet,
                );
            }
            for (index, node) in graph.nodes.iter().enumerate() {
                if node.parent_lost {
                    let rect = boxes[index];
                    let end = point(area, f64::from(rect.x) - 2.0, anchor_row(rect));
                    let start = (end.0 - STUB_LENGTH, end.1);
                    Path::new(vec![start, end]).draw(context, color_of(Role::Rule), true, false);
                    context.print(start.0, start.1 + 1.0, Line::styled("parent ended", dim()));
                }
            }
            for pulse in &app.pulses {
                let progress = (app.clock - pulse.started) / PULSE_TRAVEL;
                let from = match &pulse.from {
                    Some(id) => graph.index_of(id),
                    None => graph
                        .nodes
                        .first()
                        .filter(|node| node.state == NodeState::Operator)
                        .map(|_| 0),
                };
                let (Some(from), Some(to)) = (from, graph.index_of(&pulse.to)) else {
                    continue;
                };
                let Some(index) = graph.edges.iter().position(|edge| {
                    (edge.from, edge.to) == (from, to) || (edge.from, edge.to) == (to, from)
                }) else {
                    continue;
                };
                let edge = &graph.edges[index];
                draw_pulse(
                    context,
                    &paths[index],
                    progress,
                    edge.from != from,
                    edge_colour(edge),
                );
            }
            for (index, edge) in graph.edges.iter().enumerate() {
                draw_ends(
                    context,
                    edge,
                    &paths[index],
                    arrivals[index].badge_shift,
                    boxes,
                    touches(edge),
                    edge_colour(edge),
                );
            }
        });
    canvas.render(area, buffer);
}

/// An edge's count badge and arrowheads. Across columns they sit at the target end, on the
/// target's row (`◂ 7 ▸`: messages both ways), so badges of a fan never stack; within one
/// column the badge sits on the arc's bulge.
fn draw_ends(
    context: &mut Context,
    edge: &Edge,
    path: &Path,
    badge_shift: f64,
    boxes: &[Rect],
    selected: bool,
    edge_colour: Option<Color>,
) {
    let theme = active();
    let badge_style = if selected {
        match theme.color(Role::Info) {
            Some(bg) => bold(Role::OnAccent).bg(bg),
            None => Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        }
    } else {
        match theme.color(Role::Surface) {
            Some(bg) => bold(Role::Info).bg(bg),
            None => bold(Role::Info),
        }
    };
    let badge = format!(" {} ", edge.messages());
    if boxes[edge.from].x == boxes[edge.to].x {
        if edge.messages() > 0 {
            let (x, y) = path.at(0.5);
            context.print(x - 1.0, y, Line::styled(badge, badge_style));
        }
        return;
    }
    let arrow = if selected {
        role(Role::Info)
    } else {
        style_of_color(edge_colour)
    };
    let (end_x, end_y) = path.at(1.0);
    let head = match (edge.forward, edge.backward) {
        (0, 0) => "▹",
        (0, _) => "◂",
        _ => "▸",
    };
    context.print(end_x + 1.0, end_y, Line::styled(head, arrow));
    if edge.messages() == 0 {
        return;
    }
    let badge_x = end_x + 1.0 - badge.chars().count() as f64 - badge_shift;
    context.print(badge_x, end_y, Line::styled(badge, badge_style));
    if edge.forward > 0 && edge.backward > 0 {
        context.print(badge_x - 1.0, end_y, Line::styled("◂", arrow));
    }
}

/// A message `progress` (0..1) of the way along `path`, backwards when `reverse`: a Text head
/// and a trail fading from Info into the edge's colour.
fn draw_pulse(
    context: &mut Context,
    path: &Path,
    progress: f64,
    reverse: bool,
    edge: Option<Color>,
) {
    if !(0.0..=1.0).contains(&progress) {
        return;
    }
    let theme = active();
    // The trail keeps its length in cells, whatever the path's length.
    let step = PULSE_TRAIL_STEP * CURVE_REFERENCE_LENGTH / path.length().max(1.0);
    for trail in 0..PULSE_TRAIL {
        let along = progress - f64::from(trail) * step;
        if !(0.0..=1.0).contains(&along) {
            continue;
        }
        let (x, y) = path.at(if reverse { 1.0 - along } else { along });
        let shade = if trail == 0 {
            theme.color(Role::Text)
        } else {
            mix(
                theme.color(Role::Info),
                edge,
                f64::from(trail) / f64::from(PULSE_TRAIL),
            )
        };
        context.draw(&Points {
            coords: &[(x, y), (x, y + 0.25), (x + 0.5, y)],
            color: shade.unwrap_or(Color::Reset),
        });
    }
}

fn title_style(node: &Node, selected: bool) -> Style {
    if selected {
        active().on_accent_badge()
    } else if quiet(node) || node.connector {
        dim()
    } else {
        bold(Role::Text)
    }
}

fn draw_node(app: &App, index: usize, rect: Rect, selected: bool, buffer: &mut Buffer) {
    let node = &app.graph.nodes[index];
    let state = node.state;
    let theme = active();
    let border_style = if selected {
        bold(Role::Accent)
    } else {
        match state {
            NodeState::Working => style_of_color(breathing(app.clock)).add_modifier(Modifier::BOLD),
            NodeState::NeedsYou => bold(Role::Signal),
            NodeState::Operator => bold(Role::Text),
            NodeState::Starting | NodeState::Idle | NodeState::Ended => role(Role::Rule),
        }
    };
    let name_style = title_style(node, selected);
    let glyph_style = if selected {
        name_style
    } else {
        bold(state_role(state))
    };
    let room = usize::from(rect.width.saturating_sub(8));
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(glyph(state), glyph_style),
        Span::styled(format!(" {} ", cut(&node.name, room)), name_style),
    ]);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title);
    let inner = block.inner(rect).inner(Margin::new(1, 0));
    Clear.render(rect, buffer);
    block.render(rect, buffer);
    let [status_area, context_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    if state == NodeState::Operator {
        let sent = sent_by(&app.graph, index);
        Paragraph::new(Line::from(vec![
            Span::styled(sent.to_string(), role(Role::Text)),
            Span::styled(" sent", dim()),
        ]))
        .render(status_area, buffer);
        return;
    }
    let since = node.last_activity.unwrap_or(node.created_at);
    let status = Line::from(vec![
        Span::styled(state.word(), role(state_role(state))),
        Span::styled(format!(" · {}", age(app.graph.now - since)), dim()),
    ]);
    Paragraph::new(status).render(status_area, buffer);
    let Some(context) = node.context else {
        return;
    };
    match context.ratio() {
        Some(ratio) => {
            let filled_style = if ratio > CONTEXT_WARN {
                role(Role::Signal)
            } else {
                style_of_color(mix(theme.color(Role::Info), theme.color(Role::Accent), 0.5))
            };
            LineGauge::default()
                .ratio(ratio)
                .label(Span::styled("ctx", dim()))
                .filled_symbol("━")
                .unfilled_symbol("─")
                .filled_style(filled_style)
                .unfilled_style(role(Role::Rule))
                .render(context_area, buffer);
        }
        None => Paragraph::new(Line::styled(
            format!("ctx {}", tokens(context.tokens)),
            dim(),
        ))
        .render(context_area, buffer),
    }
}

/// One borderless line: `glyph name`.
fn draw_compact_node(node: &Node, rect: Rect, selected: bool, buffer: &mut Buffer) {
    let name_style = title_style(node, selected);
    let glyph_style = if selected {
        name_style
    } else {
        bold(state_role(node.state))
    };
    let room = usize::from(rect.width.saturating_sub(3));
    Clear.render(rect, buffer);
    Paragraph::new(Line::from(vec![
        Span::styled(glyph(node.state), glyph_style),
        Span::styled(format!(" {:<room$}", cut(&node.name, room)), name_style),
    ]))
    .render(rect, buffer);
}

/// The buffer as plain text, one line per row, trailing spaces trimmed.
pub fn buffer_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut lines = Vec::with_capacity(usize::from(area.height));
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}
