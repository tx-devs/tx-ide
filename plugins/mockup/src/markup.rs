//! Inline colour markup → ratatui lines. `[spec]` opens a style, `[/]` closes the innermost
//! one, styles stack, `[[` is a literal `[`. Anything unknown renders literally; this never
//! fails.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tx_client::theme::{ColorDepth, Role, active};

pub fn lines(text: &str) -> Vec<Line<'static>> {
    text.split('\n').map(line).collect()
}

fn line(text: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut stack: Vec<Style> = Vec::new();
    let mut buffer = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        buffer.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(stripped) = after.strip_prefix('[') {
            buffer.push('[');
            rest = stripped;
            continue;
        }
        let Some(close) = after.find(']') else {
            buffer.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = &after[..close];
        rest = &after[close + 1..];
        let style_before = stack.last().copied().unwrap_or_default();
        if tag == "/" {
            if stack.pop().is_none() {
                buffer.push_str("[/]");
                continue;
            }
        } else {
            match parse_spec(tag) {
                Some(style) => stack.push(style_before.patch(style)),
                None => {
                    buffer.push('[');
                    buffer.push_str(tag);
                    buffer.push(']');
                    continue;
                }
            }
        }
        flush(&mut spans, &mut buffer, style_before);
    }
    buffer.push_str(rest);
    flush(
        &mut spans,
        &mut buffer,
        stack.last().copied().unwrap_or_default(),
    );
    if spans.is_empty() {
        spans.push(Span::raw(""));
    }
    Line::from(spans)
}

fn flush(spans: &mut Vec<Span<'static>>, buffer: &mut String, style: Style) {
    if buffer.is_empty() {
        return;
    }
    spans.push(Span::styled(std::mem::take(buffer), style));
}

fn parse_spec(spec: &str) -> Option<Style> {
    if spec.trim().is_empty() {
        return None;
    }
    let mut style = Style::default();
    let mut tokens = spec.split_whitespace();
    while let Some(token) = tokens.next() {
        match token {
            "bold" => style = style.add_modifier(Modifier::BOLD),
            "dim" => style = style.add_modifier(Modifier::DIM),
            "italic" => style = style.add_modifier(Modifier::ITALIC),
            "reverse" => style = style.add_modifier(Modifier::REVERSED),
            "underline" => style = style.add_modifier(Modifier::UNDERLINED),
            "on" => {
                if let Some(color) = colour(tokens.next()?)? {
                    style = style.bg(color);
                }
            }
            _ => {
                if let Some(color) = colour(token)? {
                    style = style.fg(color);
                }
            }
        }
    }
    Some(style)
}

/// `Some(Some(color))` for a known colour name with a resolved colour, `Some(None)` when the
/// name is known but `NO_COLOR` suppresses colour, `None` for an unknown tag.
fn colour(name: &str) -> Option<Option<Color>> {
    let theme = active();
    let standard = |color: Color| {
        if theme.depth == ColorDepth::None {
            None
        } else {
            Some(color)
        }
    };
    Some(match name {
        "black" => standard(Color::Black),
        "yellow" => standard(Color::Yellow),
        "blue" => standard(Color::Blue),
        "magenta" => standard(Color::Magenta),
        "cyan" => standard(Color::Cyan),
        "white" => standard(Color::White),
        "gray" => standard(Color::Gray),
        "accent" => theme.color(Role::Accent),
        "warn" | "signal" => theme.color(Role::Signal),
        "fg" => theme.color(Role::Text),
        "info" => theme.color(Role::Info),
        "green" => theme.color(Role::Working),
        "red" => theme.color(Role::Failed),
        "dim-fg" => theme.color(Role::Muted),
        "border" => theme.color(Role::Rule),
        "on-accent" => theme.color(Role::OnAccent),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::lines;
    use ratatui::style::{Color, Modifier};
    use tx_client::theme::{ColorDepth, Role, active};

    #[test]
    fn plain_text_one_span() {
        let lines = lines("hello");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content, "hello");
    }

    #[test]
    fn green_span() {
        let lines = lines("a[green]b[/]c");
        let spans = &lines[0].spans;
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "a");
        assert_eq!(spans[1].content, "b");
        assert_eq!(spans[1].style.fg, active().color(Role::Working));
        assert_eq!(spans[2].content, "c");
        assert_eq!(spans[2].style.fg, None);
    }

    #[test]
    fn nested_styles_stack() {
        let lines = lines("[bold][red]x[/][/]");
        let span = &lines[0].spans[0];
        assert_eq!(span.content, "x");
        assert_eq!(span.style.fg, active().color(Role::Failed));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn on_background() {
        let lines = lines("[bold yellow on blue]x[/]");
        let span = &lines[0].spans[0];
        let expected_fg = (active().depth != ColorDepth::None).then_some(Color::Yellow);
        let expected_bg = (active().depth != ColorDepth::None).then_some(Color::Blue);
        assert_eq!(span.style.fg, expected_fg);
        assert_eq!(span.style.bg, expected_bg);
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn palette_accent() {
        let lines = lines("[accent]x[/]");
        assert_eq!(lines[0].spans[0].style.fg, active().color(Role::Accent));
    }

    #[test]
    fn palette_names_match_the_ui() {
        let theme = active();
        let lines = lines("[dim-fg]a[/][border]b[/][on-accent on green]c[/]");
        let spans = &lines[0].spans;
        assert_eq!(spans[0].style.fg, theme.color(Role::Muted));
        assert_eq!(spans[1].style.fg, theme.color(Role::Rule));
        assert_eq!(spans[2].style.fg, theme.color(Role::OnAccent));
        assert_eq!(spans[2].style.bg, theme.color(Role::Working));
    }

    #[test]
    fn escaped_bracket() {
        let lines = lines("[[x]");
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[x]");
    }

    #[test]
    fn unknown_tag_is_literal() {
        let lines = lines("[nope]x");
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[nope]x");
    }

    #[test]
    fn unclosed_is_fine() {
        let lines = lines("[red]x");
        let span = &lines[0].spans[0];
        assert_eq!(span.content, "x");
        assert_eq!(span.style.fg, active().color(Role::Failed));
    }

    #[test]
    fn newline_splits_lines() {
        let lines = lines("a\nb");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "a");
        assert_eq!(lines[1].spans[0].content, "b");
    }
}
