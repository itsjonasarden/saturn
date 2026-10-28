//! The window's root: the title bar and the grid of terminal panes, each with tabs.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Action, Animation, AnimationExt as _, AnyElement, AppContext as _, Bounds, ClickEvent, Context, CursorStyle,
    DragMoveEvent, Entity, FocusHandle, Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, PathPromptOptions, Pixels, Point, PromptLevel, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, actions, bounce, div, ease_in_out,
    prelude::FluentBuilder as _, px, relative,
};
use saturn_bridge::{Bridge, HookCall};
use saturn_core::agent_state::{AgentState, tab_title};
use saturn_core::agents::Agent;
use saturn_core::best_of;
use saturn_core::checks;
use saturn_core::clickup::Ticket;
use saturn_core::git;
use saturn_core::layout::{Cell, CellId, Direction, Layout, RowId, TabId};
use saturn_core::multi_repo;
use saturn_core::overlap;
use saturn_core::ports;
use saturn_core::pull_request::{self, ChecksState, PrState, PullRequest, ReviewNotes};
use saturn_core::recipe::{self, Source};
use saturn_core::routines::Routine;
use saturn_core::servers::{self, Server, Service};
use saturn_core::session_name::rename_for_prompt;
use saturn_core::session_usage::{self, Tokens};
use saturn_core::shared_env;
use saturn_core::snapshot::{self, Snapshot};
use saturn_core::teams::{self, Team};
use saturn_core::waiting::{self, Waiting};
use saturn_core::worktree_include;
use saturn_core::worktrees::Worktree;
use saturn_design::{color, radius, shadow, space};
use saturn_terminal::view::{TerminalView, TerminalViewEvent};

use crate::changes_view::{ChangesEvent, ChangesView};
use crate::chrome;
use crate::clickup;
use crate::editor_view::EditorView;
use crate::files_view::{FilesEvent, FilesView};
use crate::menu;
use crate::review_comments::{SendToAgent, Touched};
use crate::session::{self, SavedGrid, SavedPage, SavedTab};
use crate::settings::{self, AppearanceSetting, Panels, Settings};
use crate::status_bar::StatusBar;
use crate::theme::{self, len};
use crate::tickets_view::TicketsView;
use crate::{dock, github, menu_bar, notify, palette, settings_view, ticket_picker};

actions!(
    saturn,
    [
        NewTab,
        SplitRight,
        SplitDown,
        CloseTab,
        NextTab,
        PreviousTab,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        ToggleMaximize,
        EvenOutPanes,
        NextAttention,
        Paste,
        Copy,
        ToggleSidebar,
        ToggleEditor,
        OpenProject,
        OpenSettings,
        SaveFile,
        OpenTicketPicker,
        FocusPane1,
        FocusPane2,
        FocusPane3,
        FocusPane4,
        FocusPane5,
        FocusPane6,
        FocusPane7,
        FocusPane8,
        FocusPane9,
        GoToFile,
        CommandPalette,
        ShowHistory,
        ShowImpact,
        ShowSchema,
        ShowLineage,
        ShowProblems,
        ToggleTickets,
        TextBigger,
        TextSmaller,
        TextReset,
        SetUpWorktree,
        StartApp,
        CreateRecipe,
        ShowWorktrees,
        CompareWith,
        ShowWaiting,
        RunCheck,
        EditPrBody,
        ShowPictures,
        ShowPreview,
        ShowContext,
        TeachRepo,
        SendToSeveral,
        BestOfN,
        CompareAttempts,
        SearchSessions,
        ShowRoutines,
    ]
);

thread_local! {
    /// Every window's workspace, so app-wide shortcuts can find the one in front.
    static OPEN: std::cell::RefCell<Vec<(gpui_kit::AnyWindowHandle, gpui_kit::WeakEntity<Workspace>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `act` on the workspace of the window in front, if it has one: for shortcuts that arrive
/// while nothing in the window has the keyboard.
pub fn in_front(cx: &mut gpui_kit::App, act: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)) {
    let Some(front) = cx.active_window() else { return };
    let workspace = OPEN.with_borrow_mut(|open| {
        open.retain(|(_, workspace)| workspace.upgrade().is_some());
        open.iter().find(|(window, _)| *window == front).map(|(_, workspace)| workspace.clone())
    });
    if let Some(workspace) = workspace.and_then(|workspace| workspace.upgrade()) {
        let _ = front.update(cx, |_, window, cx| workspace.update(cx, |workspace, cx| act(workspace, window, cx)));
    }
}

/// Runs `act` on the workspace of the window in front, else of the first window open, for what
/// happens on its own (a routine due). Whether there was one.
pub fn in_any(cx: &mut gpui_kit::App, act: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)) -> bool {
    let front = cx.active_window();
    let chosen = OPEN.with_borrow_mut(|open| {
        open.retain(|(_, workspace)| workspace.upgrade().is_some());
        open.iter().find(|(window, _)| Some(*window) == front).or(open.first()).cloned()
    });
    let Some((window, workspace)) = chosen else { return false };
    let Some(workspace) = workspace.upgrade() else { return false };
    window.update(cx, |_, window, cx| workspace.update(cx, |workspace, cx| act(workspace, window, cx))).is_ok()
}

/// Every command the command palette offers: its name, its icon, and the action it runs (whose
/// shortcut the palette shows).
pub fn commands() -> Vec<(&'static str, &'static str, Box<dyn gpui_kit::Action>)> {
    vec![
        ("Go to file", "search", Box::new(GoToFile)),
        ("New terminal", "plus", Box::new(NewTab)),
        ("Split right", "columns-2", Box::new(SplitRight)),
        ("Split down", "rows-2", Box::new(SplitDown)),
        ("Close terminal", "x", Box::new(CloseTab)),
        ("Next tab", "square-arrow-right", Box::new(NextTab)),
        ("Previous tab", "arrow-left", Box::new(PreviousTab)),
        ("Maximise the pane", "maximize-2", Box::new(ToggleMaximize)),
        ("Even out the panes", "columns-2", Box::new(EvenOutPanes)),
        ("Go to the next agent that needs you", "bot", Box::new(NextAttention)),
        ("New agent", "square-plus", Box::new(OpenTicketPicker)),
        ("Send to several agents", "send", Box::new(SendToSeveral)),
        ("Best of N: one task, several agents", "git-fork", Box::new(BestOfN)),
        ("Compare attempts", "git-compare", Box::new(CompareAttempts)),
        ("Search sessions and resume one", "text-search", Box::new(SearchSessions)),
        ("Routines: agents on a schedule", "calendar-clock", Box::new(ShowRoutines)),
        ("Show or hide the sidebar", "panel-left", Box::new(ToggleSidebar)),
        ("Show or hide the editor", "panel-top", Box::new(ToggleEditor)),
        ("Show or hide your tickets", "panel-right", Box::new(ToggleTickets)),
        ("Bigger text in terminals and files", "zoom-in", Box::new(TextBigger)),
        ("Smaller text in terminals and files", "zoom-out", Box::new(TextSmaller)),
        ("Usual text size", "scan", Box::new(TextReset)),
        ("History", "git-branch", Box::new(ShowHistory)),
        ("Impact of the changes", "waypoints", Box::new(ShowImpact)),
        ("Set up this worktree", "wrench", Box::new(SetUpWorktree)),
        ("Start the app", "play", Box::new(StartApp)),
        ("Create worktree recipe", "file-code", Box::new(CreateRecipe)),
        ("Worktrees", "folder-git", Box::new(ShowWorktrees)),
        ("Compare with a branch or commit", "git-compare", Box::new(CompareWith)),
        ("Agents waiting for you", "bot", Box::new(ShowWaiting)),
        ("Run a check", "play", Box::new(RunCheck)),
        ("Edit the pull request's description", "git-pull-request", Box::new(EditPrBody)),
        ("Pictures of the app", "camera", Box::new(ShowPictures)),
        ("Preview the app", "globe", Box::new(ShowPreview)),
        ("What the agent is told", "book-open", Box::new(ShowContext)),
        ("Teach the repo: keep a correction for every session", "sprout", Box::new(TeachRepo)),
        ("Database diagram", "database", Box::new(ShowSchema)),
        ("dbt lineage", "workflow", Box::new(ShowLineage)),
        ("Problems", "circle-x", Box::new(ShowProblems)),
        ("Save the file", "file-code", Box::new(SaveFile)),
        ("Add a repo", "folder", Box::new(OpenProject)),
        ("Settings", "settings", Box::new(OpenSettings)),
        ("Check for updates", "square-arrow-right", Box::new(crate::CheckForUpdates)),
    ]
}

impl Panels {
    /// The panels on screen: none while a pane is maximised, since it covers them all.
    fn on_screen(self, maximized: bool) -> Self {
        if maximized { Self { sidebar: false, editor: false, tickets: false } } else { self }
    }
}

#[derive(Clone, Copy)]
pub enum Placement {
    Tab,
    Right,
    Down,
}

/// Tall enough that the title bar's outer controls sit 10px from the window's edges: 48 = 28 + 2 × 10.
const TITLEBAR_HEIGHT: f32 = 40.0;
/// The title bar's controls: smaller than the panes' (`chrome::CONTROL_HEIGHT`), so the bar stays a
/// thin strip. Their tracks are this plus the track's padding, and macOS's buttons sit on the same
/// middle line (see `traffic_light_position` in main.rs).
const TITLEBAR_CONTROL: f32 = 24.0;
const TRAFFIC_LIGHTS_WIDTH: f32 = 76.0;
const TAB_HEIGHT: f32 = chrome::CONTROL_HEIGHT;
/// A tab's narrowest: its status, the start of its name and its ✕.
const TAB_MIN_WIDTH: f32 = 64.0;
/// No pane is dragged smaller than this share of its row or of the grid.
const MIN_SHARE: f32 = 0.08;
/// Programs a tab can close without asking, because nothing is lost.
const SHELLS: &[&str] = &["zsh", "-zsh", "bash", "-bash", "fish", "sh", "login"];

/// Coding agents, by the name of their program: what review comments may be pasted into.
const AGENTS: &[&str] = &["claude", "codex", "gemini", "opencode", "aider", "amp", "cursor-agent"];

/// A tab being dragged to another place in the grid.
#[derive(Clone)]
struct DraggedTab {
    tab: TabId,
    name: String,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(len(space::SPACE_3))
            .h(len(TAB_HEIGHT))
            .flex()
            .items_center()
            .rounded(len(radius::RADIUS_SM))
            .bg(theme::fill_pressed())
            .border_1()
            .border_color(theme::color(color::BLUE))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::color(color::INK))
            .font_family(saturn_design::font::SANS)
            .child(self.name.clone())
    }
}

/// A divider in the grid being dragged: between two rows, or between two cells of a row.
#[derive(Clone, Copy)]
enum GridDivider {
    Rows(usize),
    Cells(RowId, usize),
}

struct DividerPreview;

impl Render for DividerPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub struct Workspace {
    /// The repos this window holds, in the sidebar's order. The first names the saved session.
    projects: Vec<PathBuf>,
    /// The selected repo: where new tabs and ticket worktrees go, and whose ticket settings apply.
    project: PathBuf,
    /// The repo whose grid is showing. Each repo has its own, as a browser tab has its own page.
    page: PathBuf,
    /// The grid showing.
    layout: Layout,
    /// The other repos' grids, put aside while another shows; their terminals keep running.
    pages: HashMap<PathBuf, Layout>,
    terminals: HashMap<TabId, Entity<TerminalView>>,
    /// Names typed by hand; they win over the terminal's own title.
    names: HashMap<TabId, String>,
    /// Tabs that changed into a state asking for the user while they looked elsewhere.
    unseen: HashSet<TabId>,
    renaming: Option<(TabId, Entity<InputState>)>,
    next_tab: u64,
    focus: FocusHandle,
    /// Which of the window's panels are showing.
    panels: Panels,
    settings: Settings,
    /// Where the Saturn plugin's hooks reach this window; `None` if it could not start.
    bridge: Option<Bridge>,
    /// The last name Saturn gave each tab's Claude session.
    session_names: HashMap<TabId, String>,
    /// The number on the Dock icon right now.
    badge: usize,
    /// Agents waiting for you, as last told to the menu bar.
    waiting: usize,
    /// The sidebar's tree and changes, per repository or worktree it has shown, so switching back
    /// is instant and each keeps watching its folder.
    roots: HashMap<PathBuf, RootViews>,
    /// The folder the sidebar shows: the project, or the focused pane's worktree.
    root: PathBuf,
    /// Whether the sidebar follows the focused pane's worktree.
    follow_pane: bool,
    /// Open files, in the top panel.
    editor: Entity<EditorView>,
    /// Along the window's foot: problems, the language server, the cursor, the branch.
    status_bar: Entity<StatusBar>,
    /// Your ClickUp tickets, on the right (made the first time it opens).
    tickets_view: Option<Entity<TicketsView>>,
    /// What the sidebar shows.
    sidebar_tab: SidebarTab,
    /// The agent's session running in each tab (whose, and its id), from its hooks, so a restart can
    /// offer to resume it.
    sessions: HashMap<TabId, (Agent, String)>,
    /// The agent Saturn started in each tab (by typing its command), or found running there, before
    /// any hook says more. Gone when the shell's prompt comes back.
    started: HashMap<TabId, Agent>,
    /// When Saturn started (or found) a tab's agent, in Unix seconds: a Codex session that began in
    /// its folder since then is its own.
    started_at: HashMap<TabId, i64>,
    /// When each tab's foreground program was last looked up, to find an agent typed by hand without
    /// running `ps` on every title change.
    program_checked: HashMap<TabId, Instant>,
    /// Panes a routine started, until their session is known: the routine and its time.
    routine_tabs: HashMap<TabId, (String, i64)>,
    /// The agent team each pane's session leads, if any.
    teams: HashMap<TabId, Team>,
    /// The window's other repos the next agent also works in, picked in New agent.
    also_in: Vec<PathBuf>,
    /// For each pane in a worktree, the same work's worktrees in the window's other repos: their
    /// folders and their repos' names.
    links: HashMap<TabId, Vec<(PathBuf, String)>>,
    /// ClickUp, when a token is set; nothing about it shows otherwise.
    clickup: ClickUpState,
    /// Where a dragged tab would land: a cell, and the side to split it on (`None`: into the cell).
    drop_target: Option<(CellId, Option<Direction>)>,
    /// The repository and branch each tab is in, read in the background.
    checkouts: HashMap<TabId, Checkout>,
    /// The pull request each tab works on, from `gh`: an open one naming its ticket, else the one last
    /// linked in it, else its branch's, else a merged or closed one naming its ticket.
    pull_requests: HashMap<TabId, PullRequest>,
    /// Whether the two above are being read, so refreshes don't pile up.
    checkout_refresh: Refresh,
    /// The checkouts whose env files have been shared with the main checkout's this session.
    env_shared: HashSet<PathBuf>,
    /// The servers running under each tab's shell (an agent's `pnpm dev`), by port.
    servers: HashMap<TabId, Vec<Server>>,
    /// This window's repos' Docker Compose services with ports on this Mac: shared by every tab.
    services: Vec<Service>,
    /// Whether the two above are being read.
    reading_servers: bool,
    /// Each checkout's first port (`SATURN_PORT`), as its terminals were given it.
    ports: HashMap<PathBuf, u16>,
    /// For each pane's checkout, the others of its repository that change some of the same files.
    overlaps: std::collections::BTreeMap<PathBuf, Vec<(PathBuf, Vec<String>)>>,
    /// Whether they're being read.
    reading_overlaps: bool,
    /// Each tab's agent's last turn: its files before and after, to undo it.
    turns: HashMap<TabId, Turn>,
    /// Each tab's Claude session transcript, as its hooks name it.
    transcripts: HashMap<TabId, PathBuf>,
    /// What each tab's session used in the plan's current 5-hour window.
    spent: HashMap<TabId, Tokens>,
    /// What each tab's agent waits for, from its hooks.
    waits: HashMap<TabId, Waiting>,
    /// The button menu that's open, if one is.
    open_menu: Option<OpenMenu>,
    /// The menu a click outside just closed, and when: the click that closes a menu by landing on
    /// its own button must not open it again.
    menu_dismissed: Option<(OpenMenu, Instant)>,
    /// The checks run in each checkout, newest last: running ones have no outcome yet.
    check_runs: HashMap<PathBuf, Vec<CheckRun>>,
    /// What reviewers left open on each tab's pull request.
    review_notes: HashMap<TabId, ReviewNotes>,
}

/// A check run in a checkout, and how it went (`None` while it runs).
struct CheckRun {
    name: String,
    command: String,
    log: PathBuf,
    outcome: Option<checks::Outcome>,
}

/// An agent's turn, saved before and after (`saturn_core::snapshot`), so its files can go back.
struct Turn {
    checkout: PathBuf,
    before: Option<Snapshot>,
    /// Once the turn has ended.
    after: Option<Snapshot>,
    /// What it changed.
    files: Vec<String>,
    undone: bool,
    /// Being undone or redone right now.
    busy: bool,
    /// What the last undo or redo said.
    said: Option<String>,
}

/// A background refresh: not running, running, or running with another asked for meanwhile (a new
/// tab, a folder change), to run once it's done.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Refresh {
    Idle,
    Running,
    RunningStale,
}

/// A menu opened from a button (see `menu`): New agent's arrow, a pane's ⋯, or the repo's ⋯.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenMenu {
    NewAgent,
    Pane(CellId),
    Repo,
}

/// More ways to start agents, under New agent's arrow.
fn new_agent_menu() -> Vec<menu::Item> {
    vec![
        Some(("square-plus", "New agent", Box::new(OpenTicketPicker))),
        Some(("git-fork", "Best of N: one task, several agents", Box::new(BestOfN))),
        Some(("send", "Send to several agents", Box::new(SendToSeveral))),
        None,
        Some(("text-search", "Search sessions and resume one", Box::new(SearchSessions))),
        Some(("calendar-clock", "Routines: agents on a schedule", Box::new(ShowRoutines))),
    ]
}

/// What applies to one pane: its app, its checks, its pull request, what its agent is told.
fn pane_menu() -> Vec<menu::Item> {
    vec![
        Some(("play", "Start the app", Box::new(StartApp))),
        Some(("globe", "Preview the app", Box::new(ShowPreview))),
        Some(("camera", "Pictures of the app", Box::new(ShowPictures))),
        Some(("list-checks", "Run a check", Box::new(RunCheck))),
        None,
        Some(("git-pull-request", "Edit the pull request's description", Box::new(EditPrBody))),
        Some(("wrench", "Set up this worktree", Box::new(SetUpWorktree))),
        None,
        Some(("book-open", "What the agent is told", Box::new(ShowContext))),
        Some(("sprout", "Teach the repo: keep a correction", Box::new(TeachRepo))),
    ]
}

/// What applies to the repo the sidebar shows: its worktrees and its history.
fn repo_menu() -> Vec<menu::Item> {
    vec![
        Some(("folder-git", "Worktrees", Box::new(ShowWorktrees))),
        Some(("git-compare", "Compare attempts", Box::new(CompareAttempts))),
        Some(("file-code", "Create worktree recipe", Box::new(CreateRecipe))),
        None,
        Some(("git-branch", "History", Box::new(ShowHistory))),
        Some(("waypoints", "Impact of the changes", Box::new(ShowImpact))),
        Some(("git-compare", "Compare with a branch or commit", Box::new(CompareWith))),
    ]
}

/// A branch checked out in a repository (or worktree), named by its top folder.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Checkout {
    root: PathBuf,
    branch: String,
}

/// What a tab's pull request can be found from.
struct TabSources {
    tab: TabId,
    dir: PathBuf,
    /// The pull request last linked in the tab's terminal.
    link: Option<String>,
    /// The ticket the tab is named after.
    ticket: Option<String>,
}

/// What finds a pull request, so tabs that share one ask `gh` once.
#[derive(Clone, PartialEq, Eq, Hash)]
enum PrLookup {
    Link(String),
    Branch(Checkout),
}

/// The branch each tab is on, and the pull request each works on: an open one whose title or branch
/// names its ticket, else the one last linked in it (the agent opened it, or you pasted it), else its
/// branch's, else a merged or closed one that names its ticket. Slow: runs `git` and `gh`.
fn read_checkouts(
    tabs: Vec<TabSources>,
) -> (HashMap<TabId, Checkout>, HashMap<TabId, PullRequest>, HashMap<TabId, ReviewNotes>) {
    let mut checkouts = HashMap::new();
    let mut pull_requests = HashMap::new();
    let mut found: HashMap<PrLookup, Option<PullRequest>> = HashMap::new();
    let mut for_tickets: HashMap<(PathBuf, String), Option<PullRequest>> = HashMap::new();
    for sources in tabs {
        let checkout =
            git::toplevel(&sources.dir).and_then(|root| git::branch(&root).map(|branch| Checkout { root, branch }));
        let root = checkout.as_ref().map_or(&sources.dir, |checkout| &checkout.root).clone();
        let mut look = |lookup: PrLookup| {
            found
                .entry(lookup.clone())
                .or_insert_with(|| match &lookup {
                    PrLookup::Link(link) => github::pull_request(&root, link),
                    PrLookup::Branch(checkout) => github::pull_request(&checkout.root, &checkout.branch),
                })
                .clone()
        };
        // An open pull request that names the tab's ticket (in its title or branch) wins even over a
        // link, so a link to some other pull request the agent mentions in passing doesn't take the
        // tab's chip. A merged or closed one only counts when nothing else is found.
        let for_ticket = sources.ticket.filter(|_| checkout.is_some()).and_then(|ticket| {
            for_tickets
                .entry((root.clone(), ticket.clone()))
                .or_insert_with(|| github::pull_request_for_ticket(&root, &ticket))
                .clone()
        });
        let ticket_open = for_ticket.clone().filter(|pr| matches!(pr.state, PrState::Open | PrState::Draft));
        let pull_request = ticket_open
            .or_else(|| sources.link.and_then(|link| look(PrLookup::Link(link))))
            .or_else(|| checkout.clone().and_then(|checkout| look(PrLookup::Branch(checkout))))
            .or(for_ticket);
        if let Some(checkout) = checkout {
            checkouts.insert(sources.tab, checkout);
        }
        if let Some(pull_request) = pull_request {
            pull_requests.insert(sources.tab, pull_request);
        }
    }
    // Where open pull requests are deployed, once each; a deployment wins over a check's link.
    let mut previews: HashMap<u64, Option<pull_request::Preview>> = HashMap::new();
    for (tab, pr) in &mut pull_requests {
        if !matches!(pr.state, PrState::Open | PrState::Draft) {
            continue;
        }
        let Some(root) = checkouts.get(tab).map(|checkout: &Checkout| checkout.root.clone()) else { continue };
        let found = previews.entry(pr.number).or_insert_with(|| github::deployment_preview(&root, pr)).clone();
        if found.is_some() {
            pr.preview = found;
        }
    }
    // What reviewers left open, for open pull requests only, once each.
    let mut notes_of: HashMap<u64, Option<ReviewNotes>> = HashMap::new();
    let notes = pull_requests
        .iter()
        .filter(|(_, pr)| matches!(pr.state, PrState::Open | PrState::Draft))
        .filter_map(|(tab, pr)| {
            let root = checkouts.get(tab).map(|checkout: &Checkout| checkout.root.clone())?;
            let found = notes_of.entry(pr.number).or_insert_with(|| github::review_notes(&root, pr)).clone()?;
            (found.count() > 0).then_some((*tab, found))
        })
        .collect();
    (checkouts, pull_requests, notes)
}

/// The ClickUp connection as the window knows it.
#[derive(Default)]
pub struct ClickUpState {
    token: Option<String>,
    /// The workspace the token reaches, once checked.
    pub account: Option<String>,
    /// The last thing that went wrong, shown in Settings and the ticket picker.
    pub problem: Option<String>,
    /// Open tickets assigned to the token's owner, refreshed every minute.
    pub tickets: Vec<Ticket>,
    pub loading: bool,
    /// The demo's tickets: made up, so never refreshed from ClickUp.
    made_up: bool,
}

impl ClickUpState {
    pub fn connected(&self) -> bool {
        self.token.is_some()
    }
}

/// What the sidebar shows for one folder.
struct RootViews {
    files: Entity<FilesView>,
    changes: Entity<ChangesView>,
    /// The branch checked out there, for the sidebar's header.
    branch: Option<String>,
    /// Whether it describes a database (Prisma or SQL migrations), for the diagram's button.
    has_schema: bool,
    /// The dbt project in it, for the lineage graph's button.
    dbt_project: Option<PathBuf>,
    _events: [Subscription; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarTab {
    Files,
    Changes,
}

/// The settings every window shares: the editor's (read by each editor as it draws) and the
/// terminals'.
pub fn apply_app_settings(settings: &Settings, cx: &mut gpui_kit::App) {
    use std::sync::atomic::Ordering;
    cx.set_global(settings.editor.clone());
    saturn_terminal::view::OPTION_AS_META.store(settings.terminal.option_as_meta, Ordering::Relaxed);
    saturn_terminal::view::COPY_ON_SELECT.store(settings.terminal.copy_on_select, Ordering::Relaxed);
    cx.refresh_windows();
}

/// The Saturn plugin that Claude sessions in Saturn's panes load: inside Saturn.app
/// (`Contents/Resources/plugin`), else `plugin/` in this repository when run from source.
pub fn plugin_dir() -> Option<PathBuf> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|macos| macos.join("../Resources/plugin")))
        .and_then(|dir| dir.canonicalize().ok());
    std::env::var_os("SATURN_PLUGIN_DIR")
        .map(PathBuf::from)
        .or(bundled)
        .or_else(|| Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugin")).canonicalize().ok())
        .filter(|dir| dir.join(".claude-plugin/plugin.json").is_file())
}

/// Starts the hook bridge on a socket only this Saturn instance uses.
fn start_bridge() -> Option<(Bridge, async_channel::Receiver<HookCall>)> {
    let path = std::env::temp_dir().join(format!("saturn-{}.sock", std::process::id()));
    match Bridge::start(&path) {
        Ok(started) => Some(started),
        Err(error) => {
            eprintln!("saturn: hooks can't reach this window ({error}); agent states come from the terminal only");
            None
        }
    }
}

/// A pane's ticket: its status in ClickUp's own colour, opening the ticket.
fn ticket_chip(cell: CellId, ticket: &Ticket) -> impl IntoElement {
    let url = ticket.url.clone();
    div()
        .id(("ticket-chip", cell.key()))
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_2))
        .h(len(TAB_HEIGHT))
        .px(len(space::SPACE_3))
        .rounded(len(radius::RADIUS_SM))
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_MUTED))
        .hover(|s| s.bg(theme::fill_hover()))
        .child(ticket_picker::status_dot(ticket))
        .child(ticket.status.clone())
        .on_click(move |_, _, _| {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        })
}

/// A pane's pull request: its number in the colour of its state, opening it on GitHub.
fn pull_request_chip(cell: CellId, pr: &PullRequest) -> impl IntoElement {
    let (icon, state_color) = match pr.state {
        PrState::Open => ("git-pull-request", color::TERM_GREEN),
        PrState::Draft => ("git-pull-request-draft", color::INK_FAINT),
        PrState::Merged => ("git-merge", color::TERM_MAGENTA),
        PrState::Closed => ("git-pull-request-closed", color::RED),
    };
    let url = pr.url.clone();
    let checks = match pr.checks.state() {
        Some(ChecksState::Passed) => format!("\nChecks passed ({})", pr.checks.passed),
        Some(ChecksState::Running) => format!("\nChecks running: {}", pr.checks.running.join(", ")),
        Some(ChecksState::Failed) => format!(
            "\nChecks failed: {}",
            pr.checks.failed.iter().map(|check| check.name.as_str()).collect::<Vec<_>>().join(", ")
        ),
        None => String::new(),
    };
    let tip = format!("{} · {}{checks}", pr.state.label(), pr.title);
    let mark = pr.checks.state().map(|state| match state {
        ChecksState::Passed => chrome::icon("circle-check", 12., theme::color(color::TERM_GREEN)),
        ChecksState::Running => chrome::icon("circle-dot", 12., theme::color(color::AMBER_TEXT)),
        ChecksState::Failed => chrome::icon("circle-x", 12., theme::color(color::RED)),
    });
    div()
        .id(("pr-chip", cell.key()))
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_1))
        .h(len(TAB_HEIGHT))
        .px(len(space::SPACE_2))
        .rounded(len(radius::RADIUS_SM))
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_MUTED))
        .hover(|s| s.bg(theme::fill_hover()))
        .child(chrome::icon(icon, 14., theme::color(state_color)))
        .child(format!("#{}", pr.number))
        .children(mark)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, _, _| {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        })
}

/// Where the pane's pull request is deployed: opens it, or says it's on its way.
fn preview_chip(cell: CellId, pr: &PullRequest) -> Option<impl IntoElement> {
    let preview = pr.preview.clone()?;
    let tip = match &preview.url {
        Some(url) => format!("Open the preview ({}): {url}", preview.name),
        None => format!("Deploying the preview ({})…", preview.name),
    };
    Some(
        div()
            .id(("preview-chip", cell.key()))
            .flex()
            .flex_none()
            .items_center()
            .gap(len(space::SPACE_1))
            .h(len(TAB_HEIGHT))
            .px(len(space::SPACE_2))
            .rounded(len(radius::RADIUS_SM))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::color(if preview.ready { color::INK_MUTED } else { color::INK_FAINT }))
            .when(preview.url.is_some(), |chip| chip.cursor_pointer().hover(|s| s.bg(theme::fill_hover())))
            .child(chrome::icon("external-link", 13., theme::color(color::INK_MUTED)))
            .child(if preview.ready { "Preview" } else { "Deploying…" })
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(move |_, _, _| {
                if let Some(url) = &preview.url {
                    let _ = std::process::Command::new("open").arg(url).spawn();
                }
            }),
    )
}

/// The same work in another repo, linked by its branch: goes to it.
fn link_chip(
    cell: CellId,
    index: usize,
    folder: &Path,
    repo: &str,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let folder = folder.to_owned();
    div()
        .id(gpui_kit::SharedString::from(format!("link-chip-{}-{index}", cell.key())))
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_1))
        .h(len(chrome::CONTROL_HEIGHT))
        .px(len(space::SPACE_2))
        .rounded(len(radius::RADIUS_SM))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_MUTED))
        .cursor_pointer()
        .hover(|s| s.bg(theme::fill_hover()))
        .child(chrome::icon("link-2", 13., theme::color(color::INK_MUTED)))
        .child(repo.to_owned())
        .tooltip(chrome::tip("The same work in another repo: go to it"))
        .on_click(cx.listener(move |this: &mut Workspace, _, window, cx| this.go_to_folder(&folder, window, cx)))
}

/// A pane working on one of Best of N's attempts: which one. It compares them all.
fn attempt_chip(cell: CellId, number: usize) -> impl IntoElement {
    div()
        .id(("attempt-chip", cell.key()))
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_1))
        .h(len(chrome::CONTROL_HEIGHT))
        .px(len(space::SPACE_2))
        .rounded(len(radius::RADIUS_SM))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_MUTED))
        .cursor_pointer()
        .hover(|s| s.bg(theme::fill_hover()))
        .child(chrome::icon("git-fork", 13., theme::color(color::INK_MUTED)))
        .child(format!("Attempt {number}"))
        .tooltip(chrome::tip_for("Compare the attempts side by side", CompareAttempts))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(CompareAttempts), cx))
}

/// A pane whose session leads an agent team: how many are in it, how far its tasks are, and
/// whether messages wait unread. It opens the team.
fn team_chip(cell: CellId, tab: TabId, team: &Team, cx: &mut Context<Workspace>) -> impl IntoElement {
    let (done, all) = team.progress();
    let mut label = format!("Team of {}", team.members.len());
    if all > 0 {
        let _ = write!(label, " · {done}/{all} tasks");
    }
    let unread = team.unread();
    div()
        .id(("team-chip", cell.key()))
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_1))
        .h(len(chrome::CONTROL_HEIGHT))
        .px(len(space::SPACE_2))
        .rounded(len(radius::RADIUS_SM))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_MUTED))
        .cursor_pointer()
        .hover(|s| s.bg(theme::fill_hover()))
        .child(chrome::icon("users", 13., theme::color(color::INK_MUTED)))
        .child(label)
        .when(unread > 0, |chip| {
            chip.child(div().text_color(theme::color(color::BLUE)).child(format!("· {unread} unread")))
        })
        .tooltip(chrome::tip("The team's members, tasks and messages"))
        .on_click(cx.listener(move |this: &mut Workspace, _, window, cx| {
            let title = this.teams.get(&tab).map(|team| team.name.clone()).unwrap_or_default();
            crate::team_view::open(cx.weak_entity(), tab, title, window, cx);
        }))
}

/// This pane's worktree changes files another worktree also changes: they'll meet at merge time.
/// Read-only: what to do about it is a decision, not a click.
fn overlap_chip(cell: CellId, others: &[(PathBuf, Vec<String>)]) -> impl IntoElement {
    let count: usize = others.iter().map(|(_, files)| files.len()).sum();
    let mut tip = String::from("Other worktrees change the same files, so merging both will meet here:");
    for (other, files) in others {
        let name =
            other.file_name().map_or_else(|| other.display().to_string(), |name| name.to_string_lossy().into_owned());
        let shown: Vec<&str> = files.iter().take(8).map(String::as_str).collect();
        let more =
            if files.len() > shown.len() { format!(" and {} more", files.len() - shown.len()) } else { String::new() };
        let _ = write!(tip, "\n\n{name}: {}{more}", shown.join(", "));
    }
    let s = if count == 1 { "" } else { "s" };
    div()
        .id(("overlap-chip", cell.key()))
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_1))
        .h(len(TAB_HEIGHT))
        .px(len(space::SPACE_2))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_MUTED))
        .child(chrome::icon("triangle-alert", 13., theme::color(color::AMBER_TEXT)))
        .child(format!("{count} shared file{s}"))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
}

/// A server the pane's terminal runs: its name and port. Clicking opens it in the browser.
fn server_chip(cell: CellId, server: &Server) -> impl IntoElement {
    let url = format!("http://localhost:{}", server.port);
    let tip = format!("{url} · {}", server.command);
    div()
        .id(("server-chip", cell.key() * 100_000 + u64::from(server.port)))
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_1))
        .h(len(TAB_HEIGHT))
        .px(len(space::SPACE_2))
        .rounded(len(radius::RADIUS_SM))
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_MUTED))
        .hover(|s| s.bg(theme::fill_hover()))
        .child(chrome::icon("globe", 13., theme::color(color::INK_FAINT)))
        .child(server.name.clone())
        .child(div().text_color(theme::color(color::INK_FAINT)).child(format!(":{}", server.port)))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, _, _| {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        })
}

/// What a pane says when its repo has no recipe yet.
fn no_recipe() -> String {
    format!(
        "Saturn couldn't tell how this repo is set up or started. Add {} to the repo, for example:\n  {{ \"setup\": [\"pnpm install\"], \"run\": {{ \"web\": \"pnpm dev\" }} }}",
        recipe::FILE
    )
}

/// What a pane says before steps Saturn guessed.
fn guessed() -> String {
    "Saturn guessed these steps from the repo's files. \"Create worktree recipe\" (⌘⇧P) saves them to adjust and commit."
        .to_owned()
}

/// Where in a pane a dragged tab lands: within a quarter of an edge splits the pane on that side,
/// anywhere further in moves the tab into the pane (`None`).
fn drop_side(bounds: Bounds<Pixels>, at: Point<Pixels>) -> Option<Direction> {
    let x = f32::from(at.x - bounds.origin.x) / f32::from(bounds.size.width).max(1.);
    let y = f32::from(at.y - bounds.origin.y) / f32::from(bounds.size.height).max(1.);
    let edges = [(x, Direction::Left), (1. - x, Direction::Right), (y, Direction::Up), (1. - y, Direction::Down)];
    let (distance, side) =
        edges.into_iter().fold((f32::MAX, Direction::Left), |best, edge| if edge.0 < best.0 { edge } else { best });
    (distance < 0.25).then_some(side)
}

/// The blue preview of where a dropped tab goes: half the pane on a side, or the whole pane.
fn drop_preview(side: Option<Direction>) -> impl IntoElement {
    let preview = div()
        .absolute()
        .rounded(len(radius::RADIUS_SM))
        .bg(theme::color(color::BLUE_VIVID).opacity(0.14))
        .border_2()
        .border_color(theme::color(color::BLUE).opacity(0.7));
    match side {
        Some(Direction::Left) => preview.top_0().left_0().h_full().w(relative(0.5)),
        Some(Direction::Right) => preview.top_0().right_0().h_full().w(relative(0.5)),
        Some(Direction::Up) => preview.top_0().left_0().w_full().h(relative(0.5)),
        Some(Direction::Down) => preview.bottom_0().left_0().w_full().h(relative(0.5)),
        None => preview.top_0().left_0().size_full(),
    }
}

pub fn state_color(state: AgentState) -> Hsla {
    theme::color(match state {
        AgentState::Shell | AgentState::Idle => color::STATUS_IDLE,
        AgentState::Working => color::STATUS_WORKING,
        AgentState::NeedsYou => color::STATUS_NEEDS_YOU,
        AgentState::YourTurn => color::STATUS_YOUR_TURN,
        AgentState::Error => color::STATUS_ERROR,
    })
}

fn status_dot(state: AgentState) -> impl IntoElement {
    div().size(px(8.)).flex_none().rounded_full().bg(state_color(state))
}

/// `glow-needs-you` as it breathes: `t` from 0 (the token itself) to 1 (wider and brighter).
fn breathing_glow(t: f32) -> Vec<saturn_design::Shadow> {
    let mut glow = shadow::GLOW_NEEDS_YOU.get().to_vec();
    if let Some(halo) = glow.get_mut(1) {
        halo.blur += 4. * t;
        halo.color.alpha += 0.12 * t;
    }
    glow
}

/// Moves the divider after `index` to `at` (0–1 across the container), keeping both neighbours
/// above the minimum share.
fn move_divider(sizes: &[f32], index: usize, at: f32) -> Vec<f32> {
    let total: f32 = sizes.iter().sum();
    let before: f32 = sizes[..index].iter().sum();
    let pair = sizes[index] + sizes[index + 1];
    let minimum = total * MIN_SHARE;
    let first = (at * total - before).clamp(minimum, (pair - minimum).max(minimum));
    let mut next = sizes.to_vec();
    next[index] = first;
    next[index + 1] = pair - first;
    next
}

/// Shares the space of the two neighbours of the divider after `index` evenly between them,
/// leaving the others as they are.
fn even_divider(sizes: &[f32], index: usize) -> Vec<f32> {
    let half = f32::midpoint(sizes[index], sizes[index + 1]);
    let mut next = sizes.to_vec();
    next[index] = half;
    next[index + 1] = half;
    next
}

/// The name of the program in the foreground of a terminal, e.g. `zsh` or `claude`.
fn process_name(pid: i32) -> Option<String> {
    let output = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output().ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let name = name.rsplit('/').next().unwrap_or(&name).to_owned();
    (!name.is_empty()).then_some(name)
}

impl Workspace {
    /// What changes outside Saturn without telling it, read again on timers: ClickUp's tickets,
    /// branches and pull requests, and the servers panes run.
    fn keep_fresh(window: &mut Window, cx: &mut Context<Self>) {
        // A saved ClickUp token turns the ticket picker on; it's read off the main thread.
        cx.spawn_in(window, async move |this, cx| {
            let token = cx.background_spawn(async { clickup::token() }).await;
            if let Some(token) = token {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.clickup.token = Some(token);
                    // The Tickets panel waits for ClickUp before it reopens: without it, it's empty.
                    if this.settings.panels.tickets {
                        this.open_tickets(window, cx);
                    } else {
                        this.refresh_tickets(cx);
                    }
                });
            }
            // Statuses change in ClickUp; a minute is fresh enough and far under its rate limit.
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(60)).await;
                if this.update(cx, Workspace::refresh_tickets).is_err() {
                    break;
                }
            }
        })
        .detach();
        // Branches change inside panes without telling Saturn, and pull requests change on GitHub;
        // a minute is fresh enough and far under GitHub's rate limit.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(60)).await;
                if this
                    .update(cx, |this, cx| {
                        this.refresh_checkouts(cx);
                        this.refresh_spent(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        // Agent teams message each other inside a pane without a hook for every message; their
        // files are small, so every few seconds keeps the team chips current.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if this.update(cx, Workspace::refresh_teams).is_err() {
                    break;
                }
            }
        })
        .detach();
        // Servers start and stop without telling anyone; reading them takes a few hundredths of a
        // second, so every few seconds keeps the chips current.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                if this.update(cx, Workspace::refresh_servers).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn new(project: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::keep_fresh(window, cx);
        // Pane sizes change by dragging, which saves nothing on its own; quitting saves them, and
        // that this window was open, so the next start opens it again.
        cx.on_app_quit(|this, cx| {
            this.save_session(cx);
            this.remember_panels(cx);
            session::remember_open_window(&this.projects[0]);
            async {}
        })
        .detach();
        // Following the system: redraw when macOS switches between light and dark.
        cx.observe_window_appearance(window, |this, _, cx| {
            if this.settings.appearance == AppearanceSetting::System {
                theme::apply(AppearanceSetting::System, cx);
            }
        })
        .detach();
        let (bridge, calls) = start_bridge().unzip();
        if let Some(calls) = calls {
            cx.spawn(async move |this, cx| {
                while let Ok(call) = calls.recv().await {
                    if this.update(cx, |this, cx| this.handle_hook(call, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        let roots = HashMap::from([(project.clone(), Self::root_views(&project, window, cx))]);
        // The editor panel can take the grid's room; the workbench follows its changes.
        let editor = cx.new(|_| EditorView::default());
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        // Review comments go to the agent working where the changes are.
        cx.subscribe_in(&editor, window, |this, _, event: &SendToAgent, window, cx| {
            this.send_to_agent(&event.repo, &event.message, window, cx);
        })
        .detach();
        // A change staged or reverted from the review: the list of changes reads again.
        cx.subscribe(&editor, |this, _, event: &Touched, cx| {
            if let Some(views) = this.roots.get(&event.repo) {
                views.changes.update(cx, ChangesView::reload);
            }
        })
        .detach();
        let handle = window.window_handle();
        let weak = cx.weak_entity();
        OPEN.with_borrow_mut(|open| open.push((handle, weak)));
        let status_bar = {
            let (editor, workspace) = (editor.clone(), cx.weak_entity());
            cx.new(|cx| StatusBar::new(editor, workspace, &project, cx))
        };
        let saved_settings = settings::load();
        Self {
            status_bar,
            tickets_view: None,
            sidebar_tab: SidebarTab::Files,
            sessions: HashMap::new(),
            started: HashMap::new(),
            program_checked: HashMap::new(),
            started_at: HashMap::new(),
            routine_tabs: HashMap::new(),
            teams: HashMap::new(),
            also_in: Vec::new(),
            links: HashMap::new(),
            clickup: ClickUpState::default(),
            checkouts: HashMap::new(),
            drop_target: None,
            pull_requests: HashMap::new(),
            checkout_refresh: Refresh::Idle,
            env_shared: HashSet::new(),
            servers: HashMap::new(),
            services: Vec::new(),
            reading_servers: false,
            ports: HashMap::new(),
            overlaps: std::collections::BTreeMap::new(),
            reading_overlaps: false,
            turns: HashMap::new(),
            transcripts: HashMap::new(),
            spent: HashMap::new(),
            waits: HashMap::new(),
            open_menu: None,
            menu_dismissed: None,
            review_notes: HashMap::new(),
            check_runs: HashMap::new(),
            roots,
            root: project.clone(),
            // Off to begin with: the sidebar stays on the repo until you ask it to follow panes.
            follow_pane: false,
            editor,
            projects: vec![project.clone()],
            page: project.clone(),
            project,
            layout: Layout::default(),
            pages: HashMap::new(),
            terminals: HashMap::new(),
            names: HashMap::new(),
            unseen: HashSet::new(),
            renaming: None,
            next_tab: 0,
            focus: cx.focus_handle(),
            panels: Panels { tickets: false, ..saved_settings.panels },
            settings: saved_settings,
            bridge,
            session_names: HashMap::new(),
            badge: 0,
            waiting: 0,
        }
    }

    /// Shows the editor panel. A maximised pane covers it, so the pane is restored first.
    fn show_editor(&mut self) {
        self.panels.editor = true;
        self.layout.maximized = None;
    }

    /// The sidebar's button and shortcut (⌘B). A maximised pane covers the sidebar, so asking for
    /// it then restores the pane and shows it, rather than toggling something unseen.
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.panels.sidebar = self.layout.maximized.take().is_some() || !self.panels.sidebar;
        self.remember_panels(cx);
        cx.notify();
    }

    /// The editor's button and shortcut (⌘J), the same way as the sidebar's.
    fn toggle_editor(&mut self, cx: &mut Context<Self>) {
        self.panels.editor = self.layout.maximized.take().is_some() || !self.panels.editor;
        self.remember_panels(cx);
        cx.notify();
    }

    /// Opens `path` in the editor, showing the editor if it was hidden.
    pub fn open_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| editor.open(path, window, cx));
        self.show_editor();
        cx.notify();
    }

    /// A tree and a changes list for `root`. A file picked in the tree opens in the editor; a new
    /// git state makes the changes list read again.
    fn root_views(root: &Path, window: &mut Window, cx: &mut Context<Self>) -> RootViews {
        let files = cx.new(|cx| FilesView::new(root, cx));
        let changes = cx.new(|cx| ChangesView::new(root, cx));
        let files_events = {
            let changes = changes.clone();
            cx.subscribe_in(&files, window, move |this, _, event: &FilesEvent, window, cx| match event {
                FilesEvent::Open(path) => this.open_file(path, window, cx),
                FilesEvent::GitChanged => {
                    changes.update(cx, ChangesView::reload);
                    this.status_bar.update(cx, StatusBar::refresh);
                }
                FilesEvent::Changed(paths) => {
                    this.editor.update(cx, |editor, cx| editor.files_changed(paths, window, cx));
                }
            })
        };
        let changes_events = Self::follow_changes(&changes, window, cx);
        // Whether there's a schema to draw is a walk of the folder: off the main thread.
        let folder = root.to_owned();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn({
                    let folder = folder.clone();
                    async move {
                        // Only when there's something to draw: SQL files alone (a dbt project's
                        // models) aren't a schema.
                        let schema = !saturn_core::schema::find_sources(&folder).is_empty()
                            && !saturn_core::schema::load(&folder).tables.is_empty();
                        (schema, saturn_core::dbt::find_projects(&folder).into_iter().next())
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(views) = this.roots.get_mut(&folder) {
                    (views.has_schema, views.dbt_project) = found;
                    cx.notify();
                }
            });
        })
        .detach();
        RootViews {
            files,
            changes,
            branch: git::branch(root),
            has_schema: false,
            dbt_project: None,
            _events: [files_events, changes_events],
        }
    }

    /// The views of the folder the sidebar shows.
    fn current_root(&self) -> &RootViews {
        self.roots.get(&self.root).or_else(|| self.roots.get(&self.project)).expect("the project's views exist")
    }

    /// Points the sidebar at the focused pane's worktree when following, else at the project.
    fn sync_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane_folder = self
            .follow_pane
            .then(|| self.layout.focused_tab().and_then(|tab| self.terminals.get(&tab)))
            .flatten()
            .map(|view| view.read(cx).cwd.clone());
        let target =
            pane_folder.map_or_else(|| self.project.clone(), |folder| git::toplevel(&folder).unwrap_or(folder));
        if let Some(owner) = self.project_of(&target) {
            self.project = owner;
        }
        if target == self.root {
            return;
        }
        if !self.roots.contains_key(&target) {
            let views = Self::root_views(&target, window, cx);
            self.roots.insert(target.clone(), views);
        }
        self.root = target;
        let root = self.root.clone();
        self.status_bar.update(cx, |bar, cx| bar.set_root(&root, cx));
        cx.notify();
    }

    /// Reviewing a change opens it in the editor panel; new changes refresh an open review.
    fn follow_changes(changes: &Entity<ChangesView>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_in(changes, window, |this, changes, event: &ChangesEvent, window, cx| match event {
            ChangesEvent::Open { files, scope, path } => {
                let repo = changes.read(cx).repo().to_owned();
                this.editor.update(cx, |editor, cx| editor.review(files.clone(), scope, path, &repo, cx));
                this.show_editor();
                cx.notify();
            }
            ChangesEvent::Updated { files, scope } => {
                this.editor.update(cx, |editor, cx| editor.refresh_review(files.clone(), scope.clone(), cx));
            }
            ChangesEvent::History { repo } => {
                this.editor.update(cx, |editor, cx| editor.show_history(repo, cx));
                this.show_editor();
                cx.notify();
            }
            ChangesEvent::Impact { repo } => {
                this.show_impact(repo, cx);
            }
            ChangesEvent::Compare { repo } => {
                crate::palette::compare(cx.weak_entity(), repo.clone(), window, cx);
            }
        })
    }

    /// Reviews everything in `repo` that differs from `target` (a branch, tag or commit).
    pub fn review_against(&mut self, repo: &Path, target: String, cx: &mut Context<Self>) {
        // The panel opens at once, so the review is seen arriving.
        self.show_editor();
        cx.notify();
        let (repo, scope) = (repo.to_owned(), git::Scope::Against(target));
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn({
                    let (repo, scope) = (repo.clone(), scope.clone());
                    async move { gpui_kit::component::diff::DiffFile::parse(&git::patch(&repo, scope)).unwrap_or_default() }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let path = files.first().map(|file| file.path().to_string()).unwrap_or_default();
                this.editor.update(cx, |editor, cx| editor.review(files, &scope, &path, &repo, cx));
            });
        })
        .detach();
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.settings.project(&self.project);
        settings_view::open(
            cx.weak_entity(),
            (project.ticket_command, project.codex_command),
            self.settings.agent_commands.clone(),
            window,
            cx,
        );
    }

    /// The folder the sidebar shows: the selected repo, or one of its worktrees.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The selected repo.
    pub fn project_path(&self) -> &Path {
        &self.project
    }

    /// The repo of this window that `folder` belongs to: the repo itself, a folder inside it, or
    /// one of its worktrees.
    fn project_of(&self, folder: &Path) -> Option<PathBuf> {
        if let Some(project) = self.projects.iter().find(|project| folder.starts_with(project)) {
            return Some(project.clone());
        }
        let main = git::main_worktree(folder)?;
        self.projects.iter().find(|project| **project == main).cloned()
    }

    /// Shows `project` in the sidebar and makes it where new tabs open. Choosing a repo by hand
    /// stops the sidebar following panes, so the choice sticks.
    pub fn select_project(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_pane = false;
        self.switch_to(project, window, cx);
        self.after_layout_change(window, cx);
    }

    /// Shows `project` in the sidebar, and its own grid in place of the one showing.
    fn switch_to(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.show_page(&project);
        self.project = project;
        if !self.roots.contains_key(&self.project) {
            let views = Self::root_views(&self.project, window, cx);
            self.roots.insert(self.project.clone(), views);
        }
        self.root = self.project.clone();
        let root = self.root.clone();
        self.status_bar.update(cx, |bar, cx| bar.set_root(&root, cx));
        cx.notify();
    }

    /// Puts the grid showing aside and shows `project`'s, empty if it has none yet.
    fn show_page(&mut self, project: &Path) {
        if self.page == project {
            return;
        }
        let grid = self.pages.remove(project).unwrap_or_default();
        let hidden = std::mem::replace(&mut self.layout, grid);
        if !hidden.is_empty() {
            self.pages.insert(self.page.clone(), hidden);
        }
        project.clone_into(&mut self.page);
        // Cells are numbered per grid: a menu or a drop meant for one isn't the other's.
        self.open_menu = None;
        self.drop_target = None;
    }

    /// Every tab of the window: the grid showing's, then the other repos'.
    fn all_tabs(&self) -> Vec<TabId> {
        self.layout.tabs().chain(self.pages.values().flat_map(Layout::tabs)).collect()
    }

    /// Adds a repo to the window (or selects it, if it's here already).
    pub fn add_project(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        settings::remember_project(&project);
        if !self.projects.contains(&project) {
            self.projects.push(project.clone());
        }
        self.select_project(project, window, cx);
        if self.layout.is_empty() {
            self.open_terminal(Placement::Tab, window, cx);
        }
        self.save_session(cx);
    }

    /// Takes a repo out of the window's list. Its panes join the grid showing, still running.
    fn remove_project(&mut self, project: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if self.projects.len() < 2 {
            return;
        }
        self.projects.retain(|p| p != project);
        if self.page == project || self.project == project {
            let first = self.projects[0].clone();
            self.switch_to(first, window, cx);
        }
        if let Some(left) = self.pages.remove(project) {
            for tab in left.tabs() {
                self.layout.add_tab(tab, None);
            }
        }
        self.after_layout_change(window, cx);
    }

    pub fn clickup(&self) -> &ClickUpState {
        &self.clickup
    }

    /// Checks `token` with ClickUp and, if it works, saves it.
    pub fn connect_clickup(&mut self, token: String, cx: &mut Context<Self>) {
        self.clickup.problem = None;
        self.clickup.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let checked = cx
                .background_spawn({
                    let token = token.clone();
                    async move { clickup::check(&token).and_then(|account| clickup::save_token(&token).map(|()| account)) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.clickup.loading = false;
                match checked {
                    Ok(account) => {
                        this.clickup.token = Some(token);
                        this.clickup.account = Some(account);
                        this.refresh_tickets(cx);
                    }
                    Err(problem) => this.clickup.problem = Some(problem),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn disconnect_clickup(&mut self, cx: &mut Context<Self>) {
        clickup::forget_token();
        self.clickup = ClickUpState::default();
        cx.notify();
    }

    /// Reads the open tickets again, off the main thread.
    pub fn refresh_tickets(&mut self, cx: &mut Context<Self>) {
        if self.clickup.made_up {
            return;
        }
        let Some(token) = self.clickup.token.clone() else { return };
        self.clickup.loading = self.clickup.tickets.is_empty();
        cx.spawn(async move |this, cx| {
            let tickets = cx.background_spawn(async move { clickup::my_tickets(&token) }).await;
            let _ = this.update(cx, |this, cx| {
                this.clickup.loading = false;
                match tickets {
                    Ok(tickets) => {
                        this.clickup.tickets = tickets;
                        this.clickup.problem = None;
                    }
                    Err(problem) => this.clickup.problem = Some(problem),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts an agent on `ticket` in a new pane: in its own worktree when the project asks
    /// Saturn to make one, running the project's ticket command.
    pub fn start_ticket(&mut self, ticket: &Ticket, agent: Agent, window: &mut Window, cx: &mut Context<Self>) {
        let make_worktree = self.settings.project(&self.project).create_worktree;
        self.start_work(ticket.key(), &ticket.title, agent, make_worktree, window, cx);
    }

    /// Starts an agent on work named by hand (`fix-login`), with no tracker: always in a worktree
    /// of its own, which is the point of naming it.
    pub fn start_named(&mut self, name: &str, agent: Agent, window: &mut Window, cx: &mut Context<Self>) {
        self.start_work(name, name, agent, true, window, cx);
    }

    /// Starts `agent` on a piece of work, `key` (a ticket's id or a name) with its `title`, in a new
    /// pane: in a worktree of its own when `make_worktree`, set up by the repo's recipe, running the
    /// project's command for that agent.
    fn start_work(
        &mut self,
        key: &str,
        title: &str,
        agent: Agent,
        make_worktree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.settings.project(&self.project);
        let mut problem = None;
        let (cwd, made) = if make_worktree {
            match git::work_worktree(&self.project, key) {
                Ok((dir, made)) => {
                    self.furnish_worktree(&dir, made);
                    (dir, made)
                }
                Err(why) => {
                    problem = Some(format!("No worktree for {key}, so the agent works in the repo itself: {why}"));
                    (self.project.clone(), false)
                }
            }
        } else {
            (self.project.clone(), false)
        };
        let Some(tab) = self.spawn_terminal(&cwd, window, cx) else { return };
        if self.layout.is_empty() {
            self.layout.add_tab(tab, None);
        } else {
            self.layout.split_right(tab, None);
        }
        if let Some(problem) = problem {
            self.note(tab, &problem, cx);
        }
        let mut command = project.command_for(agent, key, title);
        if make_worktree && cwd != self.project {
            let folders = self.also_in_worktrees(key, tab, cx);
            command = multi_repo::with_folders(&command, &folders, settings::shell_quote);
        }
        // A worktree made just now has the code and nothing else: the recipe's setup runs first,
        // and the agent starts only once it has worked.
        if made && let Some(setup) = self.setup_for(tab, &cwd, cx) {
            command = format!("{setup} && {command}");
        }
        self.type_into(tab, &command, cx);
        self.after_layout_change(window, cx);
    }

    /// Starts a routine for its time `due`: a pane in its folder, named after it, where its agent
    /// starts with its prompt. Its session is tied to the run once a hook tells it.
    pub fn start_routine(&mut self, routine: &Routine, due: i64, window: &mut Window, cx: &mut Context<Self>) {
        let there = routine.folder.is_dir();
        let folder = if there { routine.folder.clone() } else { self.project.clone() };
        let Some(tab) = self.spawn_terminal(&folder, window, cx) else { return };
        self.place_new(tab);
        self.names.insert(tab, routine.name.clone());
        if there {
            self.routine_tabs.insert(tab, (routine.id.clone(), due));
            self.type_into(tab, &routine.agent.start(Some(&routine.prompt), settings::shell_quote), cx);
            notify::post(&routine.id, &routine.name, "The routine started.", false);
        } else {
            let gone = routine.folder.display();
            self.note(tab, &format!("{} didn't start: {gone} isn't there any more.", routine.name), cx);
        }
        self.after_layout_change(window, cx);
    }

    /// Reads the team each pane's session leads, off the main thread.
    fn refresh_teams(&mut self, cx: &mut Context<Self>) {
        if self.sessions.is_empty() && self.teams.is_empty() {
            return;
        }
        // Agent teams are Claude Code's.
        let sessions: Vec<(TabId, String)> = self
            .sessions
            .iter()
            .filter(|(_, (agent, _))| *agent == Agent::Claude)
            .map(|(tab, (_, id))| (*tab, id.clone()))
            .collect();
        cx.spawn(async move |this, cx| {
            let teams = cx
                .background_spawn(async move {
                    let Some(home) = saturn_core::prompts::claude_home() else { return HashMap::new() };
                    sessions
                        .into_iter()
                        .filter_map(|(tab, session)| Some((tab, teams::for_session(&home, &session)?)))
                        .collect::<HashMap<_, _>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.teams != teams {
                    this.teams = teams;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The team a pane's session leads, as last read.
    pub fn team(&self, tab: TabId) -> Option<&Team> {
        self.teams.get(&tab)
    }

    /// Resumes an agent's session in a new pane in the folder it ran in.
    pub fn resume_session(
        &mut self,
        folder: &Path,
        agent: Agent,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = crate::session::resume_command(agent, id) else { return };
        let Some(tab) = self.spawn_terminal(folder, window, cx) else { return };
        self.place_new(tab);
        self.type_into(tab, &command, cx);
        self.after_layout_change(window, cx);
    }

    /// The window's other repos, for New agent's "Also in".
    pub fn other_repos(&self) -> Vec<PathBuf> {
        self.projects.iter().filter(|repo| **repo != self.project).cloned().collect()
    }

    /// The other repos the next agent also works in.
    pub fn also_in(&self) -> &[PathBuf] {
        &self.also_in
    }

    pub fn toggle_also_in(&mut self, repo: &Path, cx: &mut Context<Self>) {
        if let Some(index) = self.also_in.iter().position(|picked| picked == repo) {
            self.also_in.remove(index);
        } else {
            self.also_in.push(repo.to_owned());
        }
        cx.notify();
    }

    /// Makes the work's worktree in each repo picked in "Also in", for the agent in `tab` to reach,
    /// and says in the pane what it reaches. The pick is used up.
    fn also_in_worktrees(&mut self, key: &str, tab: TabId, cx: &mut Context<Self>) -> Vec<PathBuf> {
        let repos: Vec<PathBuf> =
            std::mem::take(&mut self.also_in).into_iter().filter(|repo| self.projects.contains(repo)).collect();
        let mut folders = Vec::new();
        for repo in repos {
            match git::work_worktree(&repo, key) {
                Ok((dir, made)) => {
                    self.furnish_worktree(&dir, made);
                    folders.push(dir);
                }
                Err(why) => {
                    let name = repo.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    self.note(tab, &format!("No worktree for {key} in {name}: {why}"), cx);
                }
            }
        }
        if !folders.is_empty() {
            let names: Vec<String> = folders.iter().map(|dir| dir.display().to_string()).collect();
            self.note(tab, &format!("The agent also works in {}.", names.join(", ")), cx);
        }
        folders
    }

    /// Finds, for each pane in a worktree, the same work in the window's other repos, off the main
    /// thread.
    fn refresh_links(&mut self, cx: &mut Context<Self>) {
        if self.projects.len() < 2 {
            if !self.links.is_empty() {
                self.links.clear();
                cx.notify();
            }
            return;
        }
        let repos = self.projects.clone();
        let checkouts: Vec<(TabId, PathBuf, String)> = self
            .checkouts
            .iter()
            .map(|(tab, checkout)| (*tab, checkout.root.clone(), checkout.branch.clone()))
            .collect();
        cx.spawn(async move |this, cx| {
            let links = cx
                .background_spawn(async move {
                    checkouts
                        .into_iter()
                        .filter(|(_, root, _)| git::main_worktree(root).is_some_and(|main| main != *root))
                        .filter_map(|(tab, root, branch)| {
                            let found: Vec<(PathBuf, String)> = multi_repo::linked(&branch, &repos, &root)
                                .into_iter()
                                .map(|dir| {
                                    let repo = git::main_worktree(&dir).unwrap_or_else(|| dir.clone());
                                    let name = repo.file_name().map(|name| name.to_string_lossy().into_owned());
                                    (dir, name.unwrap_or_default())
                                })
                                .collect();
                            (!found.is_empty()).then_some((tab, found))
                        })
                        .collect::<HashMap<_, _>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.links != links {
                    this.links = links;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The pane's linked worktrees in other repos, as chips.
    fn render_links(&self, cell: &Cell, cx: &mut Context<Self>) -> Vec<impl IntoElement + use<>> {
        let cell_id = cell.id;
        let mut chips = Vec::new();
        for (index, (folder, repo)) in self.links.get(&cell.active).into_iter().flatten().enumerate() {
            chips.push(link_chip(cell_id, index, folder, repo, cx));
        }
        chips
    }

    /// Goes to a pane working in `folder`, or opens one there.
    fn go_to_folder(&mut self, folder: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.attempt_panes(&[folder.to_owned()], cx).into_iter().flatten().next();
        if let Some((tab, _)) = existing {
            self.go_to_tab(tab, window, cx);
            return;
        }
        if let Some(tab) = self.spawn_terminal(folder, window, cx) {
            self.place_new(tab);
            self.after_layout_change(window, cx);
        }
    }

    /// Brings into a worktree what git leaves out, before an agent starts there: the shared env
    /// files, and (once, when it was just made) what `.worktreeinclude` lists.
    fn furnish_worktree(&mut self, dir: &Path, made: bool) {
        if self.settings.share_env_files {
            shared_env::share(dir);
            self.env_shared.insert(dir.to_owned());
        }
        // After the links just made, so it copies around them.
        if made {
            worktree_include::copy_into(dir);
        }
    }

    /// Best of N: makes a worktree for each of `count` attempts at `task` (off the main thread),
    /// then opens a pane in each that sets it up and starts Claude with the task.
    pub fn start_attempts(&mut self, task: String, agents: Vec<Agent>, window: &mut Window, cx: &mut Context<Self>) {
        let repo = git::main_worktree(&self.project).unwrap_or_else(|| self.project.clone());
        let names = best_of::work_names(&task, agents.len());
        cx.spawn_in(window, async move |this, cx| {
            let made = cx
                .background_spawn(async move {
                    names.iter().map(|name| git::work_worktree(&repo, name)).collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| this.open_attempts(&task, &agents, made, window, cx));
        })
        .detach();
    }

    fn open_attempts(
        &mut self,
        task: &str,
        agents: &[Agent],
        made: Vec<Result<(PathBuf, bool), String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (number, attempt) in made.into_iter().enumerate() {
            let (dir, fresh, problem) = match attempt {
                Ok((dir, fresh)) => (dir, fresh, None),
                Err(why) => (self.project.clone(), false, Some(why)),
            };
            if problem.is_some() {
                // Without its own worktree an attempt would trample the others' work.
                if let Some(tab) = self.spawn_terminal(&dir, window, cx) {
                    self.place_new(tab);
                    let why = problem.unwrap_or_default();
                    self.note(tab, &format!("Attempt {} has no worktree, so it didn't start: {why}", number + 1), cx);
                }
                continue;
            }
            self.furnish_worktree(&dir, fresh);
            let Some(tab) = self.spawn_terminal(&dir, window, cx) else { continue };
            self.place_new(tab);
            let agent = agents.get(number).copied().unwrap_or(Agent::Claude);
            let mut command = agent.start(Some(task), settings::shell_quote);
            if fresh && let Some(setup) = self.setup_for(tab, &dir, cx) {
                command = format!("{setup} && {command}");
            }
            self.type_into(tab, &command, cx);
        }
        self.after_layout_change(window, cx);
    }

    /// Puts a new pane into the grid, to the right of the focused one.
    fn place_new(&mut self, tab: TabId) {
        if self.layout.is_empty() {
            self.layout.add_tab(tab, None);
        } else {
            self.layout.split_right(tab, None);
        }
    }

    /// Where each attempt's agent is: its pane (the first working in its folder) and that pane's state.
    /// The prompts typed in a tab's agent session, in order: from Claude Code's transcript, or
    /// Codex's rollout.
    fn typed_prompts(&self, tab: TabId) -> Vec<String> {
        match self.sessions.get(&tab) {
            Some((Agent::Codex, id)) => Agent::Codex
                .home()
                .map(|home| saturn_core::codex_sessions::typed_prompts(&home, id))
                .unwrap_or_default(),
            _ => self
                .transcripts
                .get(&tab)
                .and_then(|path| std::fs::File::open(path).ok())
                .map(|file| {
                    saturn_core::prompts::typed_prompts(
                        std::io::BufRead::lines(std::io::BufReader::new(file)).map_while(Result::ok),
                    )
                    .into_iter()
                    .map(|asked| asked.prompt)
                    .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The agent a tab runs, as its session's hooks say, else as Saturn started it.
    pub fn agent_in(&self, tab: TabId) -> Option<Agent> {
        self.sessions.get(&tab).map(|(agent, _)| *agent).or_else(|| self.started.get(&tab).copied())
    }

    pub fn attempt_panes(&self, folders: &[PathBuf], cx: &gpui_kit::App) -> Vec<Option<(TabId, AgentState)>> {
        folders
            .iter()
            .map(|folder| {
                let folder = folder.canonicalize().unwrap_or_else(|_| folder.clone());
                self.all_tabs().into_iter().find_map(|tab| {
                    let view = self.terminals.get(&tab)?.read(cx);
                    let cwd = view.cwd.canonicalize().unwrap_or_else(|_| view.cwd.clone());
                    cwd.starts_with(&folder).then_some((tab, view.state))
                })
            })
            .collect()
    }

    /// Keeps one attempt: closes the panes working in the others and archives their worktrees, so
    /// any of them can still be restored from Worktrees.
    pub fn keep_attempt(&mut self, others: Vec<Worktree>, cx: &mut Context<Self>) {
        let folders: Vec<PathBuf> = others.iter().map(|worktree| worktree.path.clone()).collect();
        for (tab, _) in self.attempt_panes(&folders, cx).into_iter().flatten() {
            if let Some(view) = self.terminals.get(&tab) {
                view.update(cx, |view, _| view.kill());
            }
        }
        let repo = git::main_worktree(&self.project).unwrap_or_else(|| self.project.clone());
        cx.background_spawn(async move {
            for worktree in &others {
                let _ = crate::worktrees_view::archive_now(&repo, worktree);
            }
        })
        .detach();
    }

    /// Gives `message` to the agent working in `checkout`: the focused pane when its agent works there,
    /// else another pane whose does. Only a pane running an agent: pasted into a shell, the message
    /// (and any code quoted in it) would run as commands. With none, a new pane starts Claude with the
    /// message as its first. The pane comes to the front, so you see it land.
    pub fn send_to_agent(&mut self, checkout: &Path, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        let checkout = checkout.canonicalize().unwrap_or_else(|_| checkout.to_owned());
        let works_there = |view: &Entity<TerminalView>, cx: &gpui_kit::App| {
            let cwd = view.read(cx).cwd.canonicalize().unwrap_or_else(|_| view.read(cx).cwd.clone());
            cwd.starts_with(&checkout)
        };
        let runs_an_agent = |view: &Entity<TerminalView>, cx: &gpui_kit::App| {
            view.read(cx).foreground_pid().and_then(process_name).is_some_and(|name| AGENTS.contains(&name.as_str()))
        };
        let agents: Vec<TabId> = self
            .all_tabs()
            .into_iter()
            .filter(|tab| self.terminals.get(tab).is_some_and(|view| works_there(view, cx) && runs_an_agent(view, cx)))
            .collect();
        let target = self.layout.focused_tab().filter(|tab| agents.contains(tab)).or_else(|| agents.first().copied());
        if let Some(tab) = target {
            if let Some(view) = self.terminals.get(&tab) {
                view.update(cx, |view, _| {
                    view.paste(message);
                    view.write(b"\r".to_vec());
                });
            }
            self.go_to_tab(tab, window, cx);
        } else {
            let Some(tab) = self.spawn_terminal(&checkout, window, cx) else { return };
            if self.layout.is_empty() {
                self.layout.add_tab(tab, None);
            } else {
                self.layout.split_right(tab, None);
            }
            let agent = crate::agent_choice::current(cx);
            self.type_into(tab, &agent.start(Some(message), settings::shell_quote), cx);
            self.after_layout_change(window, cx);
        }
    }

    /// The recipe's setup for `checkout`, saying in `tab` when it was guessed or the recipe is wrong.
    fn setup_for(&self, tab: TabId, checkout: &Path, cx: &mut Context<Self>) -> Option<String> {
        match recipe::for_checkout(checkout) {
            Ok((recipe, source)) => {
                let setup = recipe.setup_command()?;
                if source == Source::Guessed {
                    self.note(tab, &guessed(), cx);
                }
                Some(setup)
            }
            Err(problem) => {
                self.note(tab, &format!("The worktree recipe wasn't run: {problem}"), cx);
                None
            }
        }
    }

    /// Opens a pane in a worktree just restored from the archive: its shared files back in place, and
    /// its setup run, since what git ignores (`node_modules`) wasn't kept.
    pub fn open_restored(&mut self, checkout: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.share_env_files {
            shared_env::share(checkout);
            self.env_shared.insert(checkout.to_owned());
        }
        worktree_include::copy_into(checkout);
        let Some(tab) = self.spawn_terminal(checkout, window, cx) else { return };
        if self.layout.is_empty() {
            self.layout.add_tab(tab, None);
        } else {
            self.layout.split_right(tab, None);
        }
        if let Some(setup) = self.setup_for(tab, checkout, cx) {
            self.type_into(tab, &setup, cx);
        }
        self.after_layout_change(window, cx);
    }

    /// Types `command` into a new tab's shell and runs it, once the shell has drawn its prompt.
    fn type_into(&mut self, tab: TabId, command: &str, cx: &mut Context<Self>) {
        let Some(view) = self.terminals.get(&tab).cloned() else { return };
        // The agent it starts, after any setup that runs first (`setup && codex '…'`), read before
        // it's started with the person's own command, which Saturn may not recognise.
        if let Some(agent) = command.rsplit("&& ").next().and_then(Agent::of_command) {
            self.started.insert(tab, agent);
            self.started_at.insert(tab, crate::worktrees_view::now());
        }
        let command = crate::agent_choice::as_typed(command, cx);
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
            let () = view.update(cx, |view, _| view.write(format!("{command}\r").into_bytes()));
        })
        .detach();
    }

    /// Shows a faint line from Saturn in a tab.
    fn note(&self, tab: TabId, text: &str, cx: &mut Context<Self>) {
        if let Some(view) = self.terminals.get(&tab) {
            view.update(cx, |view, cx| view.note(text, cx));
        }
    }

    /// The repo or worktree the focused pane works in.
    fn focused_checkout(&self, cx: &gpui_kit::App) -> PathBuf {
        let Some(tab) = self.layout.focused_tab() else { return self.project.clone() };
        self.checkouts
            .get(&tab)
            .map(|checkout| checkout.root.clone())
            .or_else(|| self.terminals.get(&tab).and_then(|view| git::toplevel(&view.read(cx).cwd)))
            .unwrap_or_else(|| self.project.clone())
    }

    /// A new tab in the focused pane, at the top of `checkout`, named `name`.
    fn recipe_tab(
        &mut self,
        checkout: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        let tab = self.spawn_terminal(checkout, window, cx)?;
        self.names.insert(tab, name.to_owned());
        self.layout.add_tab(tab, None);
        self.after_layout_change(window, cx);
        Some(tab)
    }

    /// Runs the recipe's setup in the focused pane's checkout, in a tab of its own. Safe to run again:
    /// for a worktree made some other way, or after the recipe changed.
    fn set_up_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let checkout = self.focused_checkout(cx);
        worktree_include::copy_into(&checkout);
        let Some(tab) = self.recipe_tab(&checkout, "Setup", window, cx) else { return };
        match recipe::for_checkout(&checkout) {
            Ok((recipe, source)) => match (recipe.setup_command(), source) {
                (Some(setup), source) => {
                    if source == Source::Guessed {
                        self.note(tab, &guessed(), cx);
                    }
                    self.type_into(tab, &setup, cx);
                }
                (None, Source::File) => self.note(tab, &format!("{} has no \"setup\" steps.", recipe::FILE), cx),
                (None, Source::Guessed) => self.note(tab, &no_recipe(), cx),
            },
            Err(problem) => self.note(tab, &problem, cx),
        }
    }

    /// Starts each of the recipe's `run` commands in a tab of its own, named after it.
    fn start_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let checkout = self.focused_checkout(cx);
        let note = match recipe::for_checkout(&checkout) {
            Ok((recipe, source)) if !recipe.run.is_empty() => {
                for (index, (name, command)) in recipe.run.into_iter().enumerate() {
                    let Some(tab) = self.recipe_tab(&checkout, &name, window, cx) else { continue };
                    if index == 0 && source == Source::Guessed {
                        self.note(tab, &guessed(), cx);
                    }
                    self.type_into(tab, &command, cx);
                }
                return;
            }
            Ok((_, Source::File)) => format!("{} has no \"run\" commands.", recipe::FILE),
            Ok((_, Source::Guessed)) => no_recipe(),
            Err(problem) => problem,
        };
        if let Some(tab) = self.recipe_tab(&checkout, "Start", window, cx) {
            self.note(tab, &note, cx);
        }
    }

    /// Lists the selected repo's worktrees, to see what's finished and remove it.
    fn show_worktrees(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = git::main_worktree(&self.project).unwrap_or_else(|| self.project.clone());
        let pane_folders = self.terminals.values().map(|view| view.read(cx).cwd.clone()).collect();
        crate::worktrees_view::open(cx.weak_entity(), repo, pane_folders, window, cx);
    }

    /// Writes the focused checkout's guessed recipe to `.saturn/worktree.json`, to adjust and commit,
    /// and opens it. A recipe that's already there just opens.
    fn create_recipe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let checkout = self.focused_checkout(cx);
        let path = checkout.join(recipe::FILE);
        if !path.exists() {
            let text = recipe::guess(&checkout).to_json();
            let written =
                path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, text));
            if let Err(error) = written {
                if let Some(tab) = self.recipe_tab(&checkout, "Recipe", window, cx) {
                    self.note(tab, &format!("Couldn't write {}: {error}", path.display()), cx);
                }
                return;
            }
        }
        self.open_file(&path, window, cx);
    }

    /// Reads the branch each tab is on and the pull request each tab works on, off the main thread.
    pub fn refresh_checkouts(&mut self, cx: &mut Context<Self>) {
        if self.checkout_refresh != Refresh::Idle {
            self.checkout_refresh = Refresh::RunningStale;
            return;
        }
        self.checkout_refresh = Refresh::Running;
        let tabs: Vec<TabSources> = self
            .terminals
            .iter()
            .map(|(tab, view)| {
                let view = view.read(cx);
                let names = [self.names.get(tab), self.session_names.get(tab), view.title.as_ref()];
                TabSources {
                    tab: *tab,
                    dir: view.cwd.clone(),
                    link: view.pull_request_link.clone(),
                    ticket: names.into_iter().flatten().find_map(|name| pull_request::ticket_in(name)),
                }
            })
            .collect();
        cx.spawn(async move |this, cx| {
            let (checkouts, pull_requests, notes) = cx.background_spawn(async move { read_checkouts(tabs) }).await;
            let _ = this.update(cx, |this, cx| {
                let stale = this.checkout_refresh == Refresh::RunningStale;
                this.checkout_refresh = Refresh::Idle;
                this.checkouts = checkouts;
                this.share_env_files(cx);
                this.refresh_overlaps(cx);
                this.refresh_links(cx);
                // Made-up agents keep their made-up pull requests.
                if std::env::var_os("SATURN_FAKE_AGENTS").is_none() {
                    this.pull_requests = pull_requests;
                    this.review_notes = notes;
                }
                cx.notify();
                if stale {
                    this.refresh_checkouts(cx);
                }
            });
        })
        .detach();
    }

    /// Reads the servers each tab runs and the repos' Docker services, off the main thread.
    fn refresh_servers(&mut self, cx: &mut Context<Self>) {
        if self.reading_servers {
            return;
        }
        self.reading_servers = true;
        let shells: Vec<(TabId, u32, Option<PathBuf>)> = self
            .terminals
            .iter()
            .filter_map(|(tab, view)| {
                let shell = view.read(cx).shell_pid()?;
                Some((*tab, shell, self.checkouts.get(tab).map(|checkout| checkout.root.clone())))
            })
            .collect();
        let projects = self.projects.clone();
        cx.spawn(async move |this, cx| {
            let (servers, services) = cx
                .background_spawn(async move {
                    let by_shell = servers::read(
                        &shells.iter().map(|(_, shell, root)| (*shell, root.clone())).collect::<Vec<_>>(),
                    );
                    let servers: HashMap<TabId, Vec<Server>> = shells
                        .iter()
                        .filter_map(|(tab, shell, _)| Some((*tab, by_shell.get(shell)?.clone())))
                        .collect();
                    // Only this window's repos' services, run from the repo or one of its worktrees.
                    let services: Vec<Service> = servers::docker_services()
                        .into_iter()
                        .filter(|service| {
                            projects.contains(&service.folder)
                                || git::main_worktree(&service.folder).is_some_and(|main| projects.contains(&main))
                        })
                        .collect();
                    (servers, services)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.reading_servers = false;
                if this.servers != servers || this.services != services {
                    this.servers = servers;
                    this.services = services;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Saves the files when a tab's agent starts a turn and when it ends it, so the turn can be undone.
    /// Asking for a permission is part of a turn; it ends when the agent is done or idle.
    fn follow_turn(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let Some(state) = self.terminals.get(&tab).map(|view| view.read(cx).state) else { return };
        let in_turn = self.turns.get(&tab).is_some_and(|turn| turn.after.is_none());
        let starts = state == AgentState::Working && !in_turn;
        let ends =
            in_turn && matches!(state, AgentState::YourTurn | AgentState::Idle | AgentState::Error | AgentState::Shell);
        if !starts && !ends {
            return;
        }
        let checkout = if starts {
            self.checkouts
                .get(&tab)
                .map(|checkout| checkout.root.clone())
                .or_else(|| self.terminals.get(&tab).and_then(|view| git::toplevel(&view.read(cx).cwd)))
        } else {
            self.turns.get(&tab).map(|turn| turn.checkout.clone())
        };
        let Some(checkout) = checkout else { return };
        if starts {
            self.turns.insert(
                tab,
                Turn {
                    checkout: checkout.clone(),
                    before: None,
                    after: None,
                    files: Vec::new(),
                    undone: false,
                    busy: false,
                    said: None,
                },
            );
        }
        let name = format!("turns/{}/{}", tab.0, if starts { "before" } else { "after" });
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn({
                    let checkout = checkout.clone();
                    async move { snapshot::take(&checkout, &name).ok() }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(turn) = this.turns.get_mut(&tab) else { return };
                if starts {
                    turn.before = saved;
                } else {
                    turn.files = match (&turn.before, &saved) {
                        (Some(before), Some(after)) => snapshot::changed_between(&turn.checkout, before, after),
                        _ => Vec::new(),
                    };
                    // A turn that ends without its start saved has nothing to go back to.
                    turn.after = saved.or_else(|| Some(Snapshot { commit: String::new(), tree: String::new() }));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Puts the files of a tab's agent's last turn back as they were before it (or, once undone, as
    /// it left them). Refused when any of them changed since, so no later work is lost.
    pub fn undo_turn(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let Some(turn) = self.turns.get_mut(&tab) else { return };
        let (Some(before), Some(after)) = (turn.before.clone(), turn.after.clone()) else { return };
        if turn.busy || turn.files.is_empty() {
            return;
        }
        turn.busy = true;
        let (checkout, files, undoing) = (turn.checkout.clone(), turn.files.clone(), !turn.undone);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move {
                    // Where the files should be now: as the turn left them, or as the undo did.
                    let (expected, target) = if undoing { (&after, &before) } else { (&before, &after) };
                    let now = snapshot::take(&checkout, &format!("turns/{}/now", tab.0))?;
                    let moved: Vec<String> = snapshot::changed_between(&checkout, expected, &now)
                        .into_iter()
                        .filter(|file| files.contains(file))
                        .collect();
                    if !moved.is_empty() {
                        return Err(format!("{} changed since, so nothing was put back", moved.join(", ")));
                    }
                    snapshot::put_back(&checkout, expected, target)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(turn) = this.turns.get_mut(&tab) else { return };
                turn.busy = false;
                turn.said = Some(match &done {
                    Ok(count) if undoing => {
                        format!("Undid the last turn: {count} file{} as before", if *count == 1 { "" } else { "s" })
                    }
                    Ok(_) => "Redid the last turn".to_owned(),
                    Err(problem) => problem.clone(),
                });
                if done.is_ok() {
                    turn.undone = undoing;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The agents waiting for you, for the list: what each waits for, where it works.
    pub fn waiting_rows(&self, cx: &gpui_kit::App) -> Vec<crate::queue_view::Row> {
        self.layout
            .tabs()
            .filter_map(|tab| {
                let view = self.terminals.get(&tab)?.read(cx);
                if !view.state.asks_for_you() {
                    return None;
                }
                Some(crate::queue_view::Row {
                    tab,
                    name: self.tab_name(tab, cx),
                    place: self.place(tab, cx),
                    state: view.state,
                    waiting: self.waits.get(&tab).cloned(),
                })
            })
            .collect()
    }

    /// Every pane, for Send to several agents: where it works, and what a message would do there.
    pub fn broadcast_panes(&self, cx: &gpui_kit::App) -> Vec<crate::broadcast_view::Pane> {
        use crate::broadcast_view::{Pane, Reach};
        self.layout
            .tabs()
            .filter_map(|tab| {
                let view = self.terminals.get(&tab)?.read(cx);
                let program = view.foreground_pid().and_then(process_name)?;
                // A login shell is named with a dash in front, `-zsh`.
                let program = program.trim_start_matches('-');
                let reach = if AGENTS.contains(&program) {
                    Reach::Agent
                } else if SHELLS.contains(&program) {
                    Reach::Shell
                } else {
                    Reach::Busy(program.to_owned())
                };
                Some(Pane { tab, name: self.tab_name(tab, cx), place: self.place(tab, cx), reach })
            })
            .collect()
    }

    /// Sends one message to several panes, read again now in case one changed: typed into the
    /// agent, or starting Claude with it where there's only a shell. Busy panes are left alone.
    pub fn send_to_panes(&mut self, tabs: &[TabId], message: &str, cx: &mut Context<Self>) {
        use crate::broadcast_view::Reach;
        for pane in self.broadcast_panes(cx) {
            if !tabs.contains(&pane.tab) {
                continue;
            }
            let Some(view) = self.terminals.get(&pane.tab) else { continue };
            match pane.reach {
                Reach::Agent => view.update(cx, |view, _| {
                    view.paste(message);
                    view.write(b"\r".to_vec());
                }),
                Reach::Shell => {
                    let command = crate::agent_choice::current(cx).start(Some(message), settings::shell_quote);
                    let command = format!("{}\r", crate::agent_choice::as_typed(&command, cx));
                    view.update(cx, |view, _| view.write(command.into_bytes()));
                }
                Reach::Busy(_) => {}
            }
        }
        cx.notify();
    }

    /// Where a tab works, for lists: its folder, and its branch when known.
    fn place(&self, tab: TabId, cx: &gpui_kit::App) -> String {
        let folder = self
            .terminals
            .get(&tab)
            .and_then(|view| view.read(cx).cwd.file_name().map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_default();
        match self.checkouts.get(&tab) {
            Some(checkout) => format!("{folder} · {}", checkout.branch),
            None => folder,
        }
    }

    /// Answers the permission a tab's agent asks for, by pressing what you would in the pane:
    /// Return for Claude's highlighted "Yes", Escape for "No". Only while it really asks.
    pub fn answer_permission(&mut self, tab: TabId, allow: bool, cx: &mut Context<Self>) -> bool {
        let asks = matches!(self.waits.get(&tab), Some(Waiting::Permission { .. }));
        let Some(view) = self.terminals.get(&tab).cloned() else { return false };
        let program = view.read(cx).foreground_pid().and_then(process_name);
        // Claude Code and Codex answer with their own keys; another agent isn't pressed for.
        let Some(agent) = program.as_deref().and_then(Agent::of_program) else { return false };
        if !asks || view.read(cx).state != AgentState::NeedsYou {
            return false;
        }
        view.update(cx, |view, _| view.write(agent.answer_keys(allow).to_vec()));
        self.waits.remove(&tab);
        cx.notify();
        true
    }

    /// Brings a tab to the front and gives it the keyboard, showing its repo's grid if another shows.
    pub fn go_to_tab(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.pages.iter().find(|(_, grid)| grid.tabs().any(|t| t == tab)).map(|(page, _)| page.clone());
        if let Some(page) = page {
            self.switch_to(page, window, cx);
        }
        self.update_layout(window, cx, |layout| layout.activate_tab(tab));
    }

    /// Reads what each tab's session used in the plan's current 5-hour window, off the main thread.
    fn refresh_spent(&mut self, cx: &mut Context<Self>) {
        let since = crate::usage::window_start(cx);
        let transcripts: Vec<(TabId, PathBuf)> =
            self.transcripts.iter().map(|(tab, path)| (*tab, path.clone())).collect();
        cx.spawn(async move |this, cx| {
            let spent = cx
                .background_spawn(async move {
                    transcripts
                        .into_iter()
                        .filter_map(|(tab, path)| {
                            let file = std::fs::File::open(path).ok()?;
                            let lines = std::io::BufRead::lines(std::io::BufReader::new(file)).map_while(Result::ok);
                            Some((tab, session_usage::read(lines, Some(since))))
                        })
                        .collect::<HashMap<TabId, Tokens>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.spent = spent;
                this.write_shares(cx);
            });
        })
        .detach();
    }

    /// Writes each pane's session's share of the 5-hour limit, estimated from its part of what all
    /// the panes' sessions used (or its tokens when the plan has no limits to share), for Saturn's
    /// status line to show under Claude's prompt. Off the main thread.
    fn write_shares(&self, cx: &mut Context<Self>) {
        let Some(path) = crate::status_line::shares_path() else { return };
        let all: u64 = self.spent.values().map(Tokens::counted).sum();
        let percent = crate::usage::session_percent(cx);
        let shares: Vec<(String, String)> = self
            .spent
            .iter()
            .filter(|(_, tokens)| tokens.counted() > 0)
            .filter_map(|(tab, tokens)| {
                // Only Claude Code runs Saturn's status line.
                let session = self.sessions.get(tab).filter(|(agent, _)| *agent == Agent::Claude)?.1.clone();
                #[expect(clippy::cast_precision_loss, reason = "a share shown as a whole percentage")]
                let share = tokens.counted() as f64 / all.max(1) as f64;
                let label = match percent {
                    Some(percent) => format!("≈{:.0}%", (share * percent).max(1.)),
                    None => format!("{} tokens", session_usage::short(tokens.counted())),
                };
                Some((session, label))
            })
            .collect();
        let now = crate::worktrees_view::now();
        cx.background_spawn(async move {
            let _ = std::fs::write(path, saturn_core::status_line::shares_file(&shares, now));
        })
        .detach();
    }

    /// Finds the panes' checkouts that change the same files as another of their repository's, off the
    /// main thread: after the branches are read, and when an agent finishes a turn.
    fn refresh_overlaps(&mut self, cx: &mut Context<Self>) {
        if self.reading_overlaps {
            return;
        }
        let roots: std::collections::BTreeSet<PathBuf> =
            self.checkouts.values().map(|checkout| checkout.root.clone()).collect();
        if roots.len() < 2 {
            if !self.overlaps.is_empty() {
                self.overlaps.clear();
                cx.notify();
            }
            return;
        }
        self.reading_overlaps = true;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    let changed = roots.into_iter().map(|root| (root.clone(), overlap::changed_files(&root))).collect();
                    overlap::overlaps(&changed, git::main_worktree)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.reading_overlaps = false;
                if this.overlaps != found {
                    this.overlaps = found;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The repos' Docker Compose services running with ports, for the bottom bar.
    pub fn services(&self) -> &[Service] {
        &self.services
    }

    /// Links the main checkout's env files into each worktree a tab is in, once a session, also
    /// worktrees an agent or a script made.
    fn share_env_files(&mut self, cx: &mut Context<Self>) {
        if !self.settings.share_env_files {
            return;
        }
        let new: Vec<PathBuf> = self
            .checkouts
            .values()
            .map(|checkout| checkout.root.clone())
            .filter(|root| self.env_shared.insert(root.clone()))
            .collect();
        if !new.is_empty() {
            cx.background_spawn(async move {
                for root in new {
                    shared_env::share(&root);
                }
            })
            .detach();
        }
    }

    /// Makes the text of terminals and files bigger (`by` 1), smaller (−1), or its usual size
    /// (`None`), and remembers it.
    pub fn change_text_size(&mut self, by: Option<i16>, cx: &mut Context<Self>) {
        let size = match by {
            None => settings::TEXT_SIZE,
            Some(by) => {
                let next = i32::from(self.settings.text_size) + i32::from(by);
                let next = u16::try_from(next).unwrap_or(settings::TEXT_SIZE);
                next.clamp(*settings::TEXT_SIZES.start(), *settings::TEXT_SIZES.end())
            }
        };
        if size != self.settings.text_size {
            self.change_settings(|settings| settings.text_size = size, cx);
        }
        theme::set_text_size(size, cx);
        for view in self.terminals.values() {
            view.update(cx, |_, cx| cx.notify());
        }
    }

    /// Saves which panels are showing, so the next window opens with the same. Only when they
    /// changed: the settings file is written by every window.
    fn remember_panels(&mut self, cx: &mut Context<Self>) {
        let panels = self.panels;
        if self.settings.panels != panels {
            self.change_settings(|settings| settings.panels = panels, cx);
        }
    }

    /// Opens or closes the Tickets panel, reading the tickets again as it opens. A maximised pane
    /// covers it, so the pane is restored and the panel shows.
    pub fn toggle_tickets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.maximized.take().is_some() || !self.panels.tickets {
            self.open_tickets(window, cx);
        } else {
            self.panels.tickets = false;
        }
        self.remember_panels(cx);
        cx.notify();
    }

    /// Shows the Tickets panel, made the first time, and reads the tickets again.
    fn open_tickets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panels.tickets = true;
        if self.tickets_view.is_none() {
            let workspace = cx.weak_entity();
            self.tickets_view = Some(cx.new(|cx| TicketsView::new(workspace, window, cx)));
        }
        self.refresh_tickets(cx);
        cx.notify();
    }

    /// The tickets that have a pane working on them, by key.
    pub fn tickets_with_agents(&self, cx: &gpui_kit::App) -> HashSet<String> {
        self.all_tabs()
            .into_iter()
            .filter_map(|tab| self.ticket_of(tab, cx).map(|ticket| ticket.key().to_owned()))
            .collect()
    }

    /// The pull request a tab works on.
    fn pull_request_of(&self, tab: TabId) -> Option<&PullRequest> {
        self.pull_requests.get(&tab)
    }

    /// The ticket a tab works on: a ticket whose key is the tab's name, its Claude session's name
    /// or its branch.
    pub fn ticket_of(&self, tab: TabId, cx: &gpui_kit::App) -> Option<&Ticket> {
        if self.clickup.tickets.is_empty() {
            return None;
        }
        let view = self.terminals.get(&tab)?.read(cx);
        let mut names: Vec<String> = Vec::new();
        names.extend(self.names.get(&tab).cloned());
        names.extend(self.session_names.get(&tab).cloned());
        names.extend(view.title.clone());
        names.extend(self.checkouts.get(&tab).map(|checkout| checkout.branch.clone()));
        self.clickup.tickets.iter().find(|ticket| {
            let key = ticket.key().to_lowercase();
            names.iter().any(|name| {
                let name = name.to_lowercase();
                name == key || name.starts_with(&format!("{key}-")) || name.starts_with(&format!("{key} "))
            })
        })
    }

    /// Saves the file in front, formatting it if the setting is on.
    pub fn save_file(&mut self, cx: &mut Context<Self>) {
        let (root, format) = (self.root.clone(), self.settings.format_on_save);
        self.editor.update(cx, |editor, cx| editor.save(&root, format, cx));
    }

    /// Shows the Changes tab and reviews its first file once the changes are read (spikes).
    /// Opens the database diagram for the folder the sidebar shows.
    pub fn show_schema(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.editor.update(cx, |editor, cx| {
            editor.show_schema(&root, cx);
            if std::env::var_os("SATURN_SCHEMA").is_some() {
                editor.maximize(cx);
            }
        });
        self.show_editor();
        cx.notify();
    }

    /// Opens the lineage graph of the dbt project in the folder the sidebar shows, if it has one.
    pub fn show_lineage(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.roots.get(&self.root).and_then(|views| views.dbt_project.clone()) else { return };
        self.editor.update(cx, |editor, cx| {
            editor.show_lineage(&project, cx);
            if std::env::var_os("SATURN_LINEAGE").is_some() {
                editor.maximize(cx);
            }
        });
        self.show_editor();
        cx.notify();
    }

    /// How many terminal tabs the window has (spikes).
    pub fn tab_count(&self) -> usize {
        self.layout.tabs().count()
    }

    /// Makes the editor panel take the grid's room (spikes).
    pub fn maximize_editor(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, EditorView::maximize);
    }

    /// Lists the language servers' errors and warnings.
    pub fn show_problems(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, EditorView::show_problems);
    }

    /// Asks the open file's language server about `line`:`character` (spikes).
    pub fn probe_language(&mut self, line: u32, character: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| editor.probe_language(line, character, window, cx));
    }

    /// Opens the Impact tab for `repo`.
    pub fn show_impact(&mut self, repo: &Path, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.show_impact(repo, None, cx);
            if std::env::var_os("SATURN_IMPACT").is_some() {
                editor.maximize(cx);
            }
        });
        self.show_editor();
        cx.notify();
    }

    /// Opens the History tab for the folder the sidebar shows.
    pub fn show_history(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.editor.update(cx, |editor, cx| {
            editor.show_history(&root, cx);
            if std::env::var_os("SATURN_HISTORY").is_some() {
                editor.maximize(cx);
            }
        });
        self.show_editor();
        cx.notify();
    }

    /// Made-up ClickUp tickets, and the Tickets panel open (`pnpm demo`, spikes, screenshots).
    /// They're never refreshed, so the made-up token never reaches ClickUp.
    pub fn fake_tickets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0));
        let day = 24 * 60 * 60 * 1000;
        let ticket =
            |id: &str, key: &str, title: &str, status: &str, colour: &str, more: (&str, &str, &str, Option<i64>)| {
                let (list, priority, tag, due) = more;
                let space = if list == "API" { "Platform" } else { "Shop" };
                Ticket {
                    id: id.to_owned(),
                    custom_id: Some(key.to_owned()),
                    title: title.to_owned(),
                    status: status.to_owned(),
                    status_color: Some(colour.to_owned()),
                    url: format!("https://app.clickup.com/t/{id}"),
                    list: Some(list.to_owned()),
                    space_id: None,
                    space: Some(space.to_owned()),
                    priority: Some(priority.to_owned()),
                    tags: vec![tag.to_owned()],
                    due: due.map(|days| now + days * day),
                }
            };
        self.clickup.token = Some("demo".to_owned());
        self.clickup.made_up = true;
        self.clickup.tickets = vec![
            ticket(
                "a1",
                "ABC-240",
                "Filter on every list page",
                "in progress",
                "#5f55ee",
                ("Web", "high", "frontend", Some(2)),
            ),
            ticket(
                "a2",
                "ABC-236",
                "Round VAT half up on invoices",
                "in progress",
                "#5f55ee",
                ("Billing", "urgent", "billing", Some(-1)),
            ),
            ticket(
                "a3",
                "ABC-251",
                "Retry failed webhooks with backoff",
                "review",
                "#f8ae00",
                ("API", "normal", "backend", Some(5)),
            ),
            ticket(
                "a4",
                "ABC-224",
                "Name sessions after their ticket",
                "to do",
                "#87909e",
                ("Web", "low", "frontend", None),
            ),
            ticket(
                "a5",
                "ABC-258",
                "Show refunds on the order page",
                "to do",
                "#87909e",
                ("Billing", "high", "billing", Some(12)),
            ),
            ticket(
                "a6",
                "ABC-262",
                "Export the customer list as CSV",
                "to do",
                "#87909e",
                ("API", "normal", "backend", None),
            ),
        ];
        // Opened directly, not through toggle_tickets: that would send the made-up token to ClickUp.
        let workspace = cx.weak_entity();
        self.tickets_view = Some(cx.new(|cx| TicketsView::new(workspace, window, cx)));
        self.panels.tickets = true;
        cx.notify();
    }

    /// Gives the panes made-up agents, so screenshots show agents in every state without running Claude.
    pub fn fake_agents(&mut self, cx: &mut Context<Self>) {
        let samples = [
            (AgentState::Working, Some(40), "Running the tests", None),
            (AgentState::NeedsYou, None, "Which branch should I deploy?", Some((158, PrState::Open))),
            (AgentState::YourTurn, None, "Done: the filter is on every list page", Some((161, PrState::Draft))),
            (AgentState::Working, Some(72), "Writing the migration", None),
        ];
        // `SATURN_FAKE_STATES=working,needs-you,…` sets the panes' states in order, so the frames of
        // a demo clip can show agents moving from one to the next.
        let states: Vec<Option<AgentState>> = std::env::var("SATURN_FAKE_STATES")
            .map(|states| {
                states
                    .split(',')
                    .map(|state| match state.trim() {
                        "working" => Some(AgentState::Working),
                        "needs-you" => Some(AgentState::NeedsYou),
                        "your-turn" => Some(AgentState::YourTurn),
                        "error" => Some(AgentState::Error),
                        "idle" => Some(AgentState::Idle),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let tabs: Vec<TabId> = self.layout.tabs().collect();
        let mut pull_requests = 0;
        for (index, (tab, (state, progress, message, pull_request))) in tabs.into_iter().zip(samples).enumerate() {
            let state = states.get(index).copied().flatten().unwrap_or(state);
            if let Some((number, state)) = pull_request {
                let url = format!("https://github.com/acme/acme-shop/pull/{number}");
                let title = "Round VAT half up".to_owned();
                // Made-up checks too: one pane's failed, one's running, the rest passed.
                pull_requests += 1;
                let checks = match pull_requests {
                    1 => pull_request::Checks {
                        passed: 3,
                        running: Vec::new(),
                        failed: vec![pull_request::Check { name: "test".to_owned(), url: None }],
                    },
                    2 => pull_request::Checks { passed: 2, running: vec!["e2e".to_owned()], failed: Vec::new() },
                    _ => pull_request::Checks { passed: 4, ..Default::default() },
                };
                let preview = (pull_requests == 2).then(|| pull_request::Preview {
                    url: Some(format!("https://pr-{number}.acme.dev")),
                    name: format!("pr-{number}"),
                    ready: true,
                });
                self.pull_requests.insert(tab, PullRequest { number, title, url, state, checks, head: None, preview });
                if pull_requests == 2 {
                    let thread = |path: &str, line, said: &str| pull_request::Thread {
                        path: path.to_owned(),
                        line: Some(line),
                        comments: vec![("reviewer".to_owned(), said.to_owned())],
                    };
                    self.review_notes.insert(
                        tab,
                        ReviewNotes {
                            threads: vec![
                                thread("src/vat.ts", 12, "Round half up, not to even."),
                                thread("src/vat.test.ts", 4, "Add a case for 0.005."),
                            ],
                            changes_requested: Vec::new(),
                        },
                    );
                }
            }
            if let Some(view) = self.terminals.get(&tab) {
                view.update(cx, |view, _| {
                    view.state = state;
                    view.progress = progress;
                    view.message = Some(message.to_owned());
                });
            }
            // Not looked at yet, so the panes ring as they would.
            if matches!(state, AgentState::NeedsYou | AgentState::YourTurn) {
                self.unseen.insert(tab);
            }
        }
        cx.notify();
    }

    /// The editor panel, for spikes.
    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
    }

    pub fn review_first_change(&mut self, cx: &mut Context<Self>) {
        self.sidebar_tab = SidebarTab::Changes;
        let changes = self.current_root().changes.clone();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
            changes.update(cx, |changes, cx| changes.open(0, cx));
        })
        .detach();
        cx.notify();
    }

    /// Files / Changes, at the top of the sidebar.
    fn render_sidebar_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = |label: &'static str, target: SidebarTab, cx: &mut Context<Self>| {
            chrome::track_item(label, self.sidebar_tab == target)
                .flex_1()
                .px(len(space::SPACE_4))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sidebar_tab = target;
                    cx.notify();
                }))
        };
        chrome::track()
            .child(tab("Files", SidebarTab::Files, cx))
            .child(tab("Changes", SidebarTab::Changes, cx))
            .into_any_element()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn change_settings(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        let (appearance, menu_bar) = (self.settings.appearance, self.settings.menu_bar);
        let codex_status_line = self.settings.codex_status_line;
        change(&mut self.settings);
        settings::save(&self.settings);
        // Codex's status line lives in Saturn's Codex profile; the next Codex session shows it.
        if self.settings.codex_status_line != codex_status_line {
            cx.background_spawn(async { crate::agent_choice::install_codex_profile() }).detach();
        }
        if self.settings.appearance != appearance {
            theme::apply(self.settings.appearance, cx);
            theme::set_text_size(self.settings.text_size, cx);
        }
        if self.settings.menu_bar != menu_bar {
            menu_bar::set_shown(self.settings.menu_bar);
        }
        apply_app_settings(&self.settings, cx);
        cx.notify();
    }

    /// A hook from an agent's session in one of the panes: Claude Code's, or the end of a Codex turn
    /// (its `notify`), read as Claude Code's `Stop`.
    fn handle_hook(&mut self, mut call: HookCall, cx: &mut Context<Self>) {
        if Agent::of_program(&call.agent) == Some(Agent::Codex) {
            match saturn_core::agents::codex_turn_as_hook(&call.input) {
                Some(input) => call.input = input,
                None => return call.done(),
            }
        }
        let Some(tab) = call.pane.parse().ok().map(TabId) else { return call.done() };
        let Some(view) = self.terminals.get(&tab).cloned() else { return call.done() };
        let session_before = self.sessions.get(&tab).cloned();
        match (call.event(), call.field("session_id")) {
            ("SessionEnd", _) => {
                self.sessions.remove(&tab);
            }
            (_, Some(session)) => {
                self.sessions
                    .insert(tab, (Agent::of_program(&call.agent).unwrap_or(Agent::Claude), session.to_owned()));
                if let Some((routine, due)) = self.routine_tabs.remove(&tab) {
                    crate::routines_view::tie_session(routine, due, session.to_owned(), cx);
                }
            }
            _ => {}
        }
        if self.sessions.get(&tab) != session_before.as_ref() {
            self.save_session(cx);
        }
        if matches!(call.event(), "SessionStart" | "TaskCreated" | "TaskCompleted" | "TeammateIdle") {
            self.refresh_teams(cx);
        }
        match waiting::from_hook(call.event(), &call.input) {
            Some(Some(waiting)) => {
                self.waits.insert(tab, waiting);
            }
            Some(None) => {
                self.waits.remove(&tab);
            }
            None => {}
        }
        if let Some(path) = call.field("transcript_path").filter(|path| !path.is_empty()) {
            self.transcripts.insert(tab, PathBuf::from(path));
        }
        if call.event() == "Stop" {
            // A turn used some of the plan.
            crate::usage::refresh(false, cx);
            self.refresh_spent(cx);
        }
        let state = match call.event() {
            "UserPromptSubmit" | "PostToolUse" => Some(AgentState::Working),
            "PermissionRequest" => Some(AgentState::NeedsYou),
            "Stop" => Some(AgentState::YourTurn),
            "StopFailure" => Some(AgentState::Error),
            "SessionEnd" => Some(AgentState::Shell),
            "Notification" if call.field("notification_type") == Some("idle_prompt") => Some(AgentState::YourTurn),
            _ => None,
        };
        if let Some(state) = state {
            view.update(cx, |view, cx| view.hook_state(state, cx));
        }
        if call.event() == "CwdChanged"
            && let Some(cwd) = call.field("new_cwd").map(PathBuf::from)
        {
            view.update(cx, |view, _| view.cwd = cwd);
        }
        if call.event() == "UserPromptSubmit" && self.settings.rename_sessions_from_commands {
            let current = call.field("session_title");
            if let Some(name) = rename_for_prompt(
                call.field("prompt").unwrap_or_default(),
                current,
                self.session_names.get(&tab).map(String::as_str),
            ) {
                self.session_names.insert(tab, name.clone());
                return call.answer(&serde_json::json!({
                    "hookSpecificOutput": { "hookEventName": "UserPromptSubmit", "sessionTitle": name }
                }));
            }
        }
        call.done();
    }

    /// Starts a shell, in the focused terminal's folder or the project's, and places its tab.
    pub fn open_terminal(
        &mut self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        // In the focused pane's folder when it's in the repo the sidebar shows, else at that repo's top.
        let pane_folder = self
            .layout
            .focused_tab()
            .and_then(|focused| self.terminals.get(&focused))
            .map(|view| view.read(cx).cwd.clone());
        let cwd = pane_folder.filter(|folder| folder.starts_with(&self.root)).unwrap_or_else(|| self.root.clone());
        let tab = self.spawn_terminal(&cwd, window, cx)?;
        match placement {
            Placement::Tab => self.layout.add_tab(tab, None),
            Placement::Right => self.layout.split_right(tab, None),
            Placement::Down => self.layout.split_down(tab, None),
        }
        self.after_layout_change(window, cx);
        Some(tab)
    }

    /// Starts a shell in `cwd` in a new tab that is not in the grid yet.
    fn spawn_terminal(&mut self, cwd: &Path, window: &mut Window, cx: &mut Context<Self>) -> Option<TabId> {
        self.next_tab += 1;
        let tab = TabId(self.next_tab);
        let mut env: Vec<(String, String)> = vec![
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
            ("TERM_PROGRAM".into(), "Saturn".into()),
            // Programs don't know Saturn by name; this tells them it can open links they mark
            // (OSC 8), so Claude Code's links work even where the text wraps or hides the address.
            ("FORCE_HYPERLINK".into(), "1".into()),
            ("SATURN_PANE_ID".into(), tab.0.to_string()),
            // The oldest way to tell programs whether the background is light or dark ("text;
            // background" as ANSI colours); they ask the terminal itself for anything newer.
            ("COLORFGBG".into(), if saturn_terminal::palette::is_dark() { "15;0" } else { "0;15" }.into()),
        ];
        // The checkout's own block of ports, the same in every terminal of it and across restarts.
        let checkout = git::toplevel(cwd).unwrap_or_else(|| cwd.to_owned());
        if let Some(port) = settings::data_dir().and_then(|dir| ports::block_for(&dir.join("ports.json"), &checkout)) {
            env.push(("SATURN_PORT".into(), port.to_string()));
            self.ports.insert(checkout, port);
        }
        // Claude sessions in this pane load the Saturn plugin and report to this window only.
        if let (Some(bridge), Some(plugin)) = (&self.bridge, plugin_dir()) {
            env.push(("SATURN_SOCKET".into(), bridge.path().display().to_string()));
            let plugin = plugin.display().to_string();
            let dirs =
                std::env::var("CLAUDE_CODE_PLUGIN_DIRS").map_or(plugin.clone(), |dirs| format!("{dirs}:{plugin}"));
            env.push(("CLAUDE_CODE_PLUGIN_DIRS".into(), dirs));
        }
        let env: Vec<(&str, &str)> = env.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
        let (terminal, output) = match TerminalView::spawn(cwd, &env) {
            Ok(started) => started,
            Err(error) => {
                eprintln!("saturn: could not start a shell in {}: {error:#}", cwd.display());
                return None;
            }
        };
        let view = cx.new(|cx| TerminalView::new(terminal, output, cwd, cx));
        cx.subscribe_in(&view, window, move |this, _view, event, window, cx| {
            this.terminal_event(tab, event, window, cx);
        })
        .detach();
        self.terminals.insert(tab, view);
        self.refresh_checkouts(cx);
        Some(tab)
    }

    /// Opens the grid as it was: each tab in its folder, with its name, and a Claude session's
    /// resume command typed (not run) where one was running. False when nothing was saved.
    pub fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(saved) = session::load(&self.project) else { return false };
        for project in &saved.projects {
            if project.is_dir() && !self.projects.contains(project) {
                self.projects.push(project.clone());
                let views = Self::root_views(project, window, cx);
                self.roots.insert(project.clone(), views);
            }
        }
        let home = self.project.clone();
        let tabs = self.restore_tabs(&saved.tabs, &home, window, cx);
        let mut restored = !tabs.is_empty();
        if restored {
            self.layout = Layout::from_shape(&saved.shape, &tabs);
        }
        for page in &saved.pages {
            if !self.projects.contains(&page.project) {
                continue;
            }
            let tabs = self.restore_tabs(&page.tabs, &page.project, window, cx);
            if !tabs.is_empty() {
                restored = true;
                self.pages.insert(page.project.clone(), Layout::from_shape(&page.shape, &tabs));
            }
        }
        if !restored {
            return false;
        }
        if let Some(shown) = saved.shown.filter(|shown| self.projects.contains(shown)) {
            self.switch_to(shown, window, cx);
        }
        self.after_layout_change(window, cx);
        true
    }

    /// Starts a grid's saved tabs again, each in its folder (or `fallback`, when that's gone).
    fn restore_tabs(
        &mut self,
        saved: &[SavedTab],
        fallback: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<TabId> {
        let mut tabs = Vec::new();
        for saved_tab in saved {
            let cwd = if saved_tab.cwd.is_dir() { saved_tab.cwd.clone() } else { fallback.to_owned() };
            let Some(tab) = self.spawn_terminal(&cwd, window, cx) else { continue };
            if let Some(name) = &saved_tab.name {
                self.names.insert(tab, name.clone());
            }
            let agent = saved_tab.agent.unwrap_or(Agent::Claude);
            // The session stays the tab's until another starts there, so a tab not resumed this
            // time still offers it after the next start.
            if let Some(id) = saved_tab.session.clone() {
                self.sessions.insert(tab, (agent, id));
            }
            if let Some(command) = saved_tab
                .session
                .as_deref()
                .and_then(|id| session::resume_command(agent, id))
                .map(|command| crate::agent_choice::as_typed(&command, cx))
                && let Some(view) = self.terminals.get(&tab).cloned()
            {
                // Once the shell has drawn its prompt; typed text waits there for Enter.
                cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(800)).await;
                    let () = view.update(cx, |view, _| view.write(command.into_bytes()));
                })
                .detach();
            }
            tabs.push(tab);
        }
        tabs
    }

    /// Remembers every repo's grid for the next start, and which one showed.
    fn save_session(&self, cx: &gpui_kit::App) {
        let home = &self.projects[0];
        let empty = Layout::default();
        let grid_of = |project: &Path| if self.page == project { Some(&self.layout) } else { self.pages.get(project) };
        let home_grid = grid_of(home).unwrap_or(&empty);
        let pages: Vec<SavedPage> = self.projects[1..]
            .iter()
            .filter_map(|project| {
                let grid = grid_of(project).filter(|grid| !grid.is_empty())?;
                Some(SavedPage {
                    project: project.clone(),
                    shape: grid.shape(),
                    tabs: self.saved_tabs(grid, project, cx),
                })
            })
            .collect();
        let grid = (!home_grid.is_empty() || !pages.is_empty()).then(|| SavedGrid {
            projects: self.projects[1..].to_vec(),
            shape: home_grid.shape(),
            tabs: self.saved_tabs(home_grid, home, cx),
            pages,
            shown: (self.page != *home).then(|| self.page.clone()),
        });
        session::save(home, grid);
    }

    fn saved_tabs(&self, grid: &Layout, project: &Path, cx: &gpui_kit::App) -> Vec<SavedTab> {
        grid.tabs()
            .map(|tab| SavedTab {
                cwd: self.terminals.get(&tab).map_or_else(|| project.to_owned(), |view| view.read(cx).cwd.clone()),
                name: self.names.get(&tab).cloned(),
                session: self.sessions.get(&tab).map(|(_, id)| id.clone()),
                agent: self.sessions.get(&tab).map(|(agent, _)| *agent),
            })
            .collect()
    }

    /// Opens a terminal next to a given cell (from its header buttons).
    fn open_in_cell(&mut self, cell: CellId, placement: Placement, window: &mut Window, cx: &mut Context<Self>) {
        self.layout.focus_cell(cell);
        self.open_terminal(placement, window, cx);
    }

    pub fn terminal(&self, tab: TabId) -> Option<&Entity<TerminalView>> {
        self.terminals.get(&tab)
    }

    fn is_looking_at(&self, tab: TabId, window: &Window) -> bool {
        window.is_window_active() && self.layout.focused_tab() == Some(tab)
    }

    fn terminal_event(&mut self, tab: TabId, event: &TerminalViewEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TerminalViewEvent::Exited => self.forget(tab, window, cx),
            TerminalViewEvent::StateChanged => {
                let asks = self.terminals.get(&tab).is_some_and(|view| view.read(cx).state.asks_for_you());
                if asks && !window.is_window_active() && self.settings.notifications {
                    self.notify_about(tab, cx);
                }
                if asks && !self.is_looking_at(tab, window) {
                    self.unseen.insert(tab);
                } else if !asks {
                    self.unseen.remove(&tab);
                }
                // A turn just ended: its files may now meet another worktree's.
                if asks {
                    self.refresh_overlaps(cx);
                }
                self.follow_turn(tab, cx);
                self.report_attention(window, cx);
                cx.notify();
            }
            TerminalViewEvent::Bell => {
                if !self.is_looking_at(tab, window) {
                    self.unseen.insert(tab);
                }
                self.report_attention(window, cx);
                cx.notify();
            }
            TerminalViewEvent::TitleChanged => {
                self.read_codex_title(tab, cx);
                cx.notify();
            }
            TerminalViewEvent::Notified(text) => {
                // Codex says what it waits for this way (with Saturn's profile).
                if self.agent_in(tab) == Some(Agent::Codex) {
                    self.waits.insert(tab, saturn_core::waiting::from_codex_notification(text));
                    cx.notify();
                }
            }
            TerminalViewEvent::PromptShown => {
                self.started.remove(&tab);
                self.program_checked.remove(&tab);
            }
            // Coming back to the window isn't looking at the pane in front (your eyes may go to
            // another one); typing or clicking in it is.
            TerminalViewEvent::Used => {
                if self.unseen.remove(&tab) {
                    self.report_attention(window, cx);
                    cx.notify();
                }
            }
            TerminalViewEvent::PullRequestLinked => self.refresh_checkouts(cx),
            TerminalViewEvent::CwdChanged => {
                if self.layout.focused_tab() == Some(tab) {
                    self.sync_root(window, cx);
                }
                self.refresh_checkouts(cx);
                self.save_session(cx);
            }
        }
    }

    /// Codex says what it's doing only in its title; a tab running it takes its state from there.
    /// A `codex` typed by hand is found by the tab's foreground program, looked up now and then.
    fn read_codex_title(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let Some(view) = self.terminals.get(&tab).cloned() else { return };
        match self.agent_in(tab) {
            Some(Agent::Codex) => {
                if view.update(cx, TerminalView::codex_title_state) {
                    self.started.remove(&tab);
                    self.program_checked.remove(&tab);
                }
                // Working again (an approval answered, a new turn): it waits for nothing.
                if view.read(cx).state == AgentState::Working && self.waits.remove(&tab).is_some() {
                    cx.notify();
                }
                self.find_codex_session(tab, cx);
            }
            Some(Agent::Claude) => {}
            None => {
                let recently = self.program_checked.get(&tab).is_some_and(|at| at.elapsed() < Duration::from_secs(3));
                let Some(pid) = view.read(cx).foreground_pid().filter(|_| !recently) else { return };
                self.program_checked.insert(tab, Instant::now());
                cx.spawn(async move |this, cx| {
                    let program = cx.background_spawn(async move { process_name(pid) }).await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(agent) = program.as_deref().and_then(Agent::of_program) {
                            this.started.insert(tab, agent);
                            // Typed by hand a moment ago, at most.
                            this.started_at.insert(tab, crate::worktrees_view::now() - 60);
                            this.read_codex_title(tab, cx);
                        }
                    });
                })
                .detach();
            }
        }
    }

    /// A Codex pane's session, before Codex has said it as a turn ends: the newest Codex session
    /// that began in the pane's folder since Codex started there. So a pane quit mid-turn still
    /// resumes. Looked for once, off the main thread.
    fn find_codex_session(&mut self, tab: TabId, cx: &mut Context<Self>) {
        if self.sessions.contains_key(&tab) {
            return;
        }
        let Some(since) = self.started_at.remove(&tab) else { return };
        let Some(home) = Agent::Codex.home() else { return };
        let Some(folder) = self.terminals.get(&tab).map(|view| view.read(cx).cwd.clone()) else { return };
        cx.spawn(async move |this, cx| {
            // Codex writes the session's first line as it starts; give it a moment.
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let found = cx
                .background_spawn(async move { saturn_core::codex_sessions::session_in(&home, &folder, since) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(id) = found
                    && !this.sessions.contains_key(&tab)
                {
                    this.sessions.insert(tab, (Agent::Codex, id));
                    this.save_session(cx);
                }
            });
        })
        .detach();
    }

    /// A macOS notification about a tab that needs you or finished, while Saturn is in the background.
    fn notify_about(&mut self, tab: TabId, cx: &Context<Self>) {
        let Some(view) = self.terminals.get(&tab) else { return };
        let (state, message) = {
            let view = view.read(cx);
            (view.state, view.message.clone())
        };
        let name = self.tab_name(tab, cx);
        let (title, fallback, sound) = match state {
            AgentState::NeedsYou => (format!("{name} needs you"), "Waiting for your answer", true),
            AgentState::Error => (format!("{name} stopped"), "The turn failed", true),
            _ => (format!("{name} is done"), "Your turn", false),
        };
        let body = message.filter(|message| !message.is_empty()).unwrap_or_else(|| fallback.to_owned());
        notify::post(&format!("tab-{}", tab.0), &title, &body, sound);
    }

    /// Tells the Dock and the menu bar how many agents wait for you. Called when states change,
    /// not only on redraw: macOS doesn't redraw a hidden window, which is when these matter.
    fn report_attention(&mut self, window: &Window, cx: &gpui_kit::App) {
        if self.unseen.len() != self.badge {
            self.badge = self.unseen.len();
            dock::set_badge(self.badge);
        }
        let waiting = self.terminals.values().filter(|view| view.read(cx).state.asks_for_you()).count();
        if waiting != self.waiting {
            self.waiting = waiting;
            menu_bar::set_waiting(window.window_handle().window_id().as_u64(), waiting);
        }
    }

    /// Keeps an agent's steps (from its task list) and context (from its transcript) current.
    fn forget(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.terminals.remove(&tab);
        self.unseen.remove(&tab);
        self.names.remove(&tab);
        self.checkouts.remove(&tab);
        self.layout.remove_tab(tab);
        for grid in self.pages.values_mut() {
            grid.remove_tab(tab);
        }
        self.pages.retain(|_, grid| !grid.is_empty());
        self.report_attention(window, cx);
        self.after_layout_change(window, cx);
    }

    /// Focuses the terminal now in front and marks it seen.
    fn after_layout_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_root(window, cx);
        self.save_session(cx);
        if let Some(front) = self.layout.focused_tab() {
            self.unseen.remove(&front);
            if self.renaming.is_none()
                && let Some(view) = self.terminals.get(&front)
            {
                window.focus(&view.focus_handle(cx), cx);
            }
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn update_layout(&mut self, window: &mut Window, cx: &mut Context<Self>, change: impl FnOnce(&mut Layout)) {
        change(&mut self.layout);
        self.after_layout_change(window, cx);
    }

    /// Closes a tab, asking first when something other than the shell runs in it. The tab goes
    /// when its shell reports the exit.
    fn close_tab(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.terminals.get(&tab).cloned() else { return };
        let running =
            view.read(cx).foreground_pid().and_then(process_name).filter(|name| !SHELLS.contains(&name.as_str()));
        let Some(program) = running else {
            view.update(cx, |view, _| view.kill());
            return;
        };
        // The tab's name is often the title Claude gave its session, which reads oddly as the
        // question; it goes in the sentence below instead.
        let message = "Close this tab?";
        let detail = format!("{program} is still running in “{}” and will be stopped.", self.tab_name(tab, cx));
        let answer = window.prompt(PromptLevel::Warning, message, Some(&detail), &["Close", "Cancel"], cx);
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                view.update(cx, |view, _| view.kill());
            }
        })
        .detach();
    }

    /// A name typed by hand, else (when the setting is on) the session's or terminal's title,
    /// else the folder the terminal is in.
    fn tab_name(&self, tab: TabId, cx: &gpui_kit::App) -> String {
        if let Some(name) = self.names.get(&tab) {
            return name.clone();
        }
        let Some(view) = self.terminals.get(&tab).map(|view| view.read(cx)) else { return "zsh".to_owned() };
        let codex = self.agent_in(tab) == Some(Agent::Codex);
        let title = self
            .settings
            .name_tabs_after_sessions
            .then(|| view.title.as_deref().and_then(|title| tab_title(title, codex)))
            .flatten();
        title.unwrap_or_else(|| {
            view.cwd.file_name().map_or_else(|| "zsh".to_owned(), |folder| folder.to_string_lossy().into_owned())
        })
    }

    fn tab_state(&self, tab: TabId, cx: &gpui_kit::App) -> AgentState {
        self.terminals.get(&tab).map_or(AgentState::Shell, |view| view.read(cx).state)
    }

    fn start_rename(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.tab_name(tab, cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        cx.subscribe_in(&input, window, move |this, input, event, window, cx| match event {
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                let name = input.read(cx).value().trim().to_owned();
                this.finish_rename(tab, Some(name), window, cx);
            }
            _ => {}
        })
        .detach();
        input.update(cx, |input, cx| input.focus(window, cx));
        self.renaming = Some((tab, input));
        cx.notify();
    }

    /// Saves (or with `None`, drops) a typed name; an empty name goes back to the terminal title.
    fn finish_rename(&mut self, tab: TabId, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.as_ref().is_none_or(|(renaming, _)| *renaming != tab) {
            return;
        }
        self.renaming = None;
        match name {
            Some(name) if !name.is_empty() => {
                self.names.insert(tab, name);
            }
            Some(_) => {
                self.names.remove(&tab);
            }
            None => {}
        }
        self.after_layout_change(window, cx);
    }

    fn drop_tab(
        &mut self,
        tab: TabId,
        cell: CellId,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_layout(window, cx, |layout| layout.move_tab(tab, cell, index));
    }

    /// Where a tab being dragged over `cell_id` would land. Every pane hears every move of a drag;
    /// only the one under the pointer answers.
    fn track_drop(&mut self, cell_id: CellId, event: &DragMoveEvent<DraggedTab>, cx: &mut Context<Self>) {
        let target = event
            .bounds
            .contains(&event.event.position)
            .then(|| (cell_id, drop_side(event.bounds, event.event.position)));
        let mine = self.drop_target.is_some_and(|(cell, _)| cell == cell_id);
        if target.is_some() && self.drop_target != target {
            self.drop_target = target;
            cx.notify();
        } else if target.is_none() && mine {
            self.drop_target = None;
            cx.notify();
        }
    }

    /// A tab dropped on a pane: split the pane on the side it landed, or move into it.
    fn drop_on_pane(&mut self, tab: TabId, cell_id: CellId, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.drop_target.filter(|(cell, _)| *cell == cell_id).and_then(|(_, side)| side);
        self.drop_target = None;
        match side {
            Some(side) => self.update_layout(window, cx, |layout| layout.split_with(tab, cell_id, side)),
            None => self.drop_tab(tab, cell_id, None, window, cx),
        }
    }

    /// Asks for a folder and adds it to the window's repos.
    fn open_project(window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add Repo".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(project) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, window, cx| this.add_project(project, window, cx));
        })
        .detach();
    }

    fn render_tab(&self, cell: &Cell, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = cell.tabs[index];
        let cell_id = cell.id;
        let selected = tab == cell.active;
        let state = self.tab_state(tab, cx);
        let name = self.tab_name(tab, cx);
        // A tab hidden behind another that needs you rings orange, as its pane does, so the pane's
        // glow says which tab it's for. A fill hid the tab's own orange dot and read as a button.
        let calling = !selected && state == AgentState::NeedsYou && self.unseen.contains(&tab);
        let foreground = if selected { theme::color(color::INK) } else { theme::color(color::INK_MUTED) };
        let renaming = self.renaming.as_ref().filter(|(renaming, _)| *renaming == tab).map(|(_, input)| input.clone());

        let close = div()
            .id(("close", tab.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded(len(radius::RADIUS_XS))
            .when(!selected, |close| close.invisible().group_hover("tab", gpui_kit::Styled::visible))
            .hover(|s| s.bg(theme::fill_control()))
            .child(chrome::icon("x", 11., foreground))
            .tooltip(chrome::tip_for("Close this terminal", CloseTab))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.close_tab(tab, window, cx)));

        div()
            .id(("tab", tab.0))
            .group("tab")
            .flex()
            // A tab gives up its name before its ✕, so even a narrow pane can be closed.
            .flex_shrink(1.)
            .min_w(px(TAB_MIN_WIDTH))
            .items_center()
            .gap(len(6.))
            .h(len(TAB_HEIGHT))
            .pl(len(space::SPACE_3))
            .pr(len(space::SPACE_1))
            .max_w(px(220.))
            .rounded(len(radius::RADIUS_SM))
            // Selected the way every tab is (chrome::raised); a hidden tab that needs you, ringed.
            .map(|tab| chrome::raised(tab, selected))
            .when(calling, |tab| tab.shadow(theme::layers(&shadow::GLOW_NEEDS_YOU.get()[..1])))
            .when(!selected, |tab| tab.hover(|s| s.bg(theme::fill_hover())))
            .text_color(foreground)
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .child(status_dot(state))
            .map(|tab_element| match renaming {
                Some(input) => tab_element.child(div().w(px(140.)).child(Input::new(&input).appearance(false))),
                None => tab_element.child(div().min_w_0().truncate().child(name.clone())),
            })
            .child(close)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.update_layout(window, cx, |layout| layout.activate_tab(tab));
                }),
            )
            .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.close_tab(tab, window, cx)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.start_rename(tab, window, cx);
                }
            }))
            .on_drag(DraggedTab { tab, name }, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            .drag_over::<DraggedTab>(|style, _, _, _| style.border_l_2().border_color(theme::color(color::BLUE)))
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                this.drop_tab(dragged.tab, cell_id, Some(index), window, cx);
            }))
            .into_any_element()
    }

    /// Runs a check in `checkout` in the background, its log kept beside Saturn's settings.
    pub fn run_check(&mut self, checkout: &Path, name: String, command: String, cx: &mut Context<Self>) {
        let folder: String =
            checkout.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let Some(log) = settings::data_dir().map(|dir| dir.join("checks").join(folder).join(format!("{name}.log")))
        else {
            return;
        };
        let runs = self.check_runs.entry(checkout.to_owned()).or_default();
        runs.retain(|run| run.name != name);
        runs.push(CheckRun { name: name.clone(), command: command.clone(), log: log.clone(), outcome: None });
        cx.notify();
        let checkout = checkout.to_owned();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn({
                    let (checkout, log) = (checkout.clone(), log.clone());
                    async move { checks::run(&checkout, &command, &log) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(run) =
                    this.check_runs.get_mut(&checkout).and_then(|runs| runs.iter_mut().find(|run| run.name == name))
                {
                    run.outcome = Some(outcome);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The focused pane's servers, else those of another pane in its checkout.
    fn focused_servers(&self, cx: &gpui_kit::App) -> Vec<Server> {
        let checkout = self.focused_checkout(cx);
        let tab = self.layout.focused_tab();
        tab.and_then(|tab| self.servers.get(&tab).cloned())
            .filter(|servers| !servers.is_empty())
            .or_else(|| {
                self.servers
                    .iter()
                    .find(|(tab, servers)| {
                        !servers.is_empty() && self.checkouts.get(tab).is_some_and(|other| other.root == checkout)
                    })
                    .map(|(_, servers)| servers.clone())
            })
            .unwrap_or_default()
    }

    /// The focused pane's running app, in the editor panel's Preview tab.
    pub fn show_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let servers = self.focused_servers(cx);
        self.editor.update(cx, |editor, cx| editor.show_preview(servers, window, cx));
        self.show_editor();
        cx.notify();
    }

    /// Pictures of the focused pane's running app, kept for its repository, all worktrees together.
    pub fn show_pictures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let checkout = self.focused_checkout(cx);
        let repo = git::main_worktree(&checkout).unwrap_or_else(|| checkout.clone());
        let folder: String =
            repo.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let Some(root) = settings::data_dir().map(|dir| dir.join("pictures").join(folder)) else { return };
        let tab = self.layout.focused_tab();
        let branch =
            tab.and_then(|tab| self.checkouts.get(&tab)).map(|checkout| checkout.branch.clone()).unwrap_or_default();
        let servers = self.focused_servers(cx);
        crate::pictures_view::open(root, branch, servers, window, cx);
    }

    /// Opens the focused pane's pull request's description, to edit against the template.
    pub fn edit_pr_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.layout.focused_tab() else { return };
        let Some(number) = self.pull_request_of(tab).map(|pr| pr.number) else {
            let checkout = self.focused_checkout(cx);
            if let Some(tab) = self.recipe_tab(&checkout, "Description", window, cx) {
                self.note(
                    tab,
                    "This pane has no pull request yet: open one, and its description can be edited here.",
                    cx,
                );
            }
            return;
        };
        let checkout = self.focused_checkout(cx);
        crate::pr_body_view::open(cx.weak_entity(), checkout, number, window, cx);
    }

    /// Runs every check of the focused pane's checkout (for spikes).
    pub fn run_all_checks(&mut self, cx: &mut Context<Self>) {
        let checkout = self.focused_checkout(cx);
        let Ok((recipe, _)) = recipe::for_checkout(&checkout) else { return };
        for (name, command) in recipe.checks {
            self.run_check(&checkout, name, command, cx);
        }
    }

    /// The checks run in the pane's checkout: each with how it went; click for its log. A failed one
    /// can go to the agent.
    fn render_check_runs(&self, cell: &Cell, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = cell.active;
        let checkout = self.checkouts.get(&tab).map(|checkout| checkout.root.clone())?;
        let runs = self.check_runs.get(&checkout).filter(|runs| !runs.is_empty())?;
        let mut row = div().flex().flex_none().items_center().gap(len(space::SPACE_1));
        for (index, run) in runs.iter().enumerate().rev().take(3) {
            let (icon, tint, words) = match &run.outcome {
                None => ("circle-dot", color::BLUE, "running".to_owned()),
                Some(outcome) if outcome.passed() => {
                    ("circle-check", color::TERM_GREEN, format!("passed in {} s", outcome.seconds))
                }
                Some(outcome) if outcome.no_tests => ("circle-x", color::RED, "ran no tests".to_owned()),
                Some(outcome) => ("circle-x", color::RED, format!("failed after {} s", outcome.seconds)),
            };
            let tip = format!("{} ({}) {words}. Click for its log.", run.name, run.command);
            let log = run.log.clone();
            row = row.child(
                div()
                    .id(("check-run", cell.id.key() * 100 + index as u64))
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_1))
                    .h(len(TAB_HEIGHT))
                    .px(len(space::SPACE_2))
                    .rounded(len(radius::RADIUS_SM))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(color::INK_MUTED))
                    .hover(|s| s.bg(theme::fill_hover()))
                    .child(chrome::icon(icon, 12., theme::color(tint)))
                    .child(run.name.clone())
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if log.is_file() {
                            this.open_file(&log, window, cx);
                        }
                    })),
            );
            if run.outcome.as_ref().is_some_and(|outcome| !outcome.passed()) {
                let name = run.name.clone();
                row = row.child(
                    chrome::icon_button(("check-to-agent", cell.id.key() * 100 + index as u64), "wrench", true)
                        .tooltip(chrome::tip("Send what failed to this pane's agent"))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.send_check_failure(tab, &name, window, cx)),
                        ),
                );
            }
        }
        Some(row.into_any_element())
    }

    /// Hands a failed check, summed up, to the agent in a pane.
    fn send_check_failure(&mut self, tab: TabId, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(checkout) = self.checkouts.get(&tab).map(|checkout| checkout.root.clone()) else { return };
        let Some(run) = self.check_runs.get(&checkout).and_then(|runs| runs.iter().find(|run| run.name == name)) else {
            return;
        };
        let Some(outcome) = &run.outcome else { return };
        let message = checks::failure_message(&run.name, &run.command, outcome, &run.log);
        self.send_to_agent(&checkout, &message, window, cx);
    }

    /// When the pane's pull request's checks failed: a button that hands them to its agent.
    fn render_fix_ci(&self, cell: &Cell, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = cell.active;
        let pr = self.pull_request_of(tab).filter(|pr| pr.checks.state() == Some(ChecksState::Failed))?;
        let names = pr.checks.failed.iter().map(|check| check.name.as_str()).collect::<Vec<_>>().join(", ");
        let tip = format!("Send the failed checks ({names}) to this pane's agent, to find out why and fix them.");
        Some(
            chrome::icon_button(("fix-ci", cell.id.key()), "wrench", true)
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| this.fix_ci(tab, window, cx)))
                .into_any_element(),
        )
    }

    /// When reviewers left things open on the pane's pull request: how many, and a button that hands
    /// them to its agent.
    fn render_review_notes(&self, cell: &Cell, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = cell.active;
        let count = self.review_notes.get(&tab)?.count();
        let s = if count == 1 { "" } else { "s" };
        let tip = format!("{count} open review comment{s} on the pull request: send them to this pane's agent.");
        Some(
            div()
                .id(("review-notes", cell.id.key()))
                .flex()
                .flex_none()
                .items_center()
                .gap(len(space::SPACE_1))
                .h(len(TAB_HEIGHT))
                .px(len(space::SPACE_2))
                .rounded(len(radius::RADIUS_SM))
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::color(color::INK_MUTED))
                .hover(|s| s.bg(theme::fill_hover()))
                .child(chrome::icon("message-square", 13., theme::color(color::INK_MUTED)))
                .child(count.to_string())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| this.send_review_notes(tab, window, cx)))
                .into_any_element(),
        )
    }

    /// Hands what reviewers left open to the agent working in the pane.
    fn send_review_notes(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(pr), Some(notes)) = (self.pull_request_of(tab), self.review_notes.get(&tab)) else { return };
        let message = pull_request::review_notes_message(pr, notes);
        let checkout = self
            .checkouts
            .get(&tab)
            .map(|checkout| checkout.root.clone())
            .or_else(|| self.terminals.get(&tab).map(|view| view.read(cx).cwd.clone()));
        if let Some(checkout) = checkout {
            self.send_to_agent(&checkout, &message, window, cx);
        }
    }

    /// Hands a pane's failed checks to the agent working there.
    fn fix_ci(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pr) = self.pull_request_of(tab) else { return };
        let message = pull_request::failed_checks_message(pr);
        let checkout = self
            .checkouts
            .get(&tab)
            .map(|checkout| checkout.root.clone())
            .or_else(|| self.terminals.get(&tab).map(|view| view.read(cx).cwd.clone()));
        if let Some(checkout) = checkout {
            self.send_to_agent(&checkout, &message, window, cx);
        }
    }

    /// Undo (or redo) the last turn of the pane's agent, once it changed files.
    fn render_turn(&self, cell: &Cell, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = cell.active;
        let turn = self.turns.get(&tab).filter(|turn| turn.after.is_some() && !turn.files.is_empty())?;
        let count = turn.files.len();
        let files = turn.files.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
        let more = if count > 6 { format!(" and {} more", count - 6) } else { String::new() };
        let (icon, tip) = if turn.undone {
            ("redo-2", format!("Redo the agent's last turn: {files}{more} as it left them."))
        } else {
            (
                "undo-2",
                format!(
                    "Undo the agent's last turn: {files}{more} back as they were before it, however they were changed. Tell the agent, so it doesn't count on them."
                ),
            )
        };
        let said = turn.said.clone();
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(len(space::SPACE_1))
                .children(said.map(|said| {
                    div()
                        .max_w(px(260.))
                        .truncate()
                        .text_size(px(12.))
                        .text_color(theme::color(color::INK_MUTED))
                        .child(said)
                }))
                .child(
                    chrome::icon_button(("undo-turn", cell.id.key()), icon, true)
                        .when(turn.busy, |button| button.opacity(0.4))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .on_click(cx.listener(move |this, _, _, cx| this.undo_turn(tab, cx))),
                )
                .into_any_element(),
        )
    }

    /// The right of a pane's tab strip: its ticket and pull request, then split, new tab and the
    /// pane's menu. In a narrow pane the chips are cut off, never the buttons: the menu is where the
    /// pane's other features are found.
    fn render_pane_actions(&self, cell: &Cell, cx: &mut Context<Self>) -> impl IntoElement {
        let cell_id: CellId = cell.id;
        let header_button = |id: &'static str, icon: &str, placement: Placement, cx: &mut Context<Self>| {
            let button = chrome::icon_button((id, cell_id.key()), icon, true)
                .on_click(cx.listener(move |this, _, window, cx| this.open_in_cell(cell_id, placement, window, cx)));
            match placement {
                Placement::Right => {
                    button.tooltip(chrome::tip_for("Split right: a new terminal beside this one", SplitRight))
                }
                Placement::Down => {
                    button.tooltip(chrome::tip_for("Split down: a new terminal below this one", SplitDown))
                }
                Placement::Tab => button.tooltip(chrome::tip_for("New terminal, as a tab here", NewTab)),
            }
        };

        // The work's chips (its ticket, pull request, checks) show while an agent runs in the pane.
        // A pane reopened after a restart holds only a shell with `claude --resume` typed in it, and
        // you may start something else there instead.
        let working = self.tab_state(cell.active, cx) != AgentState::Shell;
        let chips = div().flex().flex_shrink(1.).min_w_0().overflow_hidden().items_center().gap(len(space::SPACE_1));
        let chips = if working { self.work_chips_before(chips, cell, cx) } else { chips };
        let chips = chips
            .children(self.servers.get(&cell.active).into_iter().flatten().map(|server| server_chip(cell_id, server)));
        let chips = if working { self.work_chips_after(chips, cell, cx) } else { chips };
        let buttons = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(len(space::SPACE_1))
            .child(header_button("split-right", "columns-2", Placement::Right, cx))
            .child(header_button("split-down", "rows-2", Placement::Down, cx))
            .child(header_button("new-tab", "plus", Placement::Tab, cx))
            .child(
                self.with_menu(
                    OpenMenu::Pane(cell_id),
                    chrome::icon_button(("pane-menu", cell_id.key()), "ellipsis", true)
                        .when(self.open_menu == Some(OpenMenu::Pane(cell_id)), |button| {
                            button.bg(theme::fill_pressed())
                        })
                        .tooltip(chrome::tip("This pane: its app, checks, pull request and what its agent is told"))
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_menu(OpenMenu::Pane(cell_id), cx))),
                    cx,
                ),
            )
            // On the focused pane only, and the maximised one to restore it: one more button on
            // every pane would crowd out their chips. ⌘⇧↩ works from any pane.
            .children(
                (self.layout.focused == Some(cell_id) || self.layout.maximized == Some(cell_id))
                    .then(|| self.render_maximize_button(cell_id, cx)),
            );
        div().flex().flex_shrink(1.).min_w_0().items_center().gap(len(space::SPACE_1)).child(chips).child(buttons)
    }

    /// The work's chips before the servers: its ticket, team, attempt, other repos, what it used,
    /// its last turn and the worktrees it overlaps.
    fn work_chips_before(&self, chips: gpui_kit::Div, cell: &Cell, cx: &mut Context<Self>) -> gpui_kit::Div {
        let cell_id = cell.id;
        chips
            .children(self.ticket_of(cell.active, cx).map(|ticket| ticket_chip(cell_id, ticket)))
            .children(self.teams.get(&cell.active).map(|team| team_chip(cell_id, cell.active, team, cx)))
            .children(
                self.checkouts
                    .get(&cell.active)
                    .and_then(|checkout| best_of::attempt_of(&checkout.branch))
                    .map(|(_, number)| attempt_chip(cell_id, number)),
            )
            .children(self.render_links(cell, cx))
            .children(self.render_turn(cell, cx))
            .children(
                self.checkouts
                    .get(&cell.active)
                    .and_then(|checkout| self.overlaps.get(&checkout.root))
                    .map(|others| overlap_chip(cell_id, others)),
            )
    }

    /// The work's chips after the servers: its pull request, preview, checks and review threads.
    fn work_chips_after(&self, chips: gpui_kit::Div, cell: &Cell, cx: &mut Context<Self>) -> gpui_kit::Div {
        let cell_id = cell.id;
        chips
            .children(self.pull_request_of(cell.active).map(|pr| pull_request_chip(cell_id, pr)))
            .children(self.pull_request_of(cell.active).and_then(|pr| preview_chip(cell_id, pr)))
            .children(self.render_check_runs(cell, cx))
            .children(self.render_fix_ci(cell, cx))
            .children(self.render_review_notes(cell, cx))
    }

    /// Opens a button's menu, or closes it when it's the one open.
    fn toggle_menu(&mut self, which: OpenMenu, cx: &mut Context<Self>) {
        let just_closed = self
            .menu_dismissed
            .take()
            .is_some_and(|(closed, at)| closed == which && at.elapsed() < Duration::from_millis(400));
        self.open_menu = (self.open_menu != Some(which) && !just_closed).then_some(which);
        cx.notify();
    }

    /// Does what was picked from a menu: a pane's menu acts on that pane, as its actions do on the
    /// focused one.
    fn pick_from_menu(
        &mut self,
        which: OpenMenu,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_menu = None;
        if let OpenMenu::Pane(cell) = which {
            self.update_layout(window, cx, |layout| layout.focus_cell(cell));
        }
        cx.notify();
        window.dispatch_action(action, cx);
    }

    /// For a spike: opens a menu by name (`new-agent`, `pane`, `repo`) and picks its `pick`th line.
    pub fn spike_menu(&mut self, name: &str, pick: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let which = match name {
            "new-agent" => OpenMenu::NewAgent,
            "repo" => OpenMenu::Repo,
            _ => match self.layout.focused {
                Some(cell) => OpenMenu::Pane(cell),
                None => return,
            },
        };
        self.open_menu = Some(which);
        cx.notify();
        let items = match which {
            OpenMenu::NewAgent => new_agent_menu(),
            OpenMenu::Pane(_) => pane_menu(),
            OpenMenu::Repo => repo_menu(),
        };
        if let Some(action) = pick.and_then(|index| items.into_iter().flatten().nth(index)).map(|(_, _, action)| action)
        {
            self.pick_from_menu(which, action, window, cx);
        }
    }

    /// A button with its menu hanging under it while it's open.
    fn with_menu(&self, which: OpenMenu, button: impl IntoElement, cx: &mut Context<Self>) -> impl IntoElement {
        let menu = (self.open_menu == Some(which)).then(|| {
            let items = match which {
                OpenMenu::NewAgent => new_agent_menu(),
                OpenMenu::Pane(_) => pane_menu(),
                OpenMenu::Repo => repo_menu(),
            };
            let workspace = cx.weak_entity();
            let pick = move |action: Box<dyn Action>, window: &mut Window, cx: &mut gpui_kit::App| {
                let _ = workspace.update(cx, |this, cx| this.pick_from_menu(which, action, window, cx));
            };
            let workspace = cx.weak_entity();
            let dismiss = move |_: &mut Window, cx: &mut gpui_kit::App| {
                let _ = workspace.update(cx, |this, cx| {
                    this.open_menu = None;
                    this.menu_dismissed = Some((which, Instant::now()));
                    cx.notify();
                });
            };
            menu::render("menu", items, pick, dismiss)
        });
        div().relative().flex_none().child(button).children(menu)
    }

    /// Maximises this pane over the grid and the panels, or restores it. The pane takes focus
    /// first, so the button maximises the pane it sits on, not the one focused before.
    fn render_maximize_button(&self, cell_id: CellId, cx: &mut Context<Self>) -> impl IntoElement {
        let maximized = self.layout.maximized == Some(cell_id);
        let (icon, tip) = if maximized { ("minimize-2", "Minimise") } else { ("maximize-2", "Maximise") };
        chrome::icon_button(("maximize", cell_id.key()), icon, true)
            .tooltip(chrome::tip_for(tip, ToggleMaximize))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.update_layout(window, cx, |layout| {
                    layout.focus_cell(cell_id);
                    layout.toggle_maximize();
                });
            }))
    }

    fn render_cell(&self, cell: &Cell, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.layout.focused == Some(cell.id);
        let unseen_states: Vec<AgentState> =
            cell.tabs.iter().filter(|tab| self.unseen.contains(tab)).map(|tab| self.tab_state(*tab, cx)).collect();
        // Orange for anything waiting on you, a stopped turn included; green for a finished one.
        let needs_you = unseen_states.iter().any(|state| matches!(state, AgentState::NeedsYou | AgentState::Error));
        let glow = if needs_you {
            Some(shadow::GLOW_NEEDS_YOU)
        } else if !unseen_states.is_empty() {
            Some(shadow::GLOW_YOUR_TURN)
        } else if focused {
            Some(shadow::GLOW_FOCUS)
        } else {
            None
        };
        let terminal = self.terminals.get(&cell.active).cloned();
        let cell_id: CellId = cell.id;
        // A panel like every other: the tabs in its strip, the terminal in its well.
        let mut pane = chrome::panel()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.update_layout(window, cx, |layout| layout.focus_cell(cell_id));
                }),
            )
            .child(
                div()
                    .id(("tabs", cell_id.key()))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(len(space::SPACE_1))
                    .h(len(TAB_HEIGHT))
                    .child(
                        div()
                            .id(("tab-strip", cell_id.key()))
                            .flex()
                            .flex_1()
                            // Room for one tab, whatever the buttons beside it want.
                            .min_w(px(TAB_MIN_WIDTH))
                            .gap(px(2.))
                            // Scrolling clips to the strip, which would cut the raised tab's shadow:
                            // the strip reaches a few pixels past its tabs on every side instead.
                            .m(px(-4.))
                            .p(px(4.))
                            .overflow_x_scroll()
                            .children((0..cell.tabs.len()).map(|index| self.render_tab(cell, index, cx)))
                            .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                                this.drop_tab(dragged.tab, cell_id, None, window, cx);
                            })),
                    )
                    .child(self.render_pane_actions(cell, cx)),
            )
            .child(
                chrome::well()
                    .id(("body", cell_id.key()))
                    .pt(len(space::SPACE_3))
                    .pl(len(space::SPACE_4))
                    .pb(len(space::SPACE_1))
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DraggedTab>, _, cx| {
                        this.track_drop(cell_id, event, cx);
                    }))
                    .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                        this.drop_on_pane(dragged.tab, cell_id, window, cx);
                    }))
                    .children(terminal)
                    .children(
                        self.drop_target
                            .filter(|(cell, _)| *cell == cell_id && cx.has_active_drag())
                            .map(|(_, side)| drop_preview(side)),
                    ),
            );
        if let Some(glow) = glow {
            pane = pane.border_color(gpui_kit::transparent_black()).shadow(theme::shadows(glow));
        }
        if needs_you {
            // Needs you breathes, slowly, so it is seen from the corner of the eye without flashing.
            return pane
                .with_animation(
                    ("breathe", cell_id.key()),
                    Animation::new(Duration::from_millis(2400)).repeat().with_easing(bounce(ease_in_out)),
                    |pane, t| pane.shadow(theme::layers(&breathing_glow(t))),
                )
                .into_any_element();
        }
        pane.into_any_element()
    }

    /// A divider between rows or cells: the grid's gap, draggable. A double click makes its two
    /// neighbours the same size.
    fn render_divider(divider: GridDivider, vertical: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = match divider {
            GridDivider::Rows(index) => ("row-divider", index as u64),
            GridDivider::Cells(row, index) => ("cell-divider", row.key() * 1000 + index as u64),
        };
        div()
            .id(id)
            .flex_none()
            .when(vertical, |d| d.w(len(space::SPACE_2)).h_full().cursor(CursorStyle::ResizeLeftRight))
            .when(!vertical, |d| d.h(len(space::SPACE_2)).w_full().cursor(CursorStyle::ResizeUpDown))
            .on_drag(divider, |_, _, _, cx| cx.new(|_| DividerPreview))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.click_count() != 2 {
                    return;
                }
                match divider {
                    GridDivider::Rows(index) => {
                        let sizes: Vec<f32> = this.layout.rows.iter().map(|row| row.size).collect();
                        this.layout.resize_rows(&even_divider(&sizes, index));
                    }
                    GridDivider::Cells(row_id, index) => {
                        let Some(row) = this.layout.rows.iter().find(|row| row.id == row_id) else { return };
                        let sizes: Vec<f32> = row.cells.iter().map(|cell| cell.size).collect();
                        this.layout.resize_cells(row_id, &even_divider(&sizes, index));
                    }
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let maximized = self.layout.maximized;
        let row_sizes: Vec<f32> = self.layout.rows.iter().map(|row| row.size).collect();
        let mut grid = div().id("grid").flex().flex_col().flex_1().min_h_0().on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<GridDivider>, _, cx| {
                if let GridDivider::Rows(index) = *event.drag(cx) {
                    let at =
                        f32::from(event.event.position.y - event.bounds.top()) / f32::from(event.bounds.size.height);
                    let sizes = move_divider(&row_sizes, index, at);
                    this.layout.resize_rows(&sizes);
                    cx.notify();
                }
            },
        ));
        // A maximised pane stands back from the edges, so what it covers reads as hidden, not gone.
        if maximized.is_some() {
            grid = grid.p(len(space::SPACE_6));
        }
        for (row_index, row) in self.layout.rows.iter().enumerate() {
            let shown: Vec<&Cell> = row.cells.iter().filter(|cell| maximized.is_none_or(|id| id == cell.id)).collect();
            if shown.is_empty() {
                continue;
            }
            if row_index > 0 && maximized.is_none() {
                grid = grid.child(Self::render_divider(GridDivider::Rows(row_index - 1), false, cx));
            }
            let row_id = row.id;
            let cell_sizes: Vec<f32> = row.cells.iter().map(|cell| cell.size).collect();
            let mut row_element =
                div().id(("row", row_id.key())).flex().min_h_0().flex_basis(relative(0.)).on_drag_move(cx.listener(
                    move |this, event: &DragMoveEvent<GridDivider>, _, cx| {
                        if let GridDivider::Cells(dragged_row, index) = *event.drag(cx)
                            && dragged_row == row_id
                        {
                            let at = f32::from(event.event.position.x - event.bounds.left())
                                / f32::from(event.bounds.size.width);
                            let sizes = move_divider(&cell_sizes, index, at);
                            this.layout.resize_cells(row_id, &sizes);
                            cx.notify();
                        }
                    },
                ));
            row_element.style().flex_grow = Some(if maximized.is_some() { 1.0 } else { row.size });
            for (cell_index, cell) in shown.into_iter().enumerate() {
                if cell_index > 0 && maximized.is_none() {
                    row_element =
                        row_element.child(Self::render_divider(GridDivider::Cells(row_id, cell_index - 1), true, cx));
                }
                let mut cell_element =
                    div().flex().min_w_0().flex_basis(relative(0.)).child(self.render_cell(cell, cx));
                cell_element.style().flex_grow = Some(if maximized.is_some() { 1.0 } else { cell.size });
                row_element = row_element.child(cell_element);
            }
            grid = grid.child(row_element);
        }
        grid.into_any_element()
    }

    /// The panel toggles (raised while the panel is open) and Settings, in the title bar.
    fn render_panel_toggles(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Lit only while their panel is on screen: a maximised pane covers them all.
        let panels = self.panels.on_screen(self.layout.maximized.is_some());
        let toggle = |id: &'static str, icon: &str, on: bool| {
            chrome::track_item(id, on).size(len(TITLEBAR_CONTROL)).child(chrome::icon(
                icon,
                15.,
                theme::color(if on { color::INK } else { color::INK_MUTED }),
            ))
        };
        chrome::on_glass(
            chrome::track()
                .child(
                    toggle("toggle-sidebar", "panel-left", panels.sidebar)
                        .tooltip(chrome::tip_for("Show or hide the sidebar", ToggleSidebar))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
                .child(
                    toggle("toggle-editor", "panel-top", panels.editor)
                        .tooltip(chrome::tip_for("Show or hide the editor", ToggleEditor))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_editor(cx))),
                )
                // Only with ClickUp connected: without it there's nothing to show.
                .when(self.clickup.connected(), |track| {
                    track.child(
                        toggle("toggle-tickets", "panel-right", panels.tickets)
                            .tooltip(chrome::tip_for("Show or hide your tickets", ToggleTickets))
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_tickets(window, cx))),
                    )
                })
                .child(chrome::track_separator())
                .child(
                    toggle("open-settings", "settings", false)
                        .tooltip(chrome::tip_for("Settings", OpenSettings))
                        .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
                ),
        )
    }

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let states: Vec<AgentState> = self.terminals.values().map(|view| view.read(cx).state).collect();
        let count = |wanted: &[AgentState]| states.iter().filter(|state| wanted.contains(state)).count();
        let summary = [
            (count(&[AgentState::NeedsYou, AgentState::Error]), AgentState::NeedsYou, "need you"),
            (count(&[AgentState::YourTurn]), AgentState::YourTurn, "your turn"),
            (count(&[AgentState::Working]), AgentState::Working, "working"),
        ];
        // The title bar is drawn by Saturn, so the empty parts of it pass a double-click on to
        // macOS, which zooms or minimizes as System Settings says.
        let empty = |id: &'static str| {
            div().id(id).h_full().on_click(|event: &ClickEvent, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
        };
        // Controls live in tracks, as everywhere: the panel toggles on the left, what the agents
        // are doing on the right, then the one primary action.
        let live: Vec<(usize, AgentState, &str)> = summary.into_iter().filter(|(n, _, _)| *n > 0).collect();
        let activity = (!live.is_empty()).then(|| {
            let mut track = chrome::track();
            for (index, (n, state, label)) in live.into_iter().enumerate() {
                if index > 0 {
                    track = track.child(chrome::track_separator());
                }
                track = track.child(
                    chrome::track_item(label, false)
                        .h(len(TITLEBAR_CONTROL))
                        .px(len(space::SPACE_3))
                        .when(state == AgentState::Working, |item| {
                            item.bg(theme::color(color::BLUE_VIVID).opacity(0.14)).text_color(theme::color(color::BLUE))
                        })
                        .when(state != AgentState::Working, |item| item.text_color(state_color(state)))
                        .child(status_dot(state))
                        .child(format!("{n} {label}"))
                        .when(state == AgentState::Working, |item| {
                            item.tooltip(chrome::tip_for("Go to the next agent that needs you", NextAttention))
                                .on_click(cx.listener(|this, _, window, cx| this.next_attention(window, cx)))
                        })
                        .when(state != AgentState::Working, |item| {
                            item.tooltip(chrome::tip("What each agent waits for: answer or go to it")).on_click(
                                cx.listener(|_, _, window, cx| {
                                    crate::queue_view::open(cx.weak_entity(), window, cx);
                                }),
                            )
                        }),
                );
            }
            chrome::on_glass(track)
        });
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(len(TITLEBAR_HEIGHT))
            .pr(len(space::SPACE_1))
            .child(empty("titlebar-lights").flex_none().w(len(TRAFFIC_LIGHTS_WIDTH)))
            .child(self.render_panel_toggles(cx))
            .child(self.render_repo_tabs(cx))
            .child(empty("titlebar-middle").flex_1())
            .children(activity)
            .child(self.render_new_agent(cx))
    }

    /// New terminals come from each pane's ＋ (and ⌘N), repos from the ＋ after them. Starting an
    /// agent on new work, named or from a ticket, is the one primary action; its shortcut is in its
    /// tooltip, to keep the bar short. Its arrow holds the other ways to start agents: several at
    /// once, again, on a schedule.
    fn render_new_agent(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let height = len(TITLEBAR_CONTROL + 2. * space::SPACE_1);
        // A part of the button, with no fill of its own: like the primary button, it doesn't light
        // up under the pointer, so the gradient stays whole. The arrow looks pressed while its menu
        // is open.
        let part = |id: &'static str| div().id(id).flex().items_center().h(height).cursor_pointer();
        let open = self.open_menu == Some(OpenMenu::NewAgent);
        let more = part("new-agent-more")
            .px(len(space::SPACE_2))
            .rounded_r(len(radius::RADIUS_MD))
            .when(open, |part| part.bg(gpui_kit::white().opacity(0.1)))
            .child(chrome::icon("chevron-down", 14., theme::color(color::INK_ON_ACCENT)))
            .tooltip(chrome::tip("More ways to start agents: Best of N, routines, past sessions"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(OpenMenu::NewAgent, cx)));
        chrome::primary_surface(div())
            .flex()
            .items_center()
            .h(height)
            .rounded(len(radius::RADIUS_MD))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .child(
                part("new-agent")
                    .px(len(space::SPACE_4))
                    .rounded_l(len(radius::RADIUS_MD))
                    .child("New agent")
                    .tooltip(chrome::tip_for("New agent, in a worktree of its own", OpenTicketPicker))
                    .on_click(cx.listener(|this, _, window, cx| {
                        ticket_picker::open(cx.weak_entity(), this.clickup.connected(), window, cx);
                    })),
            )
            .child(div().flex_none().w(px(1.)).h(px(16.)).bg(theme::color(color::INK_ON_ACCENT).opacity(0.3)))
            .child(self.with_menu(OpenMenu::NewAgent, more, cx))
    }

    /// The sidebar's header: the folder it shows (the selected repo, or a ticket's worktree when
    /// following a pane in one), and the follow switch.
    fn render_sidebar_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let short = match home.as_deref().and_then(|home| self.root.strip_prefix(home).ok()) {
            Some(rest) => format!("~/{}", rest.display()),
            None => self.root.display().to_string(),
        };
        let name = self.root.file_name().map_or_else(|| "/".to_owned(), |name| name.to_string_lossy().into_owned());
        let branch = self.roots.get(&self.root).and_then(|views| views.branch.clone());
        let ports = self.ports.get(&self.root).map(|port| format!("ports {port}–{}", port + ports::BLOCK - 1));
        let detail = [branch, ports, Some(short.clone())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let follow = chrome::icon_button("follow-pane", if self.follow_pane { "locate-fixed" } else { "locate" }, true)
            .when(self.follow_pane, |button| button.bg(theme::fill_pressed()))
            .tooltip(chrome::tip("Follow the focused terminal: the sidebar shows the folder it works in"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.follow_pane = !this.follow_pane;
                this.sync_root(window, cx);
                cx.notify();
            }));
        // The database diagram, when this folder describes a database.
        let schema = self.roots.get(&self.root).is_some_and(|views| views.has_schema).then(|| {
            chrome::icon_button("show-schema", "database", true)
                .tooltip(chrome::tip("Database diagram: the tables and how they connect"))
                .on_click(cx.listener(|this, _, _, cx| this.show_schema(cx)))
        });
        // The dbt lineage graph, when this folder holds a dbt project.
        let lineage = self.roots.get(&self.root).is_some_and(|views| views.dbt_project.is_some()).then(|| {
            chrome::icon_button("show-lineage", "workflow", true)
                .tooltip(chrome::tip_for("dbt lineage: which models build on which", ShowLineage))
                .on_click(cx.listener(|this, _, _, cx| this.show_lineage(cx)))
        });
        chrome::sidebar_header(
            &name,
            &detail,
            div().flex().items_center().children(lineage).children(schema).child(follow).child(
                self.with_menu(
                    OpenMenu::Repo,
                    chrome::icon_button("repo-menu", "ellipsis", true)
                        .when(self.open_menu == Some(OpenMenu::Repo), |button| button.bg(theme::fill_pressed()))
                        .tooltip(chrome::tip("This repo: worktrees, attempts, history"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(OpenMenu::Repo, cx))),
                    cx,
                ),
            ),
        )
        .into_any_element()
    }

    /// The window's repos, in the title bar: the selected one raised, ✕ on hover to take one out
    /// (its panes stay). The ＋ after them adds one.
    fn render_repo_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let removable = self.projects.len() > 1;
        let mut track = chrome::track();
        for (index, project) in self.projects.iter().enumerate() {
            let selected = *project == self.page;
            // A repo out of sight whose agents wait for you says so, as a browser tab's title does.
            let waiting = self.pages.get(project).and_then(|grid| {
                let states: Vec<AgentState> = grid.tabs().map(|tab| self.tab_state(tab, cx)).collect();
                [AgentState::NeedsYou, AgentState::YourTurn].into_iter().find(|state| states.contains(state))
            });
            let name = project.file_name().map_or_else(|| "/".to_owned(), |name| name.to_string_lossy().into_owned());
            let picked = project.clone();
            let removed = project.clone();
            // Each tab its own hover group, so only the one under the mouse shows its ✕.
            let group = gpui_kit::SharedString::from(format!("repo-{index}"));
            track = track.child(
                chrome::track_item(("repo", index), selected)
                    .h(len(TITLEBAR_CONTROL))
                    .relative()
                    .group(group.clone())
                    .px(len(space::SPACE_3))
                    .text_color(theme::color(if selected { color::INK } else { color::INK_MUTED }))
                    .when_some(waiting, |item, state| item.child(status_dot(state)))
                    .child(name)
                    // On hover only, floating over the tab's end, so a hidden ✕ takes no room.
                    .when(removable, |item| {
                        item.child(
                            div()
                                .id(("remove-repo", index))
                                .absolute()
                                .top(px(3.))
                                .right(px(3.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(18.))
                                .rounded(len(radius::RADIUS_XS))
                                .bg(theme::color(color::SURFACE))
                                .shadow(vec![gpui_kit::BoxShadow {
                                    color: theme::overlay(0.12),
                                    offset: gpui_kit::point(px(0.), px(0.)),
                                    blur_radius: px(0.),
                                    spread_radius: px(1.),
                                    inset: false,
                                }])
                                .invisible()
                                .group_hover(group, gpui_kit::Styled::visible)
                                .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                                .tooltip(chrome::tip("Close this repo here (its terminals move to the one showing)"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.remove_project(&removed, window, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.select_project(picked.clone(), window, cx))),
            );
        }
        // Adding a repo belongs with the repos, set apart so it doesn't read as one. Square: a
        // track item is otherwise at least a full control wide.
        let track = track.child(chrome::track_separator()).child(
            chrome::track_item("add-repo", false)
                .size(len(TITLEBAR_CONTROL))
                .min_w(len(TITLEBAR_CONTROL))
                .child(chrome::icon("plus", 13., theme::color(color::INK_MUTED)))
                .tooltip(chrome::tip_for("Add repo", OpenProject))
                .on_click(cx.listener(|_, _, window, cx| Self::open_project(window, cx))),
        );
        chrome::on_glass(track).into_any_element()
    }

    /// Sidebar | (editor / grid) | tickets, with draggable dividers. The panels stay in the same
    /// place in the tree when toggled, so terminals never restart. A maximised pane covers them all.
    fn render_workbench(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = self.panels.on_screen(self.layout.maximized.is_some());
        let editor_maximized = self.editor.read(cx).maximized();
        let half_gap = len(space::SPACE_2 / 2.);
        let name = self.project.file_name().map_or_else(|| "/".to_owned(), |name| name.to_string_lossy().into_owned());
        let grid = if self.layout.is_empty() {
            chrome::empty_grid(
                &name,
                cx.listener(|this, _, window, cx| {
                    this.open_terminal(Placement::Tab, window, cx);
                }),
            )
        } else {
            self.render_grid(cx)
        };
        let switch = self.render_sidebar_switch(cx);
        let root = self.current_root();
        let (files, changes) = (root.files.clone(), root.changes.clone());
        let header = self.render_sidebar_header(cx);
        // The Claude plan's limits; what each agent is doing shows in its own pane.
        let footer =
            self.settings.show_usage.then(|| crate::usage::render(cx).map(IntoElement::into_any_element)).flatten();
        let sidebar = match self.sidebar_tab {
            SidebarTab::Files => chrome::sidebar(header, switch, files, footer),
            SidebarTab::Changes => chrome::sidebar(header, switch, changes, footer),
        };
        let work = div().size_full().when(panels.sidebar, |main| main.pl(half_gap)).child(
            v_resizable("main")
                .with_handle_appearance(chrome::quiet_resize_handle())
                .child(
                    resizable_panel()
                        .size(px(260.))
                        .size_range(px(120.)..px(4000.))
                        .visible(panels.editor)
                        .child(div().size_full().pb(half_gap).child(self.editor.clone())),
                )
                .child(resizable_panel().visible(!(panels.editor && editor_maximized)).child(
                    div().size_full().flex().flex_col().when(panels.editor, |grid| grid.pt(half_gap)).child(grid),
                )),
        );
        // The tickets panel joins the row only while it's shown: a hidden panel still counts in the
        // row's sizes, and the sidebar's handle then jumped as it was grabbed.
        let tickets = self.tickets_view.clone().filter(|_| panels.tickets);
        let work = match tickets {
            Some(tickets) => h_resizable("work-and-tickets")
                .with_handle_appearance(chrome::quiet_resize_handle())
                .child(resizable_panel().child(div().size_full().pr(half_gap).child(work)))
                .child(
                    resizable_panel()
                        .size(px(300.))
                        .size_range(px(240.)..px(520.))
                        .child(div().size_full().pl(half_gap).child(tickets)),
                )
                .into_any_element(),
            None => work.into_any_element(),
        };
        div().flex_1().min_h_0().child(
            h_resizable("workbench")
                .with_handle_appearance(chrome::quiet_resize_handle())
                .child(
                    resizable_panel()
                        .size(px(270.))
                        .size_range(px(200.)..px(480.))
                        .visible(panels.sidebar)
                        .child(div().size_full().pr(half_gap).child(sidebar)),
                )
                .child(resizable_panel().child(work)),
        )
    }

    fn next_attention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.all_tabs();
        let start =
            self.layout.focused_tab().and_then(|front| order.iter().position(|t| *t == front)).map_or(0, |i| i + 1);
        let target = order
            .iter()
            .cycle()
            .skip(start)
            .take(order.len())
            .find(|tab| self.tab_state(**tab, cx).asks_for_you())
            .copied();
        if let Some(tab) = target {
            self.go_to_tab(tab, window, cx);
        }
    }

    fn focused_view(&self) -> Option<Entity<TerminalView>> {
        self.layout.focused_tab().and_then(|tab| self.terminals.get(&tab)).cloned()
    }
}

impl Workspace {
    /// Copy and paste: in the focused terminal, unless the keyboard is in a text field (the editor,
    /// a dialog's input), which gets them instead. ⌘V and ⌘C reach Saturn first, through the Edit
    /// menu, so without this a token pasted into `.env` would be typed into the agent's pane.
    fn clipboard_actions(element: gpui_kit::Div, cx: &mut Context<Self>) -> gpui_kit::Div {
        element
            .on_action(cx.listener(|this, _: &Copy, window, cx| match this.terminal_for_keys(window, cx) {
                Some(view) => {
                    view.update(cx, |view, cx| view.copy_selection(cx));
                }
                None => window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx),
            }))
            .on_action(cx.listener(|this, _: &Paste, window, cx| match this.terminal_for_keys(window, cx) {
                Some(view) => view.update(cx, TerminalView::paste_from_clipboard),
                None => window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx),
            }))
    }

    /// The terminal the keyboard is in: the focused pane's, when it has the keyboard or nothing
    /// else does (the workspace holds it after a dialog closes). `None` when something else has
    /// it, a text field say.
    fn terminal_for_keys(&self, window: &Window, cx: &gpui_kit::App) -> Option<Entity<TerminalView>> {
        let view = self.focused_view()?;
        let terminal_has_keys = view.read(cx).focus_handle(cx).is_focused(window);
        let nothing_else = window.focused(cx).is_none_or(|focused| focused == self.focus);
        (terminal_has_keys || nothing_else).then_some(view)
    }

    /// The palettes, and the views the command palette opens by name.
    fn palette_actions(element: gpui_kit::Div, cx: &mut Context<Self>) -> gpui_kit::Div {
        element
            .on_action(cx.listener(|this, _: &ToggleTickets, window, cx| this.toggle_tickets(window, cx)))
            .on_action(cx.listener(|this, _: &TextBigger, _, cx| this.change_text_size(Some(1), cx)))
            .on_action(cx.listener(|this, _: &TextSmaller, _, cx| this.change_text_size(Some(-1), cx)))
            .on_action(cx.listener(|this, _: &TextReset, _, cx| this.change_text_size(None, cx)))
            .on_action(cx.listener(|this, _: &GoToFile, window, cx| {
                palette::go_to_file(cx.weak_entity(), this.root.clone(), window, cx);
            }))
            .on_action(cx.listener(|_, _: &CommandPalette, window, cx| palette::commands(window, cx)))
            .on_action(cx.listener(|this, _: &ShowHistory, _, cx| this.show_history(cx)))
            .on_action(cx.listener(|this, _: &ShowImpact, _, cx| {
                let root = this.root.clone();
                this.show_impact(&root, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowSchema, _, cx| this.show_schema(cx)))
            .on_action(cx.listener(|this, _: &SetUpWorktree, window, cx| this.set_up_worktree(window, cx)))
            .on_action(cx.listener(|this, _: &StartApp, window, cx| this.start_app(window, cx)))
            .on_action(cx.listener(|this, _: &CreateRecipe, window, cx| this.create_recipe(window, cx)))
            .on_action(cx.listener(|this, _: &ShowWorktrees, window, cx| this.show_worktrees(window, cx)))
            .on_action(cx.listener(|this, _: &EditPrBody, window, cx| this.edit_pr_body(window, cx)))
            .on_action(cx.listener(|this, _: &ShowPictures, window, cx| this.show_pictures(window, cx)))
            .on_action(cx.listener(|this, _: &ShowPreview, window, cx| this.show_preview(window, cx)))
            .on_action(cx.listener(|this, _: &TeachRepo, window, cx| {
                let checkout = this.focused_checkout(cx);
                let asked = this.layout.focused_tab().map(|tab| this.typed_prompts(tab)).unwrap_or_default();
                crate::teach_view::open(cx.weak_entity(), checkout, asked, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowContext, window, cx| {
                let checkout = this.focused_checkout(cx);
                let folder = this
                    .layout
                    .focused_tab()
                    .and_then(|tab| this.terminals.get(&tab))
                    .map(|view| view.read(cx).cwd.clone())
                    .filter(|folder| folder.starts_with(&checkout))
                    .unwrap_or_else(|| checkout.clone());
                // The pane's own agent, else the one new panes start.
                let agent = this
                    .layout
                    .focused_tab()
                    .and_then(|tab| this.agent_in(tab))
                    .unwrap_or_else(|| crate::agent_choice::current(cx));
                crate::context_view::open(cx.weak_entity(), checkout, &folder, agent, window, cx);
            }))
            .on_action(cx.listener(|this, _: &RunCheck, window, cx| {
                let checkout = this.focused_checkout(cx);
                crate::palette::checks(cx.weak_entity(), checkout, window, cx);
            }))
            .on_action(cx.listener(|this, _: &BestOfN, window, cx| {
                let repo = git::main_worktree(&this.project).unwrap_or_else(|| this.project.clone());
                crate::best_of_view::start(cx.weak_entity(), &repo, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CompareAttempts, window, cx| {
                let repo = git::main_worktree(&this.project).unwrap_or_else(|| this.project.clone());
                crate::best_of_view::compare(cx.weak_entity(), repo, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowRoutines, window, cx| {
                let folder = this.focused_checkout(cx);
                crate::routines_view::open(cx.weak_entity(), folder, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SearchSessions, window, cx| {
                let open = this.sessions.iter().map(|(tab, (_, id))| (id.clone(), *tab)).collect();
                crate::sessions_view::open(cx.weak_entity(), open, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SendToSeveral, window, cx| {
                let panes = this.broadcast_panes(cx);
                crate::broadcast_view::open(cx.weak_entity(), panes, window, cx);
            }))
            .on_action(cx.listener(|_, _: &ShowWaiting, window, cx| {
                crate::queue_view::open(cx.weak_entity(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &CompareWith, window, cx| {
                crate::palette::compare(cx.weak_entity(), this.root.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowLineage, _, cx| this.show_lineage(cx)))
            .on_action(cx.listener(|this, _: &ShowProblems, _, cx| this.show_problems(cx)))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.report_attention(window, cx);
        // With nothing holding the keyboard (Saturn just came forward, a dialog just closed),
        // shortcuts would reach nothing: the workspace takes it.
        if window.focused(cx).is_none() {
            let focus = self.focus.clone();
            cx.defer_in(window, move |_, window, cx| {
                if window.focused(cx).is_none() {
                    window.focus(&focus, cx);
                }
            });
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .px(len(space::SPACE_2))
            // No fill: the title bar and the gaps are the window's glass (theme::overlay). The
            // status bar's card sets the space below.
            .text_color(theme::color(color::INK))
            .font_family(saturn_design::font::SANS)
            .track_focus(&self.focus)
            .key_context("Workspace")
            .map(|element| Self::palette_actions(element, cx))
            .map(|element| Self::clipboard_actions(element, cx))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.open_terminal(Placement::Tab, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                this.open_terminal(Placement::Right, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| {
                this.open_terminal(Placement::Down, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if let Some(tab) = this.layout.focused_tab() {
                    this.close_tab(tab, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &NextTab, window, cx| this.update_layout(window, cx, |l| l.cycle_tab(true))),
            )
            .on_action(
                cx.listener(|this, _: &PreviousTab, window, cx| this.update_layout(window, cx, |l| l.cycle_tab(false))),
            )
            .on_action(cx.listener(|this, _: &FocusLeft, window, cx| {
                this.update_layout(window, cx, |l| l.focus_direction(Direction::Left));
            }))
            .on_action(cx.listener(|this, _: &FocusRight, window, cx| {
                this.update_layout(window, cx, |l| l.focus_direction(Direction::Right));
            }))
            .on_action(cx.listener(|this, _: &FocusUp, window, cx| {
                this.update_layout(window, cx, |l| l.focus_direction(Direction::Up));
            }))
            .on_action(cx.listener(|this, _: &FocusDown, window, cx| {
                this.update_layout(window, cx, |l| l.focus_direction(Direction::Down));
            }))
            .on_action(cx.listener(|this, _: &ToggleMaximize, window, cx| {
                this.update_layout(window, cx, Layout::toggle_maximize);
            }))
            .on_action(cx.listener(|this, _: &EvenOutPanes, window, cx| {
                this.update_layout(window, cx, Layout::even_out);
            }))
            .on_action(cx.listener(|this, _: &NextAttention, window, cx| this.next_attention(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleEditor, _, cx| this.toggle_editor(cx)))
            .on_action(cx.listener(|_, _: &OpenProject, window, cx| Self::open_project(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFile, _, cx| this.save_file(cx)))
            .on_action(cx.listener(|this, _: &OpenTicketPicker, window, cx| {
                ticket_picker::open(cx.weak_entity(), this.clickup.connected(), window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &FocusPane1, window, cx| this.update_layout(window, cx, |l| l.focus_nth(1))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane2, window, cx| this.update_layout(window, cx, |l| l.focus_nth(2))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane3, window, cx| this.update_layout(window, cx, |l| l.focus_nth(3))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane4, window, cx| this.update_layout(window, cx, |l| l.focus_nth(4))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane5, window, cx| this.update_layout(window, cx, |l| l.focus_nth(5))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane6, window, cx| this.update_layout(window, cx, |l| l.focus_nth(6))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane7, window, cx| this.update_layout(window, cx, |l| l.focus_nth(7))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane8, window, cx| this.update_layout(window, cx, |l| l.focus_nth(8))),
            )
            .on_action(
                cx.listener(|this, _: &FocusPane9, window, cx| this.update_layout(window, cx, |l| l.focus_nth(9))),
            )
            .child(self.render_titlebar(cx))
            .child(self.render_workbench(cx))
            .child(self.status_bar.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, drop_side, even_divider, move_divider};
    use gpui_kit::{Bounds, point, px, size};

    #[test]
    fn a_dropped_tab_splits_on_the_nearest_edge_or_joins_in_the_middle() {
        let pane = Bounds::new(point(px(100.), px(50.)), size(px(400.), px(200.)));
        let at = |x: f32, y: f32| drop_side(pane, point(px(x), px(y)));
        assert_eq!(at(110., 150.), Some(Direction::Left));
        assert_eq!(at(490., 150.), Some(Direction::Right));
        assert_eq!(at(300., 60.), Some(Direction::Up));
        assert_eq!(at(300., 240.), Some(Direction::Down));
        assert_eq!(at(300., 150.), None);
    }

    #[test]
    fn moves_a_divider_and_keeps_the_pair_total() {
        let sizes = move_divider(&[1.0, 1.0, 2.0], 0, 0.125);
        assert!((sizes[0] - 0.5).abs() < 1e-6 && (sizes[1] - 1.5).abs() < 1e-6 && (sizes[2] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn evens_out_the_two_neighbours_of_a_divider() {
        let sizes = even_divider(&[1.0, 0.5, 1.5, 2.0], 1);
        assert!((sizes[1] - 1.0).abs() < 1e-6 && (sizes[2] - 1.0).abs() < 1e-6);
        assert!((sizes[0] - 1.0).abs() < 1e-6 && (sizes[3] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn keeps_both_neighbours_above_the_minimum() {
        let sizes = move_divider(&[1.0, 1.0], 0, 0.0);
        assert!((sizes[0] - 2.0 * 0.08).abs() < 1e-6);
        let sizes = move_divider(&[1.0, 1.0], 0, 1.0);
        assert!((sizes[1] - 2.0 * 0.08).abs() < 1e-6);
    }
}
