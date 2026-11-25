# The look and the theme

tx draws the frame round the agents and the screens between them, never the inside of an agent's
terminal. This page says what that frame is made of, which parts are yours to change, and how a
change reaches every surface.

## What is tx's and what is yours

| | whose | where it is defined |
|---|---|---|
| the mark: the letters `tx` at text size, a pixel `tx` above the home's list | tx's | `tx-client/src/theme.rs` |
| one glyph per agent state: `●` working, `◌` starting, `▲` needs you, `○` idle, `×` ended | tx's | the same |
| a header bar on each pane of a view, chips in the status line, light rounded popups | tx's | the same, and the plugins' key fragments |
| every colour, by role | yours | `theme:` in `config.yaml`, a file in `<config>/themes/` |
| what the status line holds | yours | tmux's own `status-left`, `status-right` and window formats |
| Unicode or ASCII glyphs | yours | `theme.glyphs` |
| the home's backdrop | yours | the `home-themes` plugin, `b` in the home |

Nothing in tx's part is a colour, so a theme cannot remove it.

The logo is drawn with half-blocks, two pixels to a cell, which is what the home's backdrops are
made of: it sits in the scene, shaded by it, instead of on top of it. At text size the mark is
plain letters, because a drawn mark of two cells is not legible at 12 points.

## The default: the terminal's own colours

Out of the box tx has no palette. The `terminal` theme gives every role the terminal's own
foreground, one of its sixteen colours, or an attribute, so tx wears whatever theme the terminal
has: Tokyo Night in a Tokyo Night terminal, Solarized in a Solarized one.

| role | used for | in `terminal` |
|---|---|---|
| `text` | primary text | the default foreground |
| `muted` | secondary text, hints, idle and ended agents | the default foreground, dim |
| `rule` | lines and borders inside tx's screens | bright black |
| `rule_active` | a popup's border, the active pane's line when there are no fills | blue |
| `accent` | the mark, selection and focus | blue |
| `on_accent` | text on an accent fill: the `tx` chip | black on a dark terminal, bright white on a light one |
| `signal` | an agent that needs you | yellow, bold, on a dark terminal; reverse video on a light one, where yellow cannot be read |
| `working` | a working agent | green |
| `info` | messages on the graph's edges, a filter being typed | cyan |
| `failed` | a failed agent, a destructive prompt | red |
| `surface` | the status bar, a pane's header, the home's card | black on a dark terminal, white on a light one |
| `selection` | the selected row, the active pane's header, the active window's chip | a neutral grey of the 256-colour palette: 236 on dark, 254 on light |

`surface` and `selection` are the theme's two **fills**. They need a known background. tmux asks
the terminal whether it is dark or light when a client attaches; until a terminal has answered,
and in 16 colours, there are no fills: panes are divided by light lines instead of header bars,
the selected row is reverse video, the status line has no background.

The other shipped theme is `tokyo-night`: fixed 24-bit colours, tx's look before it took the
terminal's.

## Changing it

In `config.yaml`:

```yaml
theme:
  name: terminal        # terminal | tokyo-night | a file in themes/
  glyphs: unicode       # unicode | ascii
  colors:               # optional: roles to replace in the named theme
    accent: magenta
    signal: "#ffb000"
```

`theme: tokyo-night` is short for `theme: { name: tokyo-night }`. A colour is one of the
terminal's sixteen by name (`default`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
`cyan`, `white`, and `bright-black` to `bright-white`) or `#rrggbb`. From a `#rrggbb` tx derives
the 256-colour and 16-colour values itself.

A theme of your own is a file, `<config>/themes/<name>.yaml`, named by `theme.name`:

```yaml
base: terminal          # the theme it starts from; terminal when left out
colors:                 # roles replaced on any background
  working: cyan
dark:                   # roles replaced on a dark terminal only
  accent: "#ff75b5"
light:                  # and on a light one
  accent: "#a01060"
```

**Applying a change.** Run `tx update`. It rewrites `<data>/conf/theme.tmux` and
`<data>/conf/theme.lua`, and a running server takes the new tmux options at once: headers,
popups and the status line change in place. A screen that is already open (the home, a graph)
keeps the theme it started with until it is opened again.

**A mistake is reported, not skipped.** A role that does not exist, a colour that is neither
form, a theme file that is missing or has an unknown key: `tx update` fails with the reason and
leaves the previous theme files as they were.

```text
tx: update: ide: theme: unknown role `acent` (the roles are text, muted, rule, …)
```

## The frame in tmux

`<data>/conf/theme.tmux` sets tmux's own options and a handful of user options that the
plugins' formats refer to, so no fragment holds a colour.

- **The status line.** A bar in `surface`. `status-left` is the mark as a chip
  (`#[#{@tx-chip-style}] #{@tx-mark} `), then the `attention` plugin's queue: at most two named
  agents that need you, then `▲ +N`. The windows follow, the active one a chip in `selection`.
  `status-right` is the swarm counted by state (`2● 1▲ 3○`, the option `@tx-swarm`), the machine's
  load and the clock. All three are tmux's own slots and stay yours; a `status-left` of your own
  keeps the mark by keeping `#{@tx-mark}`.
- **Pane headers.** In a view each pane has a header row (`pane-border-status top`): its number,
  the agent's state glyph, its name, how long it has rested, its tags. The state leads, so it is
  at the left however wide the pane is. With fills the header is a bar and panes are divided by a
  blank column (`pane-border-lines spaces`); without, by a light line.
- **Popups.** One light rounded border in `rule_active`, the terminal's background, the title
  `tx · <what it is>`. The forms (new agent, fork, edit) are popups like the others.
- **The window title.** `tx · <session>`, so the terminal's own tab says tx.

## How the theme reaches each surface

`tx-client`'s `theme` module resolves the theme once per process: the configuration, then
whether colour is allowed, then the terminal's background, then the colour depth.

- **Colour allowed.** A non-empty `NO_COLOR`, `TERM=dumb` or `TX_COLORS=none` removes every
  colour and keeps the attributes: bold for what is active, dim for what is not, reverse for a
  selection and for `signal`. (ratatui is handed no colour at all in that case: `crossterm`
  0.29 otherwise writes a reset after the attributes and wipes them.)
- **Background.** tmux tells tx through its `client-dark-theme` and `client-light-theme` hooks,
  which run `tx _theme-apply dark|light`; `tx update` asks the attached clients. The answer is
  recorded in `<data>/plugins/ide/terminal-background`, which is where a screen started later
  reads it. `TX_THEME_POLARITY=dark|light` overrides it. Needs tmux 3.6.
- **Depth.** `TX_COLORS` (`truecolor`, `256`, `16`, `none`), else `COLORTERM` and `TERM`.

Then:

- **ratatui screens** (home, graph, board, timeline, mockup) take styles from
  `tx_client::theme::active()`. The setup checklist and the plugin switcher draw with bold, dim
  and reverse only.
- **tmux** sources `<data>/conf/theme.tmux` after `conf/tmux.conf`.
- **fzf** (the picker, the prompts) gets its `--color` argument from the theme.
- **nvim forms** read `<data>/conf/theme.lua`: each role as highlight attributes, 24-bit or the
  terminal's palette. The shipped nvim configuration keeps its own colour scheme.
- **The home's backdrops** keep their own scene colours, since a scene is a picture, and shade
  the logo from the scene. Over a backdrop the home's list sits in a card in `surface`, so no
  fish swims behind a name.
