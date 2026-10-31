# The Rust stack

Decided 2026-10-18: Akyba is built entirely in native Rust, UI included. Electron and VS Code
extensions are dropped. This document records what was researched (four parallel research tracks,
crates.io and GitHub checked the same day) and what was chosen. It replaces the earlier Electron plan;
the product, the features, the design system and the Claude Code findings all stay.

## Summary

| Need                                | Choice                                                                               | Version                                                                      |
| ----------------------------------- | ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| UI framework                        | **GPUI**, through Longbridge's **gpui-kit**                                          | `gpui-kit` 0.7.3 = `gpui-pre` 0.3.9 + `gpui-component` 0.7.3, pinned exactly |
| Components                          | gpui-component: dock, tabs, tree, editor, diff, lists, theming                       | same                                                                         |
| Terminal engine                     | **alacritty_terminal**, behind our own trait so libghostty-vt can replace it         | 0.26.0                                                                       |
| Claude's signals (OSC 7501, 7, 133) | Our own small parser that reads the stream before alacritty                          | —                                                                            |
| PTY                                 | **portable-pty**; `libproc` for the foreground program and its folder                | 0.9.0                                                                        |
| Terminal view                       | **Our own GPUI element**, following MIT/Apache apps (okena, runner, tty7)            | —                                                                            |
| Editor and syntax colours           | gpui-component `Editor` (tree-sitter 0.26), plus about 20 extra grammars             | `src/languages.rs`                                                           |
| Language servers                    | Our own JSON-RPC client; the editor's completion, hover, definition and action hooks | `lsp-types` 0.97                                                             |
| Diff                                | gpui-component `Diff` (split/unified), from `git diff`                               | —                                                                            |
| Git                                 | the `git` CLI, run off the main thread: status, diffs, staging, the history graph    | —                                                                            |
| Import graph, database schema       | Our own readers: JS/TS, Rust and Python imports; Prisma and SQL schemas              | —                                                                            |
| File watching                       | **notify**, filtered with **ignore**                                                 | 8, 0.4                                                                       |
| Dock badge, notifications, menu bar | a thin `objc2-app-kit` / UserNotifications layer                                     | —                                                                            |
| Design tokens                       | `design/tokens.json` compiled into Rust constants by a build script                  | —                                                                            |

## UI framework: why GPUI

Nothing has replaced GPUI. The newer names around it are snapshots and forks of GPUI itself:

- **`gpui`** on crates.io is frozen at 0.2.2 (October 2025). Zed develops GPUI actively in its own
  repo but doesn't publish it.
- **`gpui-pre`** is a weekly snapshot of Zed's GPUI, published by Longbridge (Jason Lee, the
  gpui-component author). 0.3.9 is zed@93d7456.
- **`gpui-fast`** is Longbridge's experimental retained-mode fork (12 days old). It promises much
  cheaper unchanged frames. It's a one-line feature switch in gpui-kit, worth benchmarking later on
  the terminal grid.
- **`gpui-kit`** bundles a matching `gpui-pre` and gpui-component, so the two never disagree.

Why it wins:

- **It's proven on exactly our workload:** Zed renders many terminals and editors in one window
  with Metal and CoreText.
- **The style API matches the design system one to one:** rounded corners, box shadows (our
  glows), linear gradients at an angle (our 135° gradient), and custom fonts.
- **gpui-kit already covers about 70% of the UI:** a dock with draggable tabs and nested splits, a
  tree, a code editor, a diff view, lists and theming.
- **It's the stack the ecosystem converged on:** more than a dozen open-source GPUI apps for
  running agents exist (below).

The alternatives looked at:

| Framework                                          | Why not                                                                                            |
| -------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Iced 0.14 (+ `iced_term`)                          | The best fallback. No native menus, no accessibility yet, a basic text editor, slow releases       |
| Slint 1.18                                         | No terminal and no code editor widget; the whole main panel would be from scratch                  |
| egui 0.37                                          | Re-lays out every frame, which is costly with 16 live terminals; hard to make look like our design |
| Makepad, Floem, Xilem, Dioxus Native, Freya, Vizia | Early, quiet, or unproven for dense text                                                           |

**The risk is GPUI's API churn:** `gpui-pre` comes out weekly from one maintainer. We pin exact
versions, commit `Cargo.lock`, and upgrade on purpose. Several agent apps fork GPUI for this
reason; we don't need to yet.

## Terminal engine

Two real options:

|                                              | alacritty_terminal 0.26                                  | libghostty-vt (Ghostty's engine)                                                                                  |
| -------------------------------------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Used by                                      | Nearly every GPUI agent app, Zed, Lapce, COSMIC Terminal | A growing few (paneflow, gpui-libghostty)                                                                         |
| OSC 7501                                     | Not understood; we parse it ourselves before alacritty   | **Built in** (added 2026-10-06), including the probe reply                                                        |
| OSC 7 / 133 / unknown sequences              | Dropped; same pre-parse                                  | Callbacks for each                                                                                                |
| Keyboard (kitty protocol) and mouse encoding | We write it (references exist, MIT)                      | Encoders included                                                                                                 |
| Build                                        | Pure Rust                                                | Needs Zig 0.16 at build time                                                                                      |
| API stability                                | Stable, released                                         | "Not yet stable", no tagged releases; the Rust wrapper lacks the 7501 and 133 callbacks, so we'd need our own FFI |
| Threading                                    | `Send`, one engine per pane                              | `!Send`, all panes on one thread                                                                                  |

**Choice: alacritty_terminal.**

- It's the stable, pure-Rust path, and most reference code we can learn from (MIT/Apache) is
  written against it.
- Catching OSC 7501 ourselves is small: we already have the parser from Phase 0.

The engine sits behind a `TerminalEngine` trait, so libghostty-vt can replace it once its API
settles. Its built-in OSC 7501 and input encoders make that attractive.

How the terminal works, following the patterns in the reference apps:

- One reader thread and one writer per pty; we drive alacritty's `Term` ourselves rather than
  using its event loop.
- Wakeups are batched (about 4 ms) into one repaint of the pane that changed.
- DEC 2026 synchronized output is honoured, so Claude's frames never tear.
- Cells with the same style are painted as one text run, backgrounds as separate rectangles, and
  each row's shaped text is cached until the row changes.
- IME pre-edit text is drawn at the cursor (real work: tty7 and runner each have code for it).

## Licences

Akyba stays permissively licensed, so **no GPL code goes in**. That rules out copying:

- **Zed's** `terminal`, `terminal_view`, `editor`, `git_ui` and sidebar crates (GPL-3.0); `gpui`
  itself is Apache-2.0.
- **Warp's client** (AGPL).
- **The GPL agent apps:** paneflow, waku, muxel, codux, COSMIC Terminal.

We can read them for ideas. Code we may adapt, with attribution:

- **MIT:** okena, runner, zeron.
- **Apache-2.0:** tty7, herdr-gpui.
- **MIT or Apache:** gpui-terminal.

`cargo deny` checks every dependency's licence on each health run.

## Prior art worth reading

| Project              | Licence    | What to learn                                                             |
| -------------------- | ---------- | ------------------------------------------------------------------------- |
| contember/okena      | MIT        | Tabs, splits, detachable panes, restoring a workspace, a Claude extension |
| yicheng47/runner     | MIT        | Closest product to Akyba; same `gpui-pre` line; IME code                  |
| l0ng-ai/tty7         | Apache-2.0 | The most complete terminal element (IME, links)                           |
| penso/herdr-gpui     | Apache-2.0 | App polish: dock badge, notifications, command palette, packaging         |
| zortax/gpui-terminal | MIT/Apache | A small, readable terminal element to start from                          |
| maddada/Ghostex      | MIT        | GPUI + libghostty-vt "agents workspace", for when we try Ghostty's engine |

## Editor, diff, files and git

- **Editor:** `EditorState::new(window, cx).language("tsx").line_number(true).searchable(true)`,
  drawn with `Editor::new(&state)`. `⌘F` find and replace are built in, and it holds 200K-line files
  (the authors' claim).
- **Syntax colours:** gpui-component's grammars come as cargo features; `src/languages.rs` registers
  about 20 more (Prisma, SCSS, Swift, XML and others), and a test compiles every highlight query.
  There's no `.env` grammar on crates.io, so `.env*` files are coloured as Bash.
- **Language servers:** `akyba-core::lsp` starts the server a language uses (typescript-language-
  server with the project's own TypeScript, else TypeScript 7's `tsc --lsp`; rust-analyzer;
  pyright; gopls) and speaks JSON-RPC over its stdio. The editor component draws completions,
  hover, diagnostics and the code-action menu; Akyba answers them from the server, ranks
  completions itself, and applies rename and quick-fix edits across files.
- **tree-sitter version:** Akyba must not depend on tree-sitter 0.27, because gpui-component uses
  0.26 and Cargo allows only one.
- **Diff:** gpui-component's diff view is read-only; Akyba feeds it `git diff` for the uncommitted
  work, the branch since it left the default branch, or one commit.
- **Tree:** gpui-component's `Tree` is virtualized, with rows drawn by us (git badges). Folders
  load when opened, walked one level at a time with `ignore::WalkBuilder`.
- **Watching:**
  - One recursive `notify` watch per worktree root, registered once, because adding watches one by
    one is slow with FSEvents.
  - Paths git ignores (`node_modules`, `target`) are filtered out afterwards, with `ignore`.
  - A burst is merged into one update: the first event waits a moment for the rest.
- **Git:** the `git` CLI, always off the main thread: status, diffs, staging and `git log --all` for
  the history graph, whose lanes `akyba-core::git_graph` lays out.

## Build and tooling

- **Workspace layout:**
  - `crates/akyba` (the app)
  - `akyba-core` (layout, agent state, title rules: pure and unit-tested; ported from the
    TypeScript versions and their tests)
  - `akyba-terminal` (pty, engine, OSC parser, element)
  - `akyba-design` (tokens)
- **Checks, the usual setup in Rust form:**
  - `cargo fmt`
  - `cargo clippy` with warnings as errors (lint)
  - `cargo test`
  - `cargo machete` (dead dependencies, like knip)
  - `cargo deny` (licences and advisories)
- **Commits and docs:** conventional commits stay enforced by commitlint and husky, and oxfmt
  formats Markdown and JSON. That's dev-only Node tooling; nothing of it ships in the app.
- **Build cost:** the first build of GPUI takes 5–15 minutes and a few GB of disk; after that,
  builds are incremental.

## Spike results (2026-10-18)

Measured with `AKYBA_PANES=12 AKYBA_COMMAND="node scripts/tui-load.mjs 30 14" AKYBA_PERF=1`,
in a release build: every pane redraws a full screen of coloured text 30 times a second, inside
synchronized-output brackets. These are the same conditions as the earlier Electron spike.

|                                | Electron (Phase 0) | Native Rust       |
| ------------------------------ | ------------------ | ----------------- |
| Memory, 12 busy panes          | ~550 MB            | **~117 MB**       |
| CPU                            | not measured       | ~31% of one core  |
| Laying out all terminal frames | —                  | ~52 ms per second |

- **Every terminal is laid out on every frame.** GPUI redraws the whole window at 120 Hz, about
  1,300 terminal layouts a second. That is roughly 4 ms of each 8 ms frame.
- **Room left:** caching shaped rows until they change, or switching on `gpui-fast`'s retained
  mode.
- **Rendering checked:** colours, bold, Czech diacritics, emoji, the block cursor and the focus glow
  all render correctly (screenshot taken with `AKYBA_SCREENSHOT`, `capture` feature).
- **One fix it found:** a shell started from the Dock has no `LANG`, and zsh then shows typed
  non-ASCII as `<0001fa90>`. Akyba sets `LANG=en_US.UTF-8` when it's missing, as Terminal.app does.

## What happens to the Electron code

The Electron app (Phases 0–1) was removed on 2026-10-18 and stays in git history. Its parts carry over as follows:

- **Behaviour:** the grid model, agent states, title rules, the OSC 7501 parser, the shell-env
  fix, the quit-crash lesson (wait for shells before exit) and the end-to-end scenarios.
- **Ported to Rust with their tests:** the grid model, agent states, title rules and the OSC 7501
  parser.
- **Removed in one commit:** the Electron code itself, once the Rust grid reaches the same point.

## Sources

- crates.io API (versions, dates, owners, licences) for every crate named above, checked
  2026-10-18
- https://github.com/longbridge/gpui-kit · https://github.com/longbridge/gpui-fast ·
  https://crates.io/crates/gpui-pre
- https://github.com/zed-industries/zed (gpui README, crate licences)
- https://github.com/alacritty/alacritty · https://github.com/alacritty/vte
- https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/terminal.h ·
  https://github.com/Uzaaft/libghostty-rs · https://mitchellh.com/writing/libghostty-is-coming
- https://github.com/wezterm/wezterm (portable-pty)
- https://github.com/iced-rs/iced/releases/tag/0.14.0 · https://slint.dev/pricing
- https://github.com/notify-rs/notify · https://microsoft.github.io/language-server-protocol/
- Prior art: contember/okena, yicheng47/runner, l0ng-ai/tty7, penso/herdr-gpui,
  zortax/gpui-terminal, maddada/Ghostex, arthjean/paneflow,
  https://dev.to/arthurj-dev/building-a-native-terminal-for-ai-coding-agents-in-rust-gpui-2bg4
