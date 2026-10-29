//! The Settings dialog (⌘,): sections on the left, under a search that looks through all of them;
//! the section's settings on the right, each a title, a sentence and its control.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, prelude::FluentBuilder as _, px,
};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::keymap::{self, SHORTCUTS, Shortcut};
use crate::settings::{AgentCommands, AppearanceSetting, Settings, TEXT_SIZES};
use crate::theme::{self, len};
use saturn_core::agents::Agent;

use crate::workspace::Workspace;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Appearance,
    Editor,
    Terminal,
    Agents,
    Keymap,
    ClickUp,
}

impl Section {
    const ALL: [Self; 7] =
        [Self::General, Self::Appearance, Self::Editor, Self::Terminal, Self::Agents, Self::Keymap, Self::ClickUp];

    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Editor => "Editor",
            Self::Terminal => "Terminal",
            Self::Agents => "Agents",
            Self::Keymap => "Keyboard shortcuts",
            Self::ClickUp => "ClickUp",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => "settings",
            Self::Appearance => "sun-moon",
            Self::Editor => "file-code",
            Self::Terminal => "square-terminal",
            Self::Agents => "bot",
            Self::Keymap => "keyboard",
            Self::ClickUp => "link-2",
        }
    }
}

/// One setting, as Settings lists it and search finds it.
struct Row {
    section: Section,
    title: SharedString,
    detail: SharedString,
    control: AnyElement,
    /// The control goes under the words, across the row (a text field).
    wide: bool,
    /// One line, the detail only for search (a shortcut, whose detail is its group's heading).
    compact: bool,
    /// The heading it sits under in its section (an agent's name), if any.
    group: Option<SharedString>,
}

impl Row {
    fn new(
        section: Section,
        title: impl Into<SharedString>,
        detail: impl Into<SharedString>,
        control: impl IntoElement,
    ) -> Self {
        Self {
            section,
            title: title.into(),
            detail: detail.into(),
            control: control.into_any_element(),
            wide: false,
            compact: false,
            group: None,
        }
    }

    fn under(mut self, group: &'static str) -> Self {
        self.group = Some(group.into());
        self
    }

    fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    fn wide(mut self) -> Self {
        self.wide = true;
        self
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.title.to_lowercase().contains(&query)
            || self.detail.to_lowercase().contains(&query)
            || self.section.name().to_lowercase().contains(&query)
    }
}

pub struct SettingsView {
    workspace: WeakEntity<Workspace>,
    section: Section,
    search: Entity<InputState>,
    token: Entity<InputState>,
    command: Entity<InputState>,
    codex_command: Entity<InputState>,
    /// What starts Claude Code and Codex (Settings → Agents).
    start_claude: Entity<InputState>,
    start_codex: Entity<InputState>,
    /// The shortcut waiting for its new keys, and what takes them before any shortcut fires.
    recording: Option<(&'static str, Subscription)>,
    /// What the last shortcut change did besides itself ("Taken from Close tab").
    note: Option<String>,
    /// Whether Claude Code shows Saturn's status line, read as Settings opens; and why the last
    /// change to it failed.
    status_line: bool,
    status_line_problem: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    /// The workspace is in the middle of an update when Settings opens, so nothing here reads it:
    /// the project's commands and what starts each agent come from the caller.
    fn new(
        workspace: WeakEntity<Workspace>,
        commands: (String, String),
        agent_commands: AgentCommands,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (ticket_command, codex_command) = commands;
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let token = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("pk_…"));
        let command = cx.new(|cx| InputState::new(window, cx).default_value(ticket_command));
        let codex_command = cx.new(|cx| InputState::new(window, cx).default_value(codex_command));
        let default = AgentCommands::default();
        let start_claude =
            cx.new(|cx| InputState::new(window, cx).placeholder(default.claude).default_value(agent_commands.claude));
        let start_codex =
            cx.new(|cx| InputState::new(window, cx).placeholder(default.codex).default_value(agent_commands.codex));
        search.update(cx, |search, cx| search.focus(window, cx));
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // The ticket command saves as it is typed.
            cx.subscribe(&command, |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                let _ = this.workspace.update(cx, |workspace, cx| {
                    let project = workspace.project_path().to_owned();
                    workspace.change_settings(
                        |settings| settings.projects.entry(project).or_default().ticket_command = value,
                        cx,
                    );
                });
            }),
            cx.subscribe(&codex_command, |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                let _ = this.workspace.update(cx, |workspace, cx| {
                    let project = workspace.project_path().to_owned();
                    workspace.change_settings(
                        |settings| settings.projects.entry(project).or_default().codex_command = value,
                        cx,
                    );
                });
            }),
            // An emptied field starts the agent by its own name again.
            cx.subscribe(&start_claude, |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().trim().to_owned();
                let value = if value.is_empty() { AgentCommands::default().claude } else { value };
                this.change(cx, |settings| settings.agent_commands.claude = value);
            }),
            cx.subscribe(&start_codex, |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().trim().to_owned();
                let value = if value.is_empty() { AgentCommands::default().codex } else { value };
                this.change(cx, |settings| settings.agent_commands.codex = value);
            }),
        ];
        let observed = workspace.upgrade().map(|workspace| cx.observe(&workspace, |_, _, cx| cx.notify()));
        // Spikes open a section by name (`SATURN_SETTINGS_SECTION=keyboard`).
        let section = std::env::var("SATURN_SETTINGS_SECTION")
            .ok()
            .and_then(|name| Section::ALL.into_iter().find(|section| section.name().to_lowercase().starts_with(&name)))
            .unwrap_or(Section::General);
        Self {
            workspace,
            section,
            search,
            token,
            command,
            codex_command,
            start_claude,
            start_codex,
            recording: None,
            note: None,
            status_line: crate::status_line::is_installed(),
            status_line_problem: None,
            _subscriptions: subscriptions.into_iter().chain(observed).collect(),
        }
    }

    fn change(&self, cx: &mut App, change: impl FnOnce(&mut Settings)) {
        let _ = self.workspace.update(cx, |workspace, cx| workspace.change_settings(change, cx));
    }

    /// An on/off switch for a setting.
    fn switch(&self, id: &'static str, on: bool, apply: fn(&mut Settings, bool)) -> impl IntoElement {
        let workspace = self.workspace.clone();
        Switch::new(id).checked(on).on_click(move |on: &bool, _, cx| {
            let on = *on;
            let _ = workspace.update(cx, |workspace, cx| workspace.change_settings(|settings| apply(settings, on), cx));
        })
    }

    /// A choice of a few values, as a track; the chosen one is raised.
    fn choice<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(&'static str, T)],
        current: T,
        apply: fn(&mut Settings, T),
    ) -> impl IntoElement {
        chrome::track().children(options.iter().map(|&(label, value)| {
            let workspace = self.workspace.clone();
            chrome::track_item(ElementId::Name(format!("{id}-{label}").into()), value == current)
                .px(len(space::SPACE_3))
                .child(label)
                .on_click(move |_, _, cx| {
                    let _ = workspace
                        .update(cx, |workspace, cx| workspace.change_settings(|settings| apply(settings, value), cx));
                })
        }))
    }

    fn general(&self, settings: &Settings) -> Vec<Row> {
        let version =
            format!("Saturn {}. It looks for a new version every hour and offers it.", env!("CARGO_PKG_VERSION"));
        vec![
            Row::new(
                Section::General,
                "Notifications",
                "When an agent needs you or finishes while Saturn is in the background, macOS shows a notification. Only in Saturn.app.",
                self.switch("notifications", settings.notifications, |settings, on| settings.notifications = on),
            ),
            Row::new(
                Section::General,
                "Menu bar icon",
                "Saturn in the menu bar, with how many agents are waiting for you, so you see it while Saturn is hidden. Clicking it takes you to the next one.",
                self.switch("menu-bar", settings.menu_bar, |settings, on| settings.menu_bar = on),
            ),
            Row::new(
                Section::General,
                "Share env files with worktrees",
                "A worktree's .env files point at the main checkout's, so a value added in either is in both. So do CLAUDE.local.md and the commands you allowed Claude for good. A worktree's own different file is left alone.",
                self.switch("share-env-files", settings.share_env_files, |settings, on| settings.share_env_files = on),
            ),
            Row::new(
                Section::General,
                "Updates",
                version,
                chrome::button("check-updates", "Check now", None, false)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::CheckForUpdates), cx)),
            ),
        ]
    }

    fn appearance(&self, settings: &Settings) -> Vec<Row> {
        let size = settings.text_size;
        let step = |id: &'static str, label: &'static str, by: i16, tip: &'static str| {
            let workspace = self.workspace.clone();
            let possible = TEXT_SIZES.contains(&size.saturating_add_signed(by));
            // Square, so − and + are the same size whatever their glyphs' widths.
            chrome::button(id, label, None, false)
                .w(len(chrome::CONTROL_HEIGHT))
                .px_0()
                .justify_center()
                .tooltip(chrome::tip(tip))
                .when(!possible, |button| button.opacity(0.4))
                .on_click(move |_, _, cx| {
                    let _ = workspace.update(cx, |workspace, cx| workspace.change_text_size(Some(by), cx));
                })
        };
        vec![
            Row::new(
                Section::Appearance,
                "Theme",
                "Light or dark, or whichever macOS is in.",
                self.choice(
                    "appearance",
                    &[
                        ("System", AppearanceSetting::System),
                        ("Light", AppearanceSetting::Light),
                        ("Dark", AppearanceSetting::Dark),
                    ],
                    settings.appearance,
                    |settings, value| settings.appearance = value,
                ),
            ),
            Row::new(
                Section::Appearance,
                "Text size",
                "Of terminals and files, in points; the rest of Saturn stays as it is. ⌘+ and ⌘− change it too, ⌘0 goes back to 13.",
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(step("text-smaller", "−", -1, "Smaller text"))
                    .child(div().w(px(28.)).text_center().text_size(px(13.)).child(size.to_string()))
                    .child(step("text-bigger", "+", 1, "Bigger text")),
            ),
            Row::new(
                Section::Appearance,
                "Plan usage",
                "The Claude plan's limits at the foot of the sidebar: the session, the week, and a model's own week.",
                self.switch("show-usage", settings.show_usage, |settings, on| settings.show_usage = on),
            ),
        ]
    }

    fn editor(&self, settings: &Settings) -> Vec<Row> {
        let editor = &settings.editor;
        vec![
            Row::new(
                Section::Editor,
                "Format on save",
                "Saving a file runs the project's own formatter on it (oxfmt, Prettier, Biome or rustfmt), only when the project has a config for one and has it installed.",
                self.switch("format-on-save", settings.format_on_save, |settings, on| settings.format_on_save = on),
            ),
            Row::new(
                Section::Editor,
                "Wrap long lines",
                "Lines longer than the panel wrap onto the next row instead of running off to the right.",
                self.switch("soft-wrap", editor.soft_wrap, |settings, on| settings.editor.soft_wrap = on),
            ),
            Row::new(
                Section::Editor,
                "Line numbers",
                "Numbers down the left edge of a file.",
                self.switch("line-numbers", editor.line_numbers, |settings, on| settings.editor.line_numbers = on),
            ),
            Row::new(
                Section::Editor,
                "Indent guides",
                "A faint line down each level of indentation.",
                self.switch("indent-guides", editor.indent_guides, |settings, on| settings.editor.indent_guides = on),
            ),
            Row::new(
                Section::Editor,
                "Show whitespace",
                "Spaces and tabs as faint dots and arrows.",
                self.switch("show-whitespace", editor.show_whitespace, |settings, on| {
                    settings.editor.show_whitespace = on;
                }),
            ),
            Row::new(
                Section::Editor,
                "Tab width",
                "How many columns Tab indents by. The project's formatter has the last word when it saves.",
                self.choice("tab-size", &[("2", 2), ("4", 4), ("8", 8)], editor.tab_size, |settings, value| {
                    settings.editor.tab_size = value;
                }),
            ),
        ]
    }

    fn terminal(&self, settings: &Settings) -> Vec<Row> {
        vec![
            Row::new(
                Section::Terminal,
                "Option key as Meta",
                "Option+key sends Escape and the key, as shells and editors expect for Alt (Option+B jumps back a word). Off: Option types what the keyboard layout puts on it, like @ or { on many layouts.",
                self.switch("option-as-meta", settings.terminal.option_as_meta, |settings, on| {
                    settings.terminal.option_as_meta = on;
                }),
            ),
            Row::new(
                Section::Terminal,
                "Copy on select",
                "Selecting text copies it when you let go of the mouse.",
                self.switch("copy-on-select", settings.terminal.copy_on_select, |settings, on| {
                    settings.terminal.copy_on_select = on;
                }),
            ),
        ]
    }

    /// Which agent new panes start, when there are two to pick from.
    fn new_agents_start(cx: &mut Context<Self>) -> Option<Row> {
        crate::agent_choice::switch("settings-agent", cx).map(|switch| {
            Row::new(
                Section::Agents,
                "New agents start",
                "New agent, Best of N and routines start this one, and remember what you pick there.",
                switch,
            )
        })
    }

    /// What Saturn found of `agent` on this Mac, the first line under its heading.
    fn found(agent: Agent, cx: &mut Context<Self>) -> Row {
        let found = cx.try_global::<crate::agent_choice::Agents>().and_then(|agents| agents.found.clone());
        let state = found.as_ref().and_then(|found| found.iter().find(|found| found.agent == agent).copied());
        let (detail, status) = match state {
            None => ("Looking for it…".to_owned(), "…"),
            Some(found) if found.ready() => {
                (format!("Installed and logged in: Saturn can start it ({}).", agent.program()), "Ready")
            }
            Some(found) if found.installed => {
                (format!("Installed, not logged in. Run {} in a terminal to log in.", agent.program()), "Not logged in")
            }
            Some(_) => (format!("Not on this Mac. Install it to run {} agents here.", agent.name()), "Not installed"),
        };
        Row::new(
            Section::Agents,
            "On this Mac",
            detail,
            div().text_size(px(13.)).text_color(theme::color(color::INK_MUTED)).child(status),
        )
        .under(agent.name())
    }

    /// What starts Codex and its status line, when Codex is on this Mac.
    fn codex(&self, settings: &Settings, cx: &mut Context<Self>) -> Vec<Row> {
        if !crate::agent_choice::ready(cx).contains(&Agent::Codex) {
            return Vec::new();
        }
        vec![
            Row::new(
                Section::Agents,
                "Start Codex with",
                "What every Codex agent Saturn starts or resumes is started with: codex, or your own alias. Saturn types what it needs after it, its profile too (-p saturn), so it still hears what Codex does.",
                Input::new(&self.start_codex),
            )
            .wide(),
            Row::new(
                Section::Agents,
                "Status line in Codex",
                "Under Codex's prompt, in the Codex sessions Saturn starts: the model, the folder and branch, how full the context is, and the plan's limits. Your own Codex config isn't touched.",
                self.switch("codex-status-line", settings.codex_status_line, |settings, on| {
                    settings.codex_status_line = on;
                }),
            ),
        ]
    }

    fn claude(&self, settings: &Settings, cx: &mut Context<Self>) -> Vec<Row> {
        let detail = format!(
            "Under Claude Code's prompt, in Saturn and in any other terminal: the model, how full the context is, the step when Claude keeps a task list, and how much of the session's limit is used (in Saturn, with this session's part of it). Turning it on keeps the status line you had, to put back when you turn it off.{}",
            self.status_line_problem.as_ref().map(|problem| format!(" {problem}")).unwrap_or_default()
        );
        vec![
            Row::new(
                Section::Agents,
                "Start Claude Code with",
                "What every Claude Code agent Saturn starts or resumes is started with: claude, or your own alias (cdsk, say). Saturn types what it needs after it, so a resumed session is cdsk --resume ….",
                Input::new(&self.start_claude),
            )
            .wide(),
            Row::new(
                Section::Agents,
                "Status line in Claude Code",
                detail,
                Switch::new("status-line").checked(self.status_line).on_click(cx.listener(|this, on: &bool, _, cx| {
                    match crate::status_line::set_installed(*on) {
                        Ok(()) => {
                            this.status_line = *on;
                            this.status_line_problem = None;
                        }
                        Err(problem) => this.status_line_problem = Some(problem),
                    }
                    cx.notify();
                })),
            ),
            Row::new(
                Section::Agents,
                "Name panes after Claude's session",
                "A pane shows the session's name, from /rename or set automatically. Off: it shows its folder, or a name you gave it by double-clicking its title.",
                self.switch("name-tabs", settings.name_tabs_after_sessions, |settings, on| {
                    settings.name_tabs_after_sessions = on;
                }),
            ),
            Row::new(
                Section::Agents,
                "Name sessions after tickets",
                "A slash command whose first word is a ticket, like /start abc-224, renames the session to ABC-224, as /rename would. A name you gave yourself is never replaced.",
                self.switch("rename-sessions", settings.rename_sessions_from_commands, |settings, on| {
                    settings.rename_sessions_from_commands = on;
                }),
            ),
        ]
    }

    fn shortcut_row(&self, shortcut: &'static Shortcut, settings: &Settings, cx: &mut Context<Self>) -> Row {
        let recording = self.recording.as_ref().is_some_and(|(id, _)| *id == shortcut.id);
        let keys = keymap::keys_of(shortcut, &settings.keymap);
        let changed = settings.keymap.contains_key(shortcut.id);
        let label = if recording {
            "Press the keys…".to_owned()
        } else if keys.is_empty() {
            "None".to_owned()
        } else {
            keys.iter().map(|keys| keymap::symbols(keys)).collect::<Vec<_>>().join("  ")
        };
        let keys_button = div()
            .id(ElementId::Name(format!("keys-{}", shortcut.id).into()))
            .flex()
            .items_center()
            .h(len(chrome::CONTROL_HEIGHT))
            .min_w(px(88.))
            .justify_center()
            .px(len(space::SPACE_3))
            .rounded(len(radius::RADIUS_SM))
            .cursor_pointer()
            .text_size(px(13.))
            .map(|button| {
                if recording {
                    button.bg(theme::color(color::BLUE).opacity(0.16)).text_color(theme::color(color::BLUE))
                } else {
                    button
                        .bg(theme::fill_control())
                        .hover(|s| s.bg(theme::fill_pressed()))
                        .text_color(theme::color(if keys.is_empty() { color::INK_FAINT } else { color::INK }))
                }
            })
            .child(label)
            .tooltip(chrome::tip(if recording {
                "Press the new keys; Escape keeps the old ones, Backspace leaves it without keys"
            } else {
                "Click, then press the keys you want"
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.record(shortcut, cx)));
        let reset = changed.then(|| {
            chrome::icon_button(ElementId::Name(format!("reset-{}", shortcut.id).into()), "rotate-ccw", true)
                .tooltip(chrome::tip("Back to the keys Saturn came with"))
                .on_click(cx.listener(move |this, _, _, cx| this.set_keys(shortcut, None, cx)))
        });
        Row::new(
            Section::Keymap,
            shortcut.name,
            shortcut.group,
            div().flex().items_center().gap(len(space::SPACE_1)).children(reset).child(keys_button),
        )
        .compact()
    }

    /// Waits for the keys to give `shortcut`, catching them before any shortcut runs.
    fn record(&mut self, shortcut: &'static Shortcut, cx: &mut Context<Self>) {
        let view = cx.weak_entity();
        let interceptor = cx.intercept_keystrokes(move |event, _, cx| {
            let keystroke = event.keystroke.clone();
            match keystroke.key.as_str() {
                "escape" if !keystroke.modifiers.modified() => {
                    let _ = view.update(cx, |this, cx| {
                        this.recording = None;
                        cx.notify();
                    });
                }
                "backspace" | "delete" if !keystroke.modifiers.modified() => {
                    let _ = view.update(cx, |this, cx| this.set_keys(shortcut, Some(String::new()), cx));
                }
                _ => match keymap::keys_for(&keystroke) {
                    Some(keys) => {
                        let _ = view.update(cx, |this, cx| this.set_keys(shortcut, Some(keys), cx));
                    }
                    // A lone modifier or a plain letter: wait for the whole shortcut.
                    None => return,
                },
            }
            cx.stop_propagation();
        });
        self.recording = Some((shortcut.id, interceptor));
        self.note = None;
        cx.notify();
    }

    /// Gives `shortcut` new keys (`None`: its own again; empty: none), taking them from any other
    /// shortcut that had them.
    fn set_keys(&mut self, shortcut: &'static Shortcut, keys: Option<String>, cx: &mut Context<Self>) {
        self.recording = None;
        let Some(workspace) = self.workspace.upgrade() else { return };
        let before = workspace.read(cx).settings().keymap.clone();
        let keys_now = keys.clone().unwrap_or_else(|| shortcut.keys.first().copied().unwrap_or_default().to_owned());
        let other = (!keys_now.is_empty()).then(|| keymap::taken_by(&keys_now, shortcut.id, &before)).flatten();
        self.note =
            other.map(|other| format!("{} was on {}; it has no keys now.", other.name, keymap::symbols(&keys_now)));
        self.change(cx, |settings| {
            match keys {
                Some(keys) => settings.keymap.insert(shortcut.id.to_owned(), keys),
                None => settings.keymap.remove(shortcut.id),
            };
            if let Some(other) = other {
                settings.keymap.insert(other.id.to_owned(), String::new());
            }
        });
        let after = workspace.read(cx).settings().keymap.clone();
        keymap::rebind(shortcut, &keymap::keys_of(shortcut, &before), &after, cx);
        if let Some(other) = other {
            keymap::rebind(other, &keymap::keys_of(other, &before), &after, cx);
        }
        crate::set_menus(cx);
        cx.notify();
    }

    fn clickup(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let Some(workspace) = self.workspace.upgrade() else { return Vec::new() };
        let workspace_view = workspace.read(cx);
        let state = workspace_view.clickup();
        let project = workspace_view.project_path().to_owned();
        let create_worktree = workspace_view.settings().project(&project).create_worktree;
        let project_name = project
            .file_name()
            .map_or_else(|| project.display().to_string(), |name| name.to_string_lossy().into_owned());
        let status = state
            .problem
            .clone()
            .or_else(|| state.loading.then(|| "Checking with ClickUp…".to_owned()))
            .unwrap_or_default();
        if !state.connected() {
            let connect = {
                let (workspace, token) = (self.workspace.clone(), self.token.clone());
                chrome::button("clickup-connect", "Connect", None, true).on_click(move |_, _, cx| {
                    let value = token.read(cx).value().trim().to_owned();
                    if !value.is_empty() {
                        let _ = workspace.update(cx, |workspace, cx| workspace.connect_clickup(value, cx));
                    }
                })
            };
            return vec![
                Row::new(
                    Section::ClickUp,
                    "Connect ClickUp",
                    format!(
                        "Start agents from your tickets and see them in the Tickets panel. Paste a personal API token (ClickUp → Settings → Apps); Saturn keeps it in a file only you can read. {status}"
                    ),
                    div()
                        .flex()
                        .items_center()
                        .gap(len(space::SPACE_2))
                        .child(div().flex_1().child(Input::new(&self.token)))
                        .child(connect),
                )
                .wide(),
            ];
        }
        let account =
            state.account.clone().map_or_else(|| "Connected.".to_owned(), |name| format!("Connected to {name}."));
        let disconnect = {
            let workspace = self.workspace.clone();
            chrome::button("clickup-disconnect", "Disconnect", None, false).on_click(move |_, _, cx| {
                let _ = workspace.update(cx, Workspace::disconnect_clickup);
            })
        };
        let worktree = {
            let workspace = self.workspace.clone();
            Switch::new("ticket-worktree").checked(create_worktree).on_click(move |on: &bool, _, cx| {
                let (on, project) = (*on, project.clone());
                let _ = workspace.update(cx, |workspace, cx| {
                    workspace.change_settings(
                        |settings| settings.projects.entry(project).or_default().create_worktree = on,
                        cx,
                    );
                });
            })
        };
        vec![
            Row::new(
                Section::ClickUp,
                "ClickUp",
                format!("{account} ⌘⇧N starts an agent on one of your tickets. {status}"),
                disconnect,
            ),
            Row::new(
                Section::ClickUp,
                format!("Starting Claude Code on a ticket in {project_name}"),
                "Runs in the new pane. {ticket} becomes the ticket's id (ABC-236), {title} its title. To have the agent start straight away, give it a first message: claude -n {ticket} \"/start {ticket}\".",
                Input::new(&self.command),
            )
            .wide(),
            Row::new(
                Section::ClickUp,
                format!("Starting Codex on a ticket in {project_name}"),
                "The same for Codex, which has no flag to name its session: codex \"Work on {ticket}: {title}\".",
                Input::new(&self.codex_command),
            )
            .wide(),
            Row::new(
                Section::ClickUp,
                "A worktree per ticket",
                "Saturn makes a git worktree next to the project, on a branch named after the ticket, and opens the pane there. Off when the command sets up its own.",
                worktree,
            ),
        ]
    }

    fn rows(&self, cx: &mut Context<Self>) -> Vec<Row> {
        let settings =
            self.workspace.upgrade().map(|workspace| workspace.read(cx).settings().clone()).unwrap_or_default();
        let mut rows = self.general(&settings);
        rows.extend(self.appearance(&settings));
        rows.extend(self.editor(&settings));
        rows.extend(self.terminal(&settings));
        // Agents: which one new panes start, then each agent under its own name.
        rows.extend(Self::new_agents_start(cx));
        rows.push(Self::found(Agent::Claude, cx));
        rows.extend(self.claude(&settings, cx).into_iter().map(|row| row.under(Agent::Claude.name())));
        rows.push(Self::found(Agent::Codex, cx));
        rows.extend(self.codex(&settings, cx).into_iter().map(|row| row.under(Agent::Codex.name())));
        rows.extend(SHORTCUTS.iter().map(|shortcut| self.shortcut_row(shortcut, &settings, cx)));
        rows.extend(self.clickup(cx));
        rows
    }

    fn render_row(row: Row, index: usize) -> AnyElement {
        let words = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(2.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(row.title))
            .when(!row.detail.is_empty() && !row.compact, |words| {
                words.child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(17.))
                        .text_color(theme::color(color::INK_MUTED))
                        .child(row.detail),
                )
            });
        let line = div()
            .id(("setting", index))
            .py(len(if row.compact { space::SPACE_2 } else { space::SPACE_3 }))
            .when(index > 0, |line| line.border_t_1().border_color(theme::separator()));
        if row.wide {
            line.flex().flex_col().gap(len(space::SPACE_2)).child(words).child(row.control).into_any_element()
        } else {
            line.flex()
                .items_center()
                .justify_between()
                .gap(len(space::SPACE_6))
                .child(words)
                .child(div().flex_none().child(row.control))
                .into_any_element()
        }
    }

    fn render_nav(&self, searching: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = div().flex().flex_col().gap(px(2.));
        for section in Section::ALL {
            let selected = !searching && section == self.section;
            nav = nav.child(
                chrome::track_item(ElementId::Name(format!("section-{}", section.name()).into()), selected)
                    .justify_start()
                    .gap(len(space::SPACE_2))
                    .px(len(space::SPACE_2))
                    .child(chrome::icon(
                        section.icon(),
                        14.,
                        theme::color(if selected { color::INK } else { color::INK_MUTED }),
                    ))
                    .child(section.name())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.section = section;
                        this.search.update(cx, |search, cx| search.set_value("", window, cx));
                        cx.notify();
                    })),
            );
        }
        nav
    }
}

/// A small heading over a group of rows: a section's name in search, a shortcut group.
fn heading(text: &str, first: bool) -> impl IntoElement {
    div()
        .pt(len(if first { space::SPACE_1 } else { space::SPACE_4 }))
        .pb(len(space::SPACE_1))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_FAINT))
        .child(text.to_uppercase())
}

type Content = gpui_kit::Stateful<gpui_kit::Div>;

impl SettingsView {
    /// What a search found, under the names of the sections it's in.
    fn render_found(mut content: Content, rows: Vec<Row>, query: &str) -> Content {
        let mut current: Option<Section> = None;
        let mut index = 0;
        let mut found = false;
        for row in rows.into_iter().filter(|row| row.matches(query)) {
            if current != Some(row.section) {
                content = content.child(heading(row.section.name(), current.is_none()));
                current = Some(row.section);
                index = 0;
            }
            content = content.child(Self::render_row(row, index));
            index += 1;
            found = true;
        }
        if !found {
            content = content.child(
                div()
                    .pt(len(space::SPACE_4))
                    .text_size(px(13.))
                    .text_color(theme::color(color::INK_MUTED))
                    .child("No setting matches."),
            );
        }
        content
    }

    /// The chosen section: its name, then its settings, shortcuts under their group's heading.
    fn render_section(&self, mut content: Content, rows: Vec<Row>) -> Content {
        let muted = |text: String, tint| {
            div().pb(len(space::SPACE_2)).text_size(px(12.)).text_color(theme::color(tint)).child(text)
        };
        content = content
            .child(div().pb(len(space::SPACE_2)).text_size(px(20.)).line_height(px(24.)).child(self.section.name()));
        if self.section == Section::Keymap {
            content = content
                .child(muted(
                    "Click a shortcut, then press the keys you want. Copy, paste and quit stay ⌘C, ⌘V and ⌘Q."
                        .to_owned(),
                    color::INK_MUTED,
                ))
                .children(self.note.clone().map(|note| muted(note, color::ORANGE)));
        }
        let mut group: Option<SharedString> = None;
        let mut index = 0;
        for row in rows.into_iter().filter(|row| row.section == self.section) {
            let under = if row.compact { Some(row.detail.clone()) } else { row.group.clone() };
            if let Some(under) = under.filter(|under| group.as_ref() != Some(under)) {
                content = content.child(heading(&under, false));
                group = Some(under);
                index = 0;
            }
            content = content.child(Self::render_row(row, index));
            index += 1;
        }
        content
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().trim().to_owned();
        let searching = !query.is_empty();
        let rows = self.rows(cx);
        let content =
            // It clips at its edges: a few points of room on the left, taken back by the margin, keep
            // a focused field's ring whole.
            div()
                .id("settings-content")
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .ml(px(-4.))
                .pl(px(4.))
                .pr(len(space::SPACE_2));
        let content =
            if searching { Self::render_found(content, rows, &query) } else { self.render_section(content, rows) };
        let search = div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(len(chrome::CONTROL_HEIGHT))
            .px(len(space::SPACE_2))
            .rounded(len(radius::RADIUS_SM))
            .bg(theme::fill_control())
            .child(chrome::icon("search", 13., theme::color(color::INK_FAINT)))
            .child(div().flex_1().child(Input::new(&self.search).appearance(false)));
        div()
            .flex()
            .w_full()
            .h(px(540.))
            .gap(len(space::SPACE_5))
            .child(
                div()
                    .flex_none()
                    .w(px(196.))
                    .flex()
                    .flex_col()
                    .gap(len(space::SPACE_3))
                    .child(search)
                    .child(self.render_nav(searching, cx)),
            )
            .child(div().w(px(1.)).h_full().bg(theme::separator()))
            .child(content)
    }
}

/// Opens Settings. `commands` are the project's current ones (Claude Code's, Codex's): the caller
/// passes them, since the workspace that opens the dialog is in the middle of an update and can't be
/// read.
pub fn open(
    workspace: WeakEntity<Workspace>,
    commands: (String, String),
    agent_commands: AgentCommands,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| SettingsView::new(workspace, commands, agent_commands, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Settings")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(820.))
            .child(view.clone())
    });
}
