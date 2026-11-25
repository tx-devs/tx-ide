//! fzf's query editor as a one-line prompt.

use std::process::{Command, Stdio};

use crate::theme::active;

/// fzf's query editor prefilled with `default`; `None` when cancelled.
pub fn prompt_with_default(header: &str, default: &str) -> std::io::Result<Option<String>> {
    let theme = active();
    let prompt = format!("  {} ", theme.glyphs.prompt);
    let colors = theme.fzf_color_arg(false);
    let output = Command::new("fzf")
        .args([
            "--disabled",
            "--query",
            default,
            "--ansi",
            "--prompt",
            &prompt,
            "--header",
            header,
            "--header-first",
            "--reverse",
            "--no-info",
            "--no-separator",
            "--bind",
            "enter:accept-or-print-query",
            &colors,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(Some(text.strip_suffix('\n').unwrap_or(&text).to_owned()))
}
