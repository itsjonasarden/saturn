//! A harness for performance spikes and screenshots, driven by environment variables:
//!
//! - `SATURN_PANES=N` opens N panes instead of one shell (0: the empty grid), `SATURN_COMMAND`
//!   types a command into each.
//! - `SATURN_PERF=1` prints how long terminal frames take, every second.
//! - `SATURN_OPEN_SETTINGS=1` opens the Settings dialog on start; `SATURN_OPEN_FILE=path` opens a file
//!   in the editor (`SATURN_SAVE=1` saves it after a moment); `SATURN_REVIEW=1` shows the Changes tab and reviews the first change
//!   (`SATURN_REVIEW_COMMENT="12:text"` comments on its line 12; `SATURN_REVIEW_SEND=1` sends it;
//!   `SATURN_REVIEW_PICK=12` clicks line 12, `SATURN_REVIEW_ACT=stage|revert|undo` presses that);
//!   `SATURN_COMPARE=main` reviews everything that differs from `main`.
//! - `SATURN_MORE_REPOS=a:b` adds more repos to the window, as ⌘O would (`SATURN_SHOW_REPO=a`
//!   clicks that repo's tab after 3 seconds); `SATURN_FAKE_AGENTS=1`
//!   gives the panes made-up agents and pull requests (`SATURN_FAKE_AGENTS_AFTER=3` after 3 seconds) (`SATURN_FAKE_STATES=working,needs-you,your-turn`
//!   sets their states in order); `SATURN_HISTORY=1` opens the History tab, `SATURN_IMPACT=1` the Impact tab (focused on `SATURN_IMPACT_FOCUS`), `SATURN_SCHEMA=1`
//!   the database diagram, `SATURN_LINEAGE=1` the dbt lineage graph; `SATURN_LSP_PROBE=line:col` asks the open file's language server about
//!   that place (0-based) and prints its answers; `SATURN_PROBLEMS=1` lists the
//!   servers' errors and warnings; `SATURN_PALETTE=files|commands` opens a palette, with
//!   `SATURN_PALETTE_QUERY` typed in it; `SATURN_TICKET_PICKER=1` opens New agent (with
//!   `SATURN_NEW_AGENT_QUERY` typed in it);
//!   `SATURN_TICKET_FILTERS=1` opens the Tickets panel's filters.
//! - `SATURN_DROP=path` drops that file on the middle of the window, as Finder would (with
//!   `SATURN_DROP_HOLD=1`, holds it there without letting go, to see what lights up);
//!   `SATURN_DRAG=270,600:390,600` drags the mouse from one point to the other, a few pixels a frame.
//! - `SATURN_RUN_COMMAND="Start the app"` runs that command from the palette's list, by its name
//!   (after `SATURN_RUN_COMMAND_AFTER` seconds, else 1.5); `SATURN_THEN_COMMAND` runs a second one
//!   later (`SATURN_THEN_AFTER`, else 8).
//! - `SATURN_BROADCAST=text` writes that in Send to several agents (`SATURN_BROADCAST_SEND=1` sends it
//!   two seconds after it opens).
//! - `SATURN_BEST_OF=text` writes that in Best of N (`SATURN_BEST_OF_START=1` starts it,
//!   `SATURN_BEST_OF_AGENTS=claude,codex` sets each attempt's agent); with Compare attempts open,
//!   `SATURN_KEEP_ATTEMPT=2` keeps attempt 2. `SATURN_AGENTS=claude,codex` says which agents the Mac
//!   has, without looking.
//! - `SATURN_SESSIONS_QUERY=text` types that into Search sessions (`CLAUDE_CONFIG_DIR` points it at
//!   made-up transcripts).
//! - `SATURN_ROUTINES=1` lets routines run in an automated run (they never do otherwise).
//! - `SATURN_TEAM=1` opens the first pane's agent team after 6 seconds.
//! - `SATURN_START_NAMED=fix-login` starts work by name, also in the other repos `SATURN_ALSO_IN` names.
//! - `SATURN_UNDO_TURN=6` presses the first pane's "Undo the agent's last turn" after 6 seconds.
//! - `SATURN_ANSWER=allow|deny` answers the first pane's permission after 5 seconds
//!   (`SATURN_ANSWER_AFTER` seconds when given).
//! - `SATURN_RUN_CHECKS=1` runs every check of the focused pane's checkout.
//! - `SATURN_PICTURES=1` opens "Pictures of the app" after 6 seconds (`SATURN_TAKE_PICTURES=1` takes a set).
//! - `SATURN_KEYS="e c h tab"` presses those keys in the focused pane, through the window, and prints
//!   the tab count after each; `SATURN_REBIND=new-tab=cmd-t` moves a shortcut first, as Settings
//!   would. `SATURN_SETTINGS_SECTION=keyboard` opens Settings on that section.
//! - `SATURN_USAGE_LIVE=1` reads the plans' real usage, as Saturn does when no spike runs it.
//! - `SATURN_MAXIMIZE_PANE=1` maximises the focused pane, over the grid and the panels.
//! - `SATURN_MENU=new-agent|pane|repo` opens that button's menu after 2 seconds (`SATURN_MENU_PICK=2`
//!   picks its third line instead).
//! - `SATURN_SCREENSHOT=path.png` saves what the window draws after `SATURN_SCREENSHOT_AFTER`
//!   seconds (needs the `capture` feature); `SATURN_QUIT_AFTER` quits after that many seconds.

use std::time::Duration;

use gpui_kit::{Context, Window};
use saturn_core::layout::TabId;

use crate::workspace::{Placement, Workspace};

fn var<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|value| value.parse().ok())
}

/// Whether this run is driven by a script (screenshot or timed quit) rather than a person.
pub fn is_automated() -> bool {
    std::env::var_os("SATURN_SCREENSHOT").is_some() || std::env::var_os("SATURN_QUIT_AFTER").is_some()
}

/// How many panes to open on start, when a spike asks for a number.
pub fn panes() -> Option<usize> {
    var("SATURN_PANES")
}

/// Lays N panes out like the Electron spike: one row up to 3, two up to 8, three beyond.
pub fn placement(index: usize, panes: usize) -> Placement {
    let rows = match panes {
        0..=3 => 1,
        4..=8 => 2,
        _ => 3,
    };
    let per_row = panes.div_ceil(rows);
    match index {
        0 => Placement::New,
        i if i % per_row == 0 => Placement::Down,
        _ => Placement::Right,
    }
}

/// Clicks the tab of the repo `SATURN_SHOW_REPO` names, later than the panes' made-up states, so
/// the repo left behind shows what waits in it.
fn show_repo(window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(path) = std::env::var_os("SATURN_SHOW_REPO").map(std::path::PathBuf::from) else { return };
    cx.spawn_in(window, async move |workspace, cx| {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let _ = workspace.update_in(cx, |workspace, window, cx| workspace.select_project(path, window, cx));
    })
    .detach();
}

fn report_perf() {
    let (frames, spent) = saturn_terminal::element::stats::take();
    eprintln!("perf: {frames} terminal frames, {:.1} ms laying them out", spent.as_secs_f64() * 1000.);
}

/// The made-up agents, at once or (`SATURN_FAKE_AGENTS_AFTER`) once the shells have started, so a
/// perf run measures the panes' states apart from their start. The window stays in the background,
/// as in every automated run.
fn start_fake_agents(window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(seconds) = var::<u64>("SATURN_FAKE_AGENTS_AFTER") {
        cx.spawn_in(window, async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_secs(seconds)).await;
            let _ = workspace.update_in(cx, |workspace, _, cx| workspace.fake_agents(cx));
        })
        .detach();
    } else if std::env::var_os("SATURN_FAKE_AGENTS").is_some() {
        cx.defer_in(window, |workspace, _, cx| workspace.fake_agents(cx));
    }
}

pub fn start(workspace: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Ok(command) = std::env::var("SATURN_COMMAND") {
        let tabs: Vec<TabId> = (1..=panes().unwrap_or(0) as u64).map(TabId).collect();
        let views: Vec<_> = tabs.iter().filter_map(|tab| workspace.terminal(*tab).cloned()).collect();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(1500)).await;
            for view in views {
                let () = view.update(cx, |view, _| view.write(format!("{command}\r").into_bytes()));
            }
        })
        .detach();
    }
    if std::env::var("SATURN_PERF").is_ok() {
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                report_perf();
            }
        })
        .detach();
    }
    if std::env::var_os("SATURN_OPEN_SETTINGS").is_some() {
        // After the window (and the Root that hosts dialogs) exists.
        cx.defer_in(window, Workspace::open_settings);
    }
    if std::env::var_os("SATURN_LINEAGE").is_some() {
        open_lineage(cx);
    }
    if std::env::var_os("SATURN_SCHEMA").is_some() {
        cx.defer_in(window, |workspace, _, cx| workspace.show_schema(cx));
    }
    if std::env::var_os("SATURN_IMPACT").is_some() {
        cx.defer_in(window, |workspace, _, cx| {
            let root = workspace.root().to_owned();
            workspace.show_impact(&root, cx);
        });
    }
    if std::env::var_os("SATURN_HISTORY").is_some() {
        cx.defer_in(window, |workspace, _, cx| workspace.show_history(cx));
    }
    if std::env::var_os("SATURN_FAKE_TICKETS").is_some() {
        cx.defer_in(window, Workspace::fake_tickets);
    }
    start_fake_agents(window, cx);
    review_from_env(window, cx);
    if std::env::var_os("SATURN_SAVE").is_some() {
        // After SATURN_OPEN_FILE has opened its file.
        cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_millis(1500)).await;
            let _ = workspace.update(cx, Workspace::save_file);
        })
        .detach();
    }
    if let Some(paths) = std::env::var_os("SATURN_MORE_REPOS") {
        // Repos added after the first, `:`-separated, as ⌘O would add them.
        for path in std::env::split_paths(&paths) {
            cx.defer_in(window, move |workspace, window, cx| workspace.add_project(path, window, cx));
        }
        // The first repo stays selected, as if the others were added earlier.
        let first = workspace.project_path().to_owned();
        cx.defer_in(window, move |workspace, window, cx| workspace.select_project(first, window, cx));
    }
    show_repo(window, cx);
    if let Some((line, character)) =
        std::env::var("SATURN_LSP_PROBE").ok().and_then(|at| at.split_once(':').map(|(l, c)| (l.parse(), c.parse())))
        && let (Ok(line), Ok(character)) = (line, character)
    {
        // After the opened file's server has had a moment to start.
        cx.spawn_in(window, async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ =
                workspace.update_in(cx, |workspace, window, cx| workspace.probe_language(line, character, window, cx));
        })
        .detach();
    }
    if let Ok(which) = std::env::var("SATURN_PALETTE") {
        open_palette(which, window, cx);
    }
    rebind_from_env(workspace, cx);
    input_from_env(window, cx);
    run_command_from_env(window, cx);
    team_from_env(window, cx);
    start_named_from_env(window, cx);
    press_from_env(cx);
    if std::env::var_os("SATURN_TICKET_PICKER").is_some() {
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(800)).await;
            let _ = cx.update(|window, cx| window.dispatch_action(Box::new(crate::workspace::OpenTicketPicker), cx));
        })
        .detach();
    }
    maximize_from_env(window, cx);
    if std::env::var_os("SATURN_PROBLEMS").is_some() {
        // Once the opened file's server has reported.
        cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            let _ = workspace.update(cx, Workspace::show_problems);
        })
        .detach();
    }
    if let Some(path) = std::env::var_os("SATURN_OPEN_FILE") {
        cx.defer_in(window, move |workspace, window, cx| workspace.open_file(std::path::Path::new(&path), window, cx));
    }
}

/// `SATURN_REBIND=new-tab=cmd-t` moves a shortcut as Settings would, before any keys are pressed.
fn rebind_from_env(workspace: &Workspace, cx: &mut Context<Workspace>) {
    if let Some((id, keys)) = std::env::var("SATURN_REBIND")
        .ok()
        .and_then(|value| value.split_once('=').map(|(id, keys)| (id.to_owned(), keys.to_owned())))
        && let Some(shortcut) = crate::keymap::SHORTCUTS.iter().find(|shortcut| shortcut.id == id)
    {
        let before = crate::keymap::keys_of(shortcut, &workspace.settings().keymap);
        let mut after = workspace.settings().keymap.clone();
        after.insert(id, keys);
        crate::keymap::rebind(shortcut, &before, &after, cx);
    }
}

/// `SATURN_MAXIMIZE_PANE`: the focused pane maximised; `SATURN_MAXIMIZE_EDITOR`: the editor in the grid's room.
fn maximize_from_env(window: &mut Window, cx: &mut Context<Workspace>) {
    if std::env::var_os("SATURN_MAXIMIZE_PANE").is_some() {
        cx.defer_in(window, |_, window, cx| window.dispatch_action(Box::new(crate::workspace::ToggleMaximize), cx));
    }
    if std::env::var_os("SATURN_MAXIMIZE_EDITOR").is_some() {
        cx.defer_in(window, |workspace, _, cx| workspace.maximize_editor(cx));
    }
    if let Ok(name) = std::env::var("SATURN_MENU") {
        let pick = var("SATURN_MENU_PICK");
        cx.spawn_in(window, async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = workspace.update_in(cx, |workspace, window, cx| workspace.spike_menu(&name, pick, window, cx));
        })
        .detach();
    }
}

/// Opens the lineage graph once the folder's walk has found the dbt project.
fn open_lineage(cx: &mut Context<Workspace>) {
    cx.spawn(async move |workspace, cx| {
        cx.background_executor().timer(Duration::from_millis(1500)).await;
        let _ = workspace.update(cx, Workspace::show_lineage);
    })
    .detach();
}

/// Reviews the first change (`SATURN_REVIEW`), and comments on it and sends the comment to the agent
/// (`SATURN_REVIEW_COMMENT`, `SATURN_REVIEW_SEND`).
fn review_from_env(window: &mut Window, cx: &mut Context<Workspace>) {
    if let Ok(target) = std::env::var("SATURN_COMPARE") {
        cx.defer_in(window, move |workspace, _, cx| {
            let repo = workspace.root().to_owned();
            workspace.review_against(&repo, target, cx);
        });
    }
    if std::env::var_os("SATURN_REVIEW").is_none() {
        return;
    }
    cx.defer_in(window, |workspace, _, cx| workspace.review_first_change(cx));
    if let Ok(line) = std::env::var("SATURN_REVIEW_PICK") {
        let press = std::env::var("SATURN_REVIEW_ACT").ok();
        cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_millis(3000)).await;
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.editor().update(cx, |editor, cx| editor.pick_in_review(line.parse().ok(), None, cx));
            });
            // Once the pick has read the diff.
            cx.background_executor().timer(Duration::from_millis(1000)).await;
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.editor().update(cx, |editor, cx| editor.pick_in_review(None, press.as_deref(), cx));
            });
        })
        .detach();
    }
    if let Ok(comment) = std::env::var("SATURN_REVIEW_COMMENT") {
        let send = std::env::var_os("SATURN_REVIEW_SEND").is_some();
        cx.spawn(async move |workspace, cx| {
            // After the review has opened (it opens 1.5 s in).
            cx.background_executor().timer(Duration::from_millis(3000)).await;
            let (line, text) = comment.split_once(':').unwrap_or(("1", comment.as_str()));
            let line = line.parse().unwrap_or(1);
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.editor().update(cx, |editor, cx| editor.comment_on_review(line, text, send, cx));
            });
        })
        .detach();
    }
}

/// Buttons a spike presses after a while: undo a turn, answer a permission, run the checks.
fn press_from_env(cx: &mut Context<Workspace>) {
    if std::env::var_os("SATURN_PICTURES").is_some() {
        // Once the pane's server has been seen; `SATURN_TAKE_PICTURES` presses Take too.
        cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            cx.update(|cx| {
                let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else { return };
                let _ = window.update(cx, |_, window, cx| {
                    let _ = workspace.update(cx, |workspace, cx| workspace.show_pictures(window, cx));
                });
            });
        })
        .detach();
    }
    undo_turn_from_env(cx);
    answer_from_env(cx);

    if std::env::var_os("SATURN_RUN_CHECKS").is_some() {
        // Every check of the focused pane's checkout, as picking "All of them" would.
        cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = workspace.update(cx, Workspace::run_all_checks);
        })
        .detach();
    }
}

/// Presses the first pane's "Undo the agent's last turn" after that many seconds (`SATURN_UNDO_TURN=6`).
fn undo_turn_from_env(cx: &mut Context<Workspace>) {
    let Ok(after) = std::env::var("SATURN_UNDO_TURN") else { return };
    let after = after.parse().unwrap_or(6);
    cx.spawn(async move |workspace, cx| {
        cx.background_executor().timer(Duration::from_secs(after)).await;
        let _ = workspace.update(cx, |workspace, cx| workspace.undo_turn(TabId(1), cx));
    })
    .detach();
}

/// Starts work by name in other repos too, as New agent would (`SATURN_START_NAMED=fix-login`,
/// `SATURN_ALSO_IN=acme-api` naming the window's other repos to pick, comma separated).
fn start_named_from_env(window: &mut Window, cx: &mut Context<Workspace>) {
    let Ok(name) = std::env::var("SATURN_START_NAMED") else { return };
    let also: Vec<String> = std::env::var("SATURN_ALSO_IN").unwrap_or_default().split(',').map(str::to_owned).collect();
    cx.spawn_in(window, async move |workspace, cx| {
        cx.background_executor().timer(Duration::from_secs(2)).await;
        let _ = workspace.update_in(cx, |workspace, window, cx| {
            for repo in workspace.other_repos() {
                if repo.file_name().is_some_and(|folder| also.iter().any(|name| folder == name.as_str())) {
                    workspace.toggle_also_in(&repo, cx);
                }
            }
            let agent = crate::agent_choice::current(cx);
            workspace.start_named(&name, agent, window, cx);
        });
    })
    .detach();
}

/// Opens the first pane's agent team after 6 seconds (`SATURN_TEAM=1`).
fn team_from_env(window: &mut Window, cx: &mut Context<Workspace>) {
    if std::env::var_os("SATURN_TEAM").is_none() {
        return;
    }
    cx.spawn_in(window, async move |workspace, cx| {
        cx.background_executor().timer(Duration::from_secs(6)).await;
        let _ = workspace.update_in(cx, |workspace, window, cx| {
            let title = workspace.team(TabId(1)).map(|team| team.name.clone());
            match title {
                Some(title) => crate::team_view::open(cx.weak_entity(), TabId(1), title, window, cx),
                None => eprintln!("SATURN_TEAM: the first pane leads no team"),
            }
        });
    })
    .detach();
}

/// Answers the first pane's permission after 5 seconds (`SATURN_ANSWER=allow|deny`, after
/// `SATURN_ANSWER_AFTER` seconds instead when given).
fn answer_from_env(cx: &mut Context<Workspace>) {
    let Ok(answer) = std::env::var("SATURN_ANSWER") else { return };
    let after = var("SATURN_ANSWER_AFTER").map_or(Duration::from_secs(5), Duration::from_secs);
    cx.spawn(async move |workspace, cx| {
        cx.background_executor().timer(after).await;
        // Then once a second until there's a permission to answer, for an agent that takes its time.
        for _ in 0..180 {
            let answered = workspace
                .update(cx, |workspace, cx| workspace.answer_permission(TabId(1), answer == "allow", cx))
                .unwrap_or(true);
            if answered {
                eprintln!("answered the permission: {answer}");
                break;
            }
            cx.background_executor().timer(Duration::from_secs(1)).await;
        }
    })
    .detach();
}

/// Runs a palette command by its name (`SATURN_RUN_COMMAND`), as picking it would, and a second one
/// later (`SATURN_THEN_COMMAND`, after `SATURN_THEN_AFTER` seconds, else 8).
fn run_command_from_env(window: &mut Window, cx: &mut Context<Workspace>) {
    if let Ok(name) = std::env::var("SATURN_THEN_COMMAND") {
        let after = var("SATURN_THEN_AFTER").map_or(Duration::from_secs(8), Duration::from_secs);
        run_command_after(name, after, window, cx);
    }
    let Ok(name) = std::env::var("SATURN_RUN_COMMAND") else { return };
    // Later for commands about a pane's server, which takes a few seconds to be seen.
    let after = var("SATURN_RUN_COMMAND_AFTER").map_or(Duration::from_millis(1500), Duration::from_secs);
    run_command_after(name, after, window, cx);
}

fn run_command_after(name: String, after: Duration, window: &mut Window, cx: &mut Context<Workspace>) {
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(after).await;
        let _ = cx.update(|window, cx| {
            let command = crate::workspace::commands().into_iter().find(|(label, _, _)| *label == name);
            match command {
                Some((_, _, action)) => window.dispatch_action(action, cx),
                None => eprintln!("SATURN_RUN_COMMAND: no command named {name:?}"),
            }
        });
    })
    .detach();
}

/// Input as a person would give it: a dropped file (`SATURN_DROP`), keys (`SATURN_KEYS`).
fn input_from_env(window: &mut Window, cx: &mut Context<Workspace>) {
    if let Ok(path) = std::env::var("SATURN_DROP") {
        drop_file(path, window, cx);
    }
    if let Ok(keys) = std::env::var("SATURN_KEYS") {
        press_keys(keys, window, cx);
    }
    if let Ok(drag) = std::env::var("SATURN_DRAG") {
        drag_mouse(&drag, window, cx);
    }
}

/// Presses the mouse at one point and drags it to another a few pixels a frame, as a hand would
/// (`SATURN_DRAG=270,400:420,400`): how dividers and resize handles are tried.
fn drag_mouse(drag: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let point = |text: &str| {
        let (x, y) = text.split_once(',')?;
        Some(gpui_kit::point(gpui_kit::px(x.trim().parse().ok()?), gpui_kit::px(y.trim().parse().ok()?)))
    };
    let Some((from, to)) = drag.split_once(':').and_then(|(from, to)| Some((point(from)?, point(to)?))) else {
        return;
    };
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(Duration::from_millis(2000)).await;
        let left = gpui_kit::MouseButton::Left;
        let _ = cx.update(|window, cx| {
            let down = gpui_kit::MouseDownEvent {
                button: left,
                position: from,
                modifiers: gpui_kit::Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            };
            window.dispatch_event(gpui_kit::PlatformInput::MouseDown(down), cx);
        });
        let steps = 40;
        for step in 1..=steps {
            cx.background_executor().timer(Duration::from_millis(16)).await;
            #[allow(clippy::cast_precision_loss)]
            let along = step as f32 / steps as f32;
            let position = from + (to - from) * along;
            let _ = cx.update(|window, cx| {
                let event = gpui_kit::MouseMoveEvent {
                    position,
                    pressed_button: Some(left),
                    modifiers: gpui_kit::Modifiers::default(),
                };
                window.dispatch_event(gpui_kit::PlatformInput::MouseMove(event), cx);
            });
        }
        let _ = cx.update(|window, cx| {
            let up = gpui_kit::MouseUpEvent {
                button: left,
                position: to,
                modifiers: gpui_kit::Modifiers::default(),
                click_count: 1,
            };
            window.dispatch_event(gpui_kit::PlatformInput::MouseUp(up), cx);
        });
    })
    .detach();
}

/// Drops a file on the middle of the window, as Finder would (`SATURN_DROP=path`).
fn drop_file(path: String, window: &mut Window, cx: &mut Context<Workspace>) {
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(Duration::from_millis(2000)).await;
        let _ = cx.update(|window, cx| {
            let middle = window.bounds().size.center();
            let paths = gpui_kit::ExternalPaths(vec![std::path::PathBuf::from(path)].into());
            let hold = std::env::var_os("SATURN_DROP_HOLD").is_some();
            let events = [
                gpui_kit::FileDropEvent::Entered { position: middle, paths },
                gpui_kit::FileDropEvent::Pending { position: middle },
                gpui_kit::FileDropEvent::Submit { position: middle },
            ];
            for event in events.into_iter().take(if hold { 2 } else { 3 }) {
                window.dispatch_event(gpui_kit::PlatformInput::FileDrop(event), cx);
            }
        });
    })
    .detach();
}

/// Presses keys in the focused pane as a person would, through the window (`SATURN_KEYS`,
/// keystrokes separated by spaces: `e c h tab`), to see what reaches the shell.
fn press_keys(keys: String, window: &mut Window, cx: &mut Context<Workspace>) {
    cx.spawn_in(window, async move |workspace, cx| {
        cx.background_executor().timer(Duration::from_millis(2000)).await;
        for key in keys.split_whitespace() {
            let _ = cx.update(|window, cx| {
                if let Ok(keystroke) = gpui_kit::Keystroke::parse(key) {
                    let handled = window.dispatch_keystroke(keystroke, cx);
                    eprintln!("key {key}: handled = {handled}");
                }
            });
            cx.background_executor().timer(Duration::from_millis(100)).await;
            let _ = workspace.update(cx, |workspace, _| eprintln!("tabs: {}", workspace.tab_count()));
        }
    })
    .detach();
}

/// Opens a palette (`files` or `commands`) once the window is up, and reports whether its search
/// has the keyboard.
fn open_palette(which: String, window: &mut Window, cx: &mut Context<Workspace>) {
    // After the window and its dialog layer exist: `files` (⌘P) or `commands` (⌘⇧P).
    cx.spawn_in(window, async move |workspace, cx| {
        cx.background_executor().timer(Duration::from_millis(800)).await;
        let _ = workspace.update_in(cx, |workspace, window, cx| {
            if which == "files" {
                crate::palette::go_to_file(cx.weak_entity(), workspace.root().to_owned(), window, cx);
            } else {
                crate::palette::commands(window, cx);
            }
        });
        cx.background_executor().timer(Duration::from_millis(600)).await;
        let _ = workspace.update_in(cx, |_, window, cx| {
            let palette =
                crate::palette::LAST.with_borrow(|last| last.as_ref().and_then(gpui_kit::WeakEntity::upgrade));
            if let Some(palette) = palette {
                eprintln!("palette: search focused = {}", palette.read(cx).search_has_focus(window, cx));
            }
        });
    })
    .detach();
}

/// The screenshot and the timed quit, for the window itself: they apply on the welcome screen too.
pub fn start_window<T: 'static>(window: &mut Window, cx: &mut Context<T>) {
    capture(window, cx);
    if let Some(seconds) = var::<u64>("SATURN_QUIT_AFTER") {
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_secs(seconds)).await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    }
}

#[cfg(feature = "capture")]
fn capture<T: 'static>(window: &mut Window, cx: &mut Context<T>) {
    let Ok(path) = std::env::var("SATURN_SCREENSHOT") else { return };
    let after = var::<u64>("SATURN_SCREENSHOT_AFTER").unwrap_or(3);
    let handle = window.window_handle();
    cx.spawn(async move |_, cx| {
        cx.background_executor().timer(Duration::from_secs(after)).await;
        // macOS stops sending frames to a window it considers hidden, so draw one now: what
        // arrived late (a shell's output, a server's answer) is then in the picture.
        let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
        // Twice: a view that sizes itself to the first frame (a diagram fitting the view) settles
        // on the second, as it would on screen.
        let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
        let _ = handle.update(cx, |_, window, _| match window.render_to_image() {
            Ok(image) => match image.save(&path) {
                Ok(()) => eprintln!("screenshot: {path}"),
                Err(error) => eprintln!("screenshot failed: {error}"),
            },
            Err(error) => eprintln!("screenshot failed: {error}"),
        });
    })
    .detach();
}

#[cfg(not(feature = "capture"))]
fn capture<T: 'static>(_: &mut Window, _: &mut Context<T>) {}
