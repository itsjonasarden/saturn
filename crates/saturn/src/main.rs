//! Saturn: a terminal grid for running many Claude Code agents side by side.

mod agent_choice;
mod agents_view;
mod app_view;
mod assets;
mod best_of_view;
mod broadcast_view;
mod changes_view;
mod chrome;
mod clickup;
mod context_view;
mod dock;
mod editor_view;
mod files_view;
mod github;
mod history_view;
mod impact_view;
mod keymap;
mod languages;
mod lineage_view;
mod lsp;
mod menu;
mod menu_bar;
mod notify;
mod palette;
mod pictures_view;
mod pr_body_view;
mod preview_view;
mod problems_view;
mod queue_view;
mod references_view;
mod review_comments;
mod routines_view;
mod schema_view;
mod session;
mod sessions_view;
mod settings;
mod settings_view;
mod spike;
mod status_bar;
mod status_line;
mod teach_view;
mod team_view;
mod theme;
mod ticket_picker;
mod tickets_view;
mod updates;
mod usage;
mod workspace;
mod worktrees_view;

use std::borrow::Cow;

use gpui_kit::{
    App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, actions, point, px, size,
};

use crate::app_view::AppView;
use crate::files_view as saturn_files;
use crate::workspace::{
    BestOfN, CloseTab, CommandPalette, CompareAttempts, CompareWith, Copy, CreateRecipe, EditPrBody, EvenOutPanes,
    GoToFile, NewTab, NextAttention, NextTab, OpenProject, OpenSettings, OpenTicketPicker, Paste, PreviousTab,
    RunCheck, SaveFile, SearchSessions, SendToSeveral, SetUpWorktree, ShowContext, ShowHistory, ShowImpact,
    ShowLineage, ShowPictures, ShowPreview, ShowProblems, ShowRoutines, ShowSchema, ShowWaiting, ShowWorktrees,
    SplitDown, SplitRight, StartApp, TeachRepo, TextBigger, TextReset, TextSmaller, ToggleEditor, ToggleMaximize,
    ToggleSidebar, ToggleTickets,
};

actions!(saturn, [Quit, CheckForUpdates]);

/// The login shell's `PATH`, read once at start, for the tools Saturn runs itself.
pub static LOGIN_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// One static file per weight the design system uses: GPUI draws a variable font only at its default
/// weight. Instanced from the variable fonts with fontTools (OFL 1.1; neither declares a Reserved
/// Font Name). Rajdhani is Cyberdeck's interface font, as Google Fonts ships it (OFL 1.1, no Reserved
/// Font Name).
const FONTS: [&[u8]; 9] = [
    include_bytes!("../assets/fonts/Urbanist-Light.ttf"),
    include_bytes!("../assets/fonts/Urbanist-Regular.ttf"),
    include_bytes!("../assets/fonts/Urbanist-Medium.ttf"),
    include_bytes!("../assets/fonts/Urbanist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/Rajdhani-Medium.ttf"),
    include_bytes!("../assets/fonts/Rajdhani-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Rajdhani-Bold.ttf"),
];

fn bind_keys(cx: &mut App) {
    saturn_terminal::view::bind_keys(cx);
    // Copy, paste and quit are macOS's own and stay where they are; the rest the user can change.
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("cmd-c", Copy, None),
    ]);
    keymap::bind(&settings::load().keymap, cx);
}

/// The menu bar's menus; set again when a shortcut changes, so they show its new keys.
pub fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu::new("Saturn").items([
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit Saturn", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New File…", saturn_files::NewFile),
            MenuItem::action("New Folder…", saturn_files::NewFolder),
            MenuItem::separator(),
            MenuItem::action("Go to File…", GoToFile),
            MenuItem::action("Open Project…", OpenProject),
            MenuItem::action("Close Project", workspace::CloseRepo),
            MenuItem::action("Save", SaveFile),
        ]),
        Menu::new("Terminal").items([
            MenuItem::action("New Terminal", NewTab),
            MenuItem::action("New Agent…", OpenTicketPicker),
            MenuItem::action("Split Right", SplitRight),
            MenuItem::action("Split Down", SplitDown),
            MenuItem::action("Close Terminal", CloseTab),
            MenuItem::separator(),
            MenuItem::action("Next Pane", NextTab),
            MenuItem::action("Previous Pane", PreviousTab),
            MenuItem::action("Maximise Pane", ToggleMaximize),
            MenuItem::action("Even Out Panes", EvenOutPanes),
            MenuItem::action("Next Agent Needing You", NextAttention),
        ]),
        // Every feature has a place here, so none is found only by typing in the command palette.
        Menu::new("Agents").items([
            MenuItem::action("New Agent…", OpenTicketPicker),
            MenuItem::action("Best of N…", BestOfN),
            MenuItem::action("Send to Several Agents…", SendToSeveral),
            MenuItem::separator(),
            MenuItem::action("Search Sessions…", SearchSessions),
            MenuItem::action("Routines…", ShowRoutines),
            MenuItem::separator(),
            MenuItem::action("Agents Waiting for You…", ShowWaiting),
        ]),
        Menu::new("Pane").items([
            MenuItem::action("Start the App", StartApp),
            MenuItem::action("Preview the App", ShowPreview),
            MenuItem::action("Pictures of the App…", ShowPictures),
            MenuItem::action("Run a Check…", RunCheck),
            MenuItem::separator(),
            MenuItem::action("Edit the Pull Request's Description…", EditPrBody),
            MenuItem::action("Set Up This Worktree", SetUpWorktree),
            MenuItem::separator(),
            MenuItem::action("What the Agent Is Told…", ShowContext),
            MenuItem::action("Teach the Repo…", TeachRepo),
        ]),
        Menu::new("Repo").items([
            MenuItem::action("Worktrees…", ShowWorktrees),
            MenuItem::action("Compare Attempts…", CompareAttempts),
            MenuItem::action("Create Worktree Recipe", CreateRecipe),
            MenuItem::separator(),
            MenuItem::action("History", ShowHistory),
            MenuItem::action("Impact of the Changes", ShowImpact),
            MenuItem::action("Compare with a Branch or Commit…", CompareWith),
            MenuItem::separator(),
            MenuItem::action("Database Diagram", ShowSchema),
            MenuItem::action("dbt Lineage", ShowLineage),
            MenuItem::action("Problems", ShowProblems),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette…", CommandPalette),
            MenuItem::separator(),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Editor", ToggleEditor),
            MenuItem::action("Toggle Tickets", ToggleTickets),
            MenuItem::separator(),
            MenuItem::action("Bigger Text", TextBigger),
            MenuItem::action("Smaller Text", TextSmaller),
            MenuItem::action("Actual Size", TextReset),
        ]),
        Menu::new("Edit").items([MenuItem::action("Copy", Copy), MenuItem::action("Paste", Paste)]),
    ]);
}

fn main() {
    // Claude Code runs Saturn for its status line: print it and go, without a window.
    if std::env::args().nth(1).as_deref() == Some(status_line::ARGUMENT) {
        status_line::print();
        return;
    }
    // Off the main thread: an interactive shell's startup files can take a moment.
    std::thread::spawn(|| {
        if let Some(path) = saturn_core::shell_env::login_path() {
            let _ = LOGIN_PATH.set(path);
        }
    });
    // A folder Saturn is started in or for; else the windows open when it last quit (an update's
    // relaunch, the Dock); else, with none, the welcome screen.
    let projects: Vec<Option<std::path::PathBuf>> = settings::startup_project().map_or_else(
        || {
            let open = session::take_open_windows();
            if open.is_empty() { vec![None] } else { open.into_iter().map(Some).collect() }
        },
        |project| vec![Some(project)],
    );

    let app = gpui_kit::application().with_assets(assets::Assets);
    // Clicking the Dock icon with no window open (the last one was closed) opens one, as a fresh
    // start does: the welcome screen, where a recent repo comes back with its grid.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(settings::startup_project(), cx);
            cx.activate(true);
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        languages::register();
        theme::apply(&settings::load(), cx);
        theme::set_text_size(settings::load().text_size, cx);
        agent_choice::start(cx);
        usage::start(cx);
        routines_view::start(cx);
        workspace::apply_app_settings(&settings::load(), cx);
        if settings::load().notifications && !spike::is_automated() {
            notify::request_permission();
        }
        cx.text_system()
            .add_fonts(FONTS.iter().map(|font| Cow::Borrowed(*font)).collect())
            .expect("the bundled fonts load");
        bind_keys(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CheckForUpdates, _| updates::check());
        // Only when the window's own handlers didn't get them: nothing had the keyboard.
        cx.on_action(|_: &GoToFile, cx| {
            workspace::in_front(cx, |workspace, window, cx| {
                palette::go_to_file(cx.weak_entity(), workspace.root().to_owned(), window, cx);
            });
        });
        cx.on_action(|_: &CommandPalette, cx| {
            workspace::in_front(cx, |_, window, cx| palette::commands(window, cx));
        });
        set_menus(cx);

        for project in projects {
            open_main_window(project, cx);
        }
        // Automated runs (screenshots, spikes) must not take the keyboard from whatever the user is doing.
        if !spike::is_automated() {
            cx.activate(true);
            show_menu_bar(cx);
            updates::start();
        } else if std::env::var_os("SATURN_UPDATES_DEBUG").is_some() {
            updates::start();
        }
    });
}

/// Saturn's window: on `project`, or the welcome screen with the recent repos.
fn open_main_window(project: Option<std::path::PathBuf>, cx: &mut App) {
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Saturn".into()),
            appears_transparent: true,
            // Centred in the title bar, as far in from the side as from the top.
            traffic_light_position: Some(point(px(14.), px(13.))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1600.), px(1000.)), cx))),
        // The glass is the window itself: macOS blurs the desktop behind the translucent ground.
        window_background: WindowBackgroundAppearance::Blurred,
        ..WindowOptions::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| AppView::new(project, window, cx)))
        .expect("the window opens");
}

/// Saturn's menu bar item: its menu brings Saturn forward, on the next agent that needs you when
/// that's what was picked, or quits it.
fn show_menu_bar(cx: &mut App) {
    let (clicks, clicked) = async_channel::unbounded();
    menu_bar::on_click(clicks);
    menu_bar::set_shown(settings::load().menu_bar);
    cx.spawn(async move |cx| {
        while let Ok(choice) = clicked.recv().await {
            cx.update(|cx| {
                if choice == menu_bar::Choice::Quit {
                    cx.quit();
                    return;
                }
                cx.activate(true);
                if cx.windows().is_empty() {
                    open_main_window(settings::startup_project(), cx);
                } else if let Some(window) = cx.windows().into_iter().next() {
                    let _ = window.update(cx, |_, window, cx| {
                        window.activate_window();
                        if choice == menu_bar::Choice::Next {
                            window.dispatch_action(Box::new(NextAttention), cx);
                        }
                    });
                }
            });
        }
    })
    .detach();
}
