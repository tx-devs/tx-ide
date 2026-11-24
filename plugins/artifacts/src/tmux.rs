//! The tmux calls prefix+a needs beyond `tx_client::tmux::Ui` (a client's size, the pane a
//! nested client paints into), ported from main's `tmux.rs`. All go through `Ui::run` / `Ui::quiet`,
//! so every call carries the core's `-S` and `-f`. Parsers are pure.

use tx_client::tmux::Ui;

pub use tx_client::tmux::DIM_FG_HEX;

/// `client`'s terminal size, `(columns, rows)`; `None` when there is no such client. Read from
/// `list-clients`: `display-message -c` expands its formats for whichever client tmux deems
/// current, not necessarily `client`.
pub fn client_size(ui: &Ui, client: &str) -> Option<(usize, usize)> {
    let (_, clients) = ui.quiet(&[
        "list-clients",
        "-F",
        "#{client_name}\t#{client_width}\t#{client_height}",
    ]);
    clients.lines().find_map(|line| {
        let mut fields = line.split('\t');
        (fields.next()? == client).then_some(())?;
        Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
    })
}

/// The pane whose tty a nested client runs on (a client's name is its tty): the pane that
/// client paints into. `None` for a terminal client.
pub fn pane_hosting_client(ui: &Ui, client: &str) -> Option<String> {
    let (_, panes) = ui.quiet(&["list-panes", "-a", "-F", "#{pane_tty}\t#{pane_id}"]);
    panes
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(tty, _)| *tty == client)
        .map(|(_, pane_id)| pane_id.to_owned())
}
