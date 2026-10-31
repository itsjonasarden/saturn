# Working on Saturn

Saturn is a native macOS workbench for running many Claude Code agents side by side: a grid of
terminals, a light editor, a diff review, and a clear signal when an agent needs you. It's Rust on
GPUI, macOS only. Read the [README](README.md) for what it does, and
[the design system](docs/design-system.md) and [the Rust stack](docs/rust-stack.md) before
changing how it looks or what it's built on.

These rules are for anyone changing Saturn, people and coding agents alike.

## Before you commit

```sh
pnpm install          # once: the commit hooks and the Markdown/JSON formatter
pnpm health           # format check, clippy, tests, unused dependencies, licences
```

`pnpm health` must pass. The pre-commit hook runs it, and formats staged files first.

- **Conventional Commits**, checked by commitlint: `type(scope): subject`, the subject in lowercase,
  the header at most 100 characters, body lines too. The scope is the area you touched: `app`,
  `editor`, `design`, `grid`, `impact`, `files`, `git`, `pty`, `clickup`.
- One change per commit. A fix found on the way gets its own.
- No secrets, tokens or personal paths in code, tests, docs or screenshots.

## The code

| Crate                    | What belongs there                                                           |
| ------------------------ | ---------------------------------------------------------------------------- |
| `crates/saturn-core`     | Logic without UI: layout, agent states, git, imports, schemas, LSP client    |
| `crates/saturn`          | The app: window, views, editor panel, settings, everything GPUI              |
| `crates/saturn-terminal` | Shells in ptys, alacritty's engine, Claude Code's signals, the terminal view |
| `crates/saturn-design`   | Design tokens, compiled from `design/tokens.json`                            |
| `crates/saturn-bridge`   | Where the Claude Code plugin's hooks reach Saturn                            |

- **Logic goes in `saturn-core`, with tests.** If it can be tested without a window, it lives there.
- **Clippy pedantic is on** and warnings fail the build. No `unwrap()`; an `expect()` says what
  must be true. `unsafe` is denied outside a few AppKit calls that say why.
- **Nothing slow on the main thread.** Git, the file system, processes and language servers run in
  `cx.background_spawn`; the UI updates when they answer.
- **Comments say why**, in plain sentences, on items and on anything surprising. Match the density
  and voice of the code around you. Names are words, not abbreviations.
- **Stay generic.** Saturn is for anyone running agents: no company's workflow, ticket prefix or
  commands built in. Examples use `acme-web`, `ABC-123` and `/start`. Integrations (ClickUp, `gh`)
  are optional and invisible when unused.
- **Learn from other editors, don't copy them.** Zed and other GPL code can be read for ideas,
  never pasted. New dependencies must pass `cargo deny` (see `deny.toml`).
- **`vendor/` holds GPUI with Saturn's patches.** Change it only when there's no other way, mark the
  change "Saturn's patch", and list it in `vendor/README.md`. Its formatting stays upstream's.

## The design

Saturn has a design system, and the code follows it. Full rules in
[docs/design-system.md](docs/design-system.md); the ones that matter most:

- **Colours come from tokens** (`saturn_design::color`, through `theme::color`), never hex in code.
  Strokes and control fills are relative (`theme::overlay`, `separator`, `fill_*`): black in light
  mode, white in dark, at a low alpha.
- **Colour has a job or it isn't there.** Blue: working, focus, the primary action. Orange: needs
  you. Green: your turn. Red: errors, always with words. Everything else is neutral.
- **Glass and cards.** The title bar and the gaps are glass; the sidebar, panes and editor are
  opaque cards (`chrome::panel`, `chrome::card`). Controls that belong together sit in a
  `chrome::track`.
- **Corners nest:** inner radius = outer radius − inset, starting from the macOS window (about 16).
  Cards 10, controls 6, chips 4. Nothing is rounder than what holds it.
- **Density:** controls are 28px tall (`chrome::CONTROL_HEIGHT`), cards 6px apart, padded 4px.
  Weights 400 for text, 500 for controls and names; semibold only in headlines.
- **Icon buttons have a tooltip** saying what they do: `chrome::tip(..)`, or `chrome::tip_for(..,
Action)` when a shortcut does the same, so the shortcut shows. A button that says what it does
  in words (and shows its shortcut) needs none. One tooltip per element (GPUI panics on two).
- **Only what can be clicked reacts to the pointer.** A mark or a label gets no hover.
- **Every feature has a way in you can see**, not only the command palette: a button where it
  applies (the arrow beside New agent, a pane's ⋯, the repo's ⋯, a chip) and its line in the menu
  bar (`set_menus` in `main.rs`). A feature found only by typing its name is a feature nobody finds.
- **Words:** sentence case, no emoji, buttons say what happens ("Add repo", not "OK").
- Both themes, always: check light and dark.

- **Sizes, spacing, corners, tones and icons come from the scales**: `.text_scale(text::LABEL)`,
  `len(space::SPACE_3)`, `theme::corner(radius::RADIUS_SM)`, `theme::tone(tone::HOVER)`, `icon::SM`,
  `size::ROW`. Views use the components in `chrome.rs`
  and never ask which theme is on; a component asks the theme (`chrome::look()`) which version to
  draw. `crates/saturn/tests/design_rules.rs` checks this in `pnpm health`. `pnpm gallery` shows
  every component in the current theme.

`design/tokens.json` is the single source of the tokens, the themes included. The browsable design system (an artifact
linked from the design doc) is kept in step with it.

## Trying a change

Run it with `pnpm dev`. To see it without touching the screen, Saturn's spikes drive it from
environment variables and save what the window draws, offscreen
(see the top of `crates/saturn/src/spike.rs` for the full list):

```sh
cargo run -p saturn --features capture
# with, for example:
SATURN_PROJECT=/path/to/repo SATURN_PANES=2 SATURN_HISTORY=1 \
SATURN_SCREENSHOT=/tmp/shot.png SATURN_SCREENSHOT_AFTER=5 SATURN_QUIT_AFTER=7
```

- `SATURN_USER_DATA=<dir>` keeps a test's settings out of your own.
- Automated runs never bring a window forward or take the keyboard: whoever is at the Mac keeps
  typing into their own app. Don't add anything that activates a window in a spike.
- Never capture the real screen (`screencapture`): it shows whatever else is open. Use
  `SATURN_SCREENSHOT`, which renders Saturn's window alone.
- Language servers: `SATURN_LSP_DEBUG=1` prints every message a server sends;
  `SATURN_LSP_PROBE=line:col` asks the open file's server about a place and prints the answers.

## Docs

A change people can see updates the docs in the same commit: [docs/features.md](docs/features.md),
[shortcuts](docs/shortcuts.md) and [settings](docs/settings.md), and the [README](README.md) when it
changes what Saturn is about. The README stays short: what Saturn is, a few lines per area, install. Screenshots in `docs/images` come from spikes over a made-up project, never
a real one. The README's animation, `docs/images/agents.gif`, is four
such screenshots with `SATURN_FAKE_AGENTS=1` and `SATURN_FAKE_STATES` set to the panes' states, joined
with ffmpeg after the window buttons are drawn on (an offscreen render has none); `docs/images/social-preview.png` is the card GitHub shows when the repo is linked (set in
the repo's settings).

## Building the app

```sh
pnpm install-app      # Saturn.app in /Applications, for this Mac
pnpm dist:dmg         # a disk image for Apple Silicon and Intel, to share
pnpm release 0.2.0    # publish a version: installed copies offer it within a day
```

Releasing needs the Sparkle signing key in the publisher's keychain; updates signed with any other
key are refused by every installed Saturn.

The app is code-signed with the publisher's self-signed "Saturn Code Signing" certificate when
it's in the keychain (`scripts/bundle.sh` finds it), so every version has the same identity. Without it, builds are
signed ad hoc.

Saturn isn't notarised (there's no Apple Developer ID), so a downloaded copy is opened once with
Open Anyway; the disk image explains how.
