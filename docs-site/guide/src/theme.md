# The look and the theme

tx draws the frame round the agents and the screens between them, never the inside of an agent's
terminal. This page says what that frame is made of, which parts are yours to change, and how a
change reaches every surface.

## What is tx's and what is yours

| | whose | where it is defined |
|---|---|---|
| the mark `╋╳`, the box-drawn `tx` banner | tx's | `tx-client/src/theme.rs` |
| line weights: heavy is tx's frame, light is content, rounded is the operator | tx's | the same |
| a title set into its border, `┫ graph ┣` on a heavy line, `┤ new agent ├` on a light or rounded one | tx's | the same |
| one glyph per agent state: `●` working, `▖▘▝▗` starting, `▲` needs you, `○` idle, `×` ended | tx's | the same |
| every colour, by role | yours | `theme:` in `config.yaml`, a file in `<config>/themes/` |
| the status line: a rule or a bar, and what is on it | yours | `theme.status_style`; tmux's own `status-left`, `status-right` and window formats |
| Unicode or ASCII glyphs | yours | `theme.glyphs` |
| the home's backdrop | yours | the `home-themes` plugin, `b` in the home |

Nothing in tx's part is a colour, so a theme cannot remove it, and it is the same in 16 colours
and with `NO_COLOR`.

## The roles

A theme gives a colour to each of twelve roles. No plugin holds a colour of its own.

| role | used for |
|---|---|
| `text` | primary text |
| `muted` | secondary text, hints, idle and ended agents |
| `rule` | inactive borders, the status rule |
| `rule_active` | the active pane's border and label, a popup's border |
| `signal` | an agent that needs you: the one colour allowed on the frame round an agent |
| `accent` | selection and focus inside tx's own screens (the home, the graph, the board, the timeline, the pickers, the forms) |
| `on_accent` | text on an accent fill |
| `working` | a working agent, inside tx's own screens |
| `info` | messages on the graph's edges, a filter being typed |
| `failed` | a failed agent, a destructive prompt |
| `surface` | a popup's background, the status bar's when `status_style` is `bar` |
| `selection` | the selected row's background |

The frame round an agent (pane borders, pane labels, the status rule) uses `rule`,
`rule_active`, `muted` and `signal` only. If something on it is coloured, it wants you.

## The shipped themes

- `tx-dark` and `tx-light`: the default look. The frame has no hue but `signal`; tx's own screens
  use a teal `accent`.
- `terminal`: every role is the terminal's own foreground, one of its sixteen colours, or an
  attribute (bold, dim, reverse). It is right on any terminal theme without knowing it.
- `tokyo-night`: tx's look before this one.
- `auto`, the default: `tx-dark` on a dark terminal, `tx-light` on a light one, and `terminal`
  when the terminal has not said which it is.

Each role in a shipped theme has three values: a 24-bit colour, a chosen entry of the 256-colour
palette, and a chosen one of the terminal's sixteen. They are chosen by hand, because a
terminal's own approximation moves hues (tmux turns tx's amber into red on an 8-colour client).

## Changing it

In `config.yaml`:

```yaml
theme:
  name: auto            # auto | tx-dark | tx-light | terminal | tokyo-night | a file in themes/
  status_style: rule    # rule: the bottom edge of the frame | bar: a filled tmux bar
  glyphs: unicode       # unicode | ascii
  colors:               # optional: roles to replace in the named theme
    accent: "#ff75b5"
    signal: yellow
```

`theme: tokyo-night` is short for `theme: { name: tokyo-night }`. A colour is `#rrggbb` or one of
the terminal's sixteen by name: `default`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
`cyan`, `white`, and `bright-black` to `bright-white`. From a `#rrggbb` tx derives the 256-colour
and 16-colour values itself.

A theme of your own is a file, `<config>/themes/<name>.yaml`, named by `theme.name`:

```yaml
base: tx-dark           # the theme it starts from; auto when left out
colors:                 # roles replaced on any background
  working: green
dark:                   # roles replaced on a dark terminal only
  accent: "#ff75b5"
light:                  # and on a light one
  accent: "#a01060"
```

**Applying a change.** Run `tx update`. It rewrites `<data>/conf/theme.tmux` and
`<data>/conf/theme.lua`, and a running server takes the new tmux options at once: borders,
popups and the status line change in place. A screen that is already open (the home, a graph)
keeps the theme it started with until it is opened again.

**A mistake is reported, not skipped.** A role that does not exist, a colour that is neither
form, a theme file that is missing or has an unknown key: `tx update` fails with the reason and
leaves the previous theme files as they were.

```text
tx: update: ide: theme: unknown role `acent` (the roles are text, muted, rule, …)
```

## The status line

With `status_style: rule`, the status line has no background: it is the bottom edge of the
frame, a heavy rule with the active window set into it.

```text
╋╳ ▲ issue-134 ┫ 1:agents ┣ 2:review ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ CPU 12%  MEM 34%  20:18  work
```

tmux's own slots are kept, so they are still yours: `status-left` (the mark, then the
`attention` plugin's queue: at most two named agents, then `▲ +N`), `window-status-format`,
`window-status-current-format` with their styles, and `status-right`. Only the gap between the
windows and `status-right` is tx's: it is filled with the rule. The mark is the tmux option
`@tx-mark`, so a `status-left` of your own can keep it: `set -g status-left "#{@tx-mark} …"`.

With `status_style: bar` tx leaves tmux's `status-format` alone and gives the bar the
`surface` colour.

## How the theme reaches each surface

`tx-client`'s `theme` module resolves the theme once per process: the configuration, then
whether colour is allowed, then the terminal's background, then the colour depth.

- **Colour allowed.** A non-empty `NO_COLOR`, `TERM=dumb` or `TX_COLORS=none` removes every
  colour and keeps the attributes: bold for what is active, dim for what is not, reverse for a
  selection and for `signal`. (ratatui is handed no colour at all in that case: `crossterm`
  0.29 otherwise writes a reset after the attributes and wipes them.)
- **Background.** tmux asks the terminal when a client attaches, and tells tx through its
  `client-dark-theme` and `client-light-theme` hooks, which run `tx _theme-apply dark|light`.
  That verb regenerates the theme files and records the answer in
  `<data>/plugins/ide/terminal-background`, which is where a screen started later reads it. `TX_THEME_POLARITY=dark|light` overrides it.
  Needs tmux 3.6.
- **Depth.** `TX_COLORS` (`truecolor`, `256`, `16`, `none`), else `COLORTERM` and `TERM`.

Then:

- **ratatui screens** (home, graph, board, timeline, mockup) take styles from
  `tx_client::theme::active()`. The setup checklist and the plugin switcher draw with bold, dim
  and reverse only.
- **tmux** sources `<data>/conf/theme.tmux` after `conf/tmux.conf`: user options (`@tx-mark`,
  `@tx-rule-style`, `@tx-signal-style`, …), the pane border and popup options, the status
  format. The plugins' key fragments and scripts refer to those options instead of a colour.
- **fzf** (the picker, the prompts) gets its `--color` argument from the theme.
- **nvim forms** read `<data>/conf/theme.lua`. The shipped nvim configuration keeps its own
  colour scheme.
- **The home's backdrops** keep their own scene colours: a scene is a picture.
