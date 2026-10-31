//! What Saturn remembers between launches, in `~/Library/Application Support/Saturn/settings.json`.

use std::path::{Path, PathBuf};

use saturn_core::agents::Agent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "independent on/off switches, each its own JSON field")]
pub struct Settings {
    /// Projects opened before, most recent first, for the welcome screen.
    pub recent_projects: Vec<PathBuf>,
    /// The most recent project in settings written before `recent_projects` existed.
    #[serde(skip_serializing)]
    last_project: Option<PathBuf>,
    /// Tabs show the Claude session's name (or the terminal's title). Off: the folder, or a
    /// name typed by hand.
    pub name_tabs_after_sessions: bool,
    /// A slash command whose first word is a ticket (`/start abc-224`) names the session after it.
    pub rename_sessions_from_commands: bool,
    /// Saving a file runs the project's own formatter on it, when the project has one.
    pub format_on_save: bool,
    /// A notification when an agent needs you or finishes while Saturn is in the background.
    pub notifications: bool,
    /// Saturn's item in the menu bar, with how many agents are waiting for you.
    pub menu_bar: bool,
    pub appearance: AppearanceSetting,
    /// The theme, by its id in design/tokens.json (`saturn`, `cyberdeck`); an unknown one draws as
    /// the first.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// The text size of terminals and files, in points (⌘+, ⌘−, ⌘0).
    pub text_size: u16,
    /// What starting an agent on a ticket does, per project folder.
    pub projects: std::collections::HashMap<PathBuf, ProjectSettings>,
    /// Shortcuts the user changed, by id (`keymap::SHORTCUTS`): their keys, or empty for none.
    pub keymap: std::collections::BTreeMap<String, String>,
    pub editor: EditorSettings,
    pub terminal: TerminalSettings,
    /// The Claude plan's limits at the foot of the sidebar.
    pub show_usage: bool,
    /// Which tickets the Tickets panel shows.
    pub ticket_filter: saturn_core::clickup::Filter,
    /// A worktree's env files are links to the main checkout's, so every worktree has the same
    /// values (`saturn_core::shared_env`).
    pub share_env_files: bool,
    /// The panels showing when Saturn last quit or one was shown or hidden; new windows open
    /// with the same.
    pub panels: Panels,
    /// The agent picked last (Claude Code or Codex), which new panes start while it's ready.
    pub agent: Option<Agent>,
    /// Codex shows its status line (model, folder, branch, context, limits) in the sessions Saturn
    /// starts.
    pub codex_status_line: bool,
    /// What starts each agent: its own name, or the person's alias for it (`cdsk`).
    pub agent_commands: AgentCommands,
    /// Claude Code's resume command, as 0.1.29 kept it (`cdsk --resume`): read once into
    /// `agent_commands`.
    #[serde(skip_serializing)]
    claude_resume: Option<String>,
}

/// What Saturn types to start each agent, before what it adds itself (a first message,
/// `--resume …`, Codex's `-p saturn`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentCommands {
    pub claude: String,
    pub codex: String,
}

impl Default for AgentCommands {
    fn default() -> Self {
        Self { claude: Agent::Claude.program().to_owned(), codex: Agent::Codex.program().to_owned() }
    }
}

impl AgentCommands {
    pub fn get(&self, agent: Agent) -> &str {
        match agent {
            Agent::Claude => &self.claude,
            Agent::Codex => &self.codex,
        }
    }
}

/// The window's panels around the grid, and whether each is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panels {
    /// The files and changes, on the left (⌘B).
    pub sidebar: bool,
    /// The editor, above the grid (⌘J).
    pub editor: bool,
    /// The ClickUp tickets, on the right (⌘⌥B).
    pub tickets: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self { sidebar: true, editor: true, tickets: false }
    }
}

/// How files look in the editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "independent on/off switches, each its own JSON field")]
pub struct EditorSettings {
    /// Long lines wrap at the panel's edge instead of scrolling sideways.
    pub soft_wrap: bool,
    pub line_numbers: bool,
    /// Faint lines down each level of indentation.
    pub indent_guides: bool,
    /// Spaces and tabs drawn as faint dots and arrows.
    pub show_whitespace: bool,
    /// Columns per indent, where the file doesn't say.
    pub tab_size: u8,
}

/// The editor settings in force, for every editor in every window.
impl gpui_kit::Global for EditorSettings {}

impl Default for EditorSettings {
    fn default() -> Self {
        Self { soft_wrap: false, line_numbers: true, indent_guides: true, show_whitespace: false, tab_size: 2 }
    }
}

/// How the terminals behave.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Option sends Escape before the key (Meta, as shells and editors expect) instead of typing
    /// the character the keyboard layout puts on it.
    pub option_as_meta: bool,
    /// Selecting text copies it, as soon as the mouse is let go.
    pub copy_on_select: bool,
}

/// How a project starts an agent on a ClickUp ticket. Kept in Saturn's settings, never in the
/// project's repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    /// Run in the new pane to start Claude Code, with `{ticket}` (ABC-236) and `{title}` filled in.
    pub ticket_command: String,
    /// The same, to start Codex.
    pub codex_command: String,
    /// Saturn makes a git worktree per ticket and opens the pane in it. Off when the command sets
    /// up its own (a skill that creates worktrees, for example).
    pub create_worktree: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            ticket_command: Agent::Claude.ticket_command().to_owned(),
            codex_command: Agent::Codex.ticket_command().to_owned(),
            create_worktree: true,
        }
    }
}

impl ProjectSettings {
    /// What starts `agent` on a ticket, as the project has it.
    pub fn command(&self, agent: Agent) -> &str {
        match agent {
            Agent::Claude => &self.ticket_command,
            Agent::Codex => &self.codex_command,
        }
    }

    /// The command that starts `agent` on `ticket`, quoted for the shell where filled in.
    pub fn command_for(&self, agent: Agent, ticket: &str, title: &str) -> String {
        self.command(agent).replace("{ticket}", &shell_quote(ticket)).replace("{title}", &shell_quote(title))
    }
}

/// `text` as one shell word: as is when it is plain, else in single quotes.
pub fn shell_quote(text: &str) -> String {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@".contains(c)) {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// Light or dark, or whichever macOS is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppearanceSetting {
    #[default]
    System,
    Light,
    Dark,
}

impl AppearanceSetting {
    /// The appearance to draw in, given the one macOS is in.
    pub fn resolve(self, system: saturn_design::Appearance) -> saturn_design::Appearance {
        match self {
            Self::System => system,
            Self::Light => saturn_design::Appearance::Light,
            Self::Dark => saturn_design::Appearance::Dark,
        }
    }
}

/// The first theme, Saturn's own.
fn default_theme() -> String {
    saturn_design::THEMES[0].id.to_owned()
}

/// The text size terminals and files start at, and go back to with ⌘0.
pub const TEXT_SIZE: u16 = 13;

/// The smallest and largest text sizes ⌘− and ⌘+ go to.
pub const TEXT_SIZES: std::ops::RangeInclusive<u16> = 9..=24;

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent_projects: Vec::new(),
            last_project: None,
            name_tabs_after_sessions: true,
            rename_sessions_from_commands: true,
            format_on_save: true,
            notifications: true,
            menu_bar: true,
            appearance: AppearanceSetting::System,
            theme: default_theme(),
            text_size: TEXT_SIZE,
            projects: std::collections::HashMap::new(),
            keymap: std::collections::BTreeMap::new(),
            editor: EditorSettings::default(),
            terminal: TerminalSettings::default(),
            show_usage: true,
            ticket_filter: saturn_core::clickup::Filter::default(),
            share_env_files: true,
            panels: Panels::default(),
            agent: None,
            codex_status_line: false,
            agent_commands: AgentCommands::default(),
            claude_resume: None,
        }
    }
}

/// Where Saturn keeps what it remembers. Tests and spikes point it at their own folder.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SATURN_USER_DATA") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Saturn"))
}

fn file() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("settings.json"))
}

pub fn load() -> Settings {
    file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .map(upgrade)
        .unwrap_or_default()
}

/// Settings written by an older Saturn, in today's terms.
fn upgrade(mut settings: Settings) -> Settings {
    // 0.1.29 kept only Claude Code's resume command; what comes before `--resume` starts it.
    if let Some(resume) = settings.claude_resume.take()
        && let Some(command) = resume.trim().strip_suffix("--resume").map(str::trim)
        && !command.is_empty()
        && settings.agent_commands == AgentCommands::default()
    {
        command.clone_into(&mut settings.agent_commands.claude);
    }
    settings
}

impl Settings {
    /// How `project` starts agents on tickets.
    pub fn project(&self, project: &Path) -> ProjectSettings {
        self.projects.get(project).cloned().unwrap_or_default()
    }
}

/// How many projects the welcome screen offers.
const RECENT: usize = 8;

impl Settings {
    /// Recent projects that still exist, most recent first.
    pub fn recent(&self) -> Vec<PathBuf> {
        let mut recent = self.recent_projects.clone();
        if recent.is_empty() {
            recent.extend(self.last_project.clone());
        }
        recent.retain(|path| path.is_dir());
        recent
    }

    fn remember(&mut self, project: &Path) {
        let mut recent = self.recent();
        recent.retain(|path| path != project);
        recent.insert(0, project.to_owned());
        recent.truncate(RECENT);
        self.recent_projects = recent;
    }
}

pub fn remember_project(project: &Path) {
    let mut settings = load();
    settings.remember(project);
    save(&settings);
}

pub fn save(settings: &Settings) {
    let Some(path) = file() else { return };
    if let Some(dir) = path.parent()
        && std::fs::create_dir_all(dir).is_ok()
        && let Ok(text) = serde_json::to_string_pretty(settings)
    {
        let _ = std::fs::write(path, text);
    }
}

/// The folder to open straight away: `SATURN_PROJECT`, else the folder Saturn was started in from
/// a terminal. `None` (started from Finder, the Dock or Spotlight, which start apps in `/`) shows
/// the welcome screen instead.
pub fn startup_project() -> Option<PathBuf> {
    if let Some(project) = std::env::var_os("SATURN_PROJECT") {
        return Some(PathBuf::from(project));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    std::env::current_dir().ok().filter(|dir| dir != Path::new("/") && Some(dir) != home.as_ref())
}

#[cfg(test)]
mod tests {
    use saturn_design::Appearance;

    use super::*;

    #[test]
    fn appearance_follows_the_system_unless_chosen() {
        assert_eq!(AppearanceSetting::System.resolve(Appearance::Light), Appearance::Light);
        assert_eq!(AppearanceSetting::Dark.resolve(Appearance::Light), Appearance::Dark);
        assert_eq!(AppearanceSetting::Light.resolve(Appearance::Dark), Appearance::Light);
    }

    #[test]
    fn settings_saved_before_appearance_existed_still_load() {
        let settings: Settings = serde_json::from_str(r#"{"name_tabs_after_sessions": false}"#).expect("loads");
        assert_eq!(settings.appearance, AppearanceSetting::System);
        assert!(!settings.name_tabs_after_sessions);
    }

    #[test]
    fn a_resume_command_of_your_own_becomes_what_starts_claude() {
        let settings: Settings = serde_json::from_str(r#"{"claude_resume": "cdsk --resume"}"#).expect("loads");
        let settings = upgrade(settings);
        assert_eq!(
            (settings.agent_commands.claude.as_str(), settings.agent_commands.codex.as_str()),
            ("cdsk", "codex")
        );
        let plain: Settings = serde_json::from_str(r#"{"claude_resume": "claude --resume"}"#).expect("loads");
        assert_eq!(upgrade(plain).agent_commands, AgentCommands::default());
        assert!(!serde_json::to_string(&settings).expect("saves").contains("claude_resume"));
    }

    #[test]
    fn panels_open_as_they_were_left() {
        let before: Settings = serde_json::from_str("{}").expect("loads");
        assert_eq!(before.panels, Panels { sidebar: true, editor: true, tickets: false });
        let closed: Settings = serde_json::from_str(r#"{"panels": {"editor": false}}"#).expect("loads");
        assert_eq!(closed.panels, Panels { sidebar: true, editor: false, tickets: false });
    }

    #[test]
    fn recent_projects_keep_the_latest_first_without_duplicates() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).expect("a");
        std::fs::create_dir_all(&b).expect("b");
        let mut settings = Settings::default();
        settings.remember(&a);
        settings.remember(&b);
        settings.remember(&a);
        assert_eq!(settings.recent(), [a.clone(), b]);
    }

    #[test]
    fn an_old_last_project_becomes_the_first_recent_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let text = format!(r#"{{"last_project": {:?}}}"#, dir.path());
        let settings: Settings = serde_json::from_str(&text).expect("loads");
        assert_eq!(settings.recent(), [dir.path().to_owned()]);
    }

    #[test]
    fn ticket_commands_are_filled_in_and_quoted() {
        let project = ProjectSettings {
            ticket_command: "claude -n {ticket} \"/start {ticket}\"".to_owned(),
            create_worktree: false,
            ..ProjectSettings::default()
        };
        assert_eq!(project.command_for(Agent::Claude, "ABC-236", "x"), "claude -n ABC-236 \"/start ABC-236\"");
        assert_eq!(project.command_for(Agent::Codex, "ABC-236", "x"), "codex");
        let titled = ProjectSettings { ticket_command: "echo {title}".to_owned(), ..ProjectSettings::default() };
        assert_eq!(titled.command_for(Agent::Claude, "ABC-1", "Don't break; rm -rf"), "echo 'Don'\\''t break; rm -rf'");
    }
}
