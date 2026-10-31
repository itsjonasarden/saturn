# Develop

## Build from source

You need [rustup](https://rustup.rs) and [pnpm](https://pnpm.io) as well.

```sh
git clone https://github.com/itsjonasarden/akyba.git
cd akyba
pnpm install
pnpm install-app      # builds Akyba.app and puts it in /Applications
```

Rust's version is pinned in `rust-toolchain.toml`, and rustup installs it on the first build.
`pnpm dist:dmg` builds a disk image for Apple Silicon and Intel to share, and
`pnpm release 0.2.0` publishes a version: it tags it, builds the disk image, signs it with the
Sparkle key in your keychain, and puts it on GitHub with the feed.

```sh
pnpm demo             # Akyba on a made-up project, to look around or show it off
pnpm dev              # build, sign and run Akyba from the code
pnpm health           # format check, clippy, tests, unused deps, licences
pnpm dist             # build Akyba.app in target/bundle.noindex
```

Node is only used for commit hooks and for formatting Markdown and JSON. Commits follow
[Conventional Commits](https://www.conventionalcommits.org).

| Crate                   | What it holds                                                                |
| ----------------------- | ---------------------------------------------------------------------------- |
| `crates/akyba`          | The app: window, title bar, grid, sidebar, editor, views, settings           |
| `crates/akyba-terminal` | Shells in ptys, alacritty's engine, Claude Code's signals, the terminal view |
| `crates/akyba-core`     | Logic without UI: layout, agents, git, imports, schemas, the LSP client      |
| `crates/akyba-design`   | Design tokens compiled from [`design/tokens.json`](../design/tokens.json)    |
| `crates/akyba-bridge`   | Where the plugin's hooks reach Akyba: a small HTTP server on a Unix socket   |

More reading: [the design system](design-system.md) and [the Rust stack](rust-stack.md).
Want to change something? [AGENTS.md](../AGENTS.md) has the rules, for people and coding agents alike.

## Spikes and screenshots

`AKYBA_PANES=12 AKYBA_COMMAND="node scripts/tui-load.mjs 30 14" AKYBA_PERF=1 cargo run -p akyba --release`
opens a busy grid and prints frame timings. With `--features capture`, `AKYBA_SCREENSHOT=out.png`
saves what the window draws.
