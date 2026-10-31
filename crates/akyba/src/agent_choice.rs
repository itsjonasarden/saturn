//! Which agents this Mac has (Claude Code, Codex: installed and logged in), found once as Akyba
//! starts, and which one a new pane starts: the one picked last, remembered in the settings. With
//! only one ready, there's nothing to pick.

use std::time::Duration;

use akyba_core::agents::{self, Agent, Found};
use akyba_design::{color, space};
use gpui_kit::{
    App, AppContext as _, Global, IntoElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    div,
};

use crate::settings;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use akyba_design::text;

/// What was found, once it has been.
#[derive(Default)]
pub struct Agents {
    pub found: Option<Vec<Found>>,
    /// Akyba's Codex profile is in place, so Codex sessions Akyba starts use it.
    pub codex_profile: bool,
}

impl Global for Agents {}

/// Finds the agents, off the main thread, once the login shell's `PATH` is known. A spike names
/// them instead (`AKYBA_AGENTS=claude,codex`, or empty for none), without running anything.
pub fn start(cx: &mut App) {
    cx.set_global(Agents::default());
    if let Ok(names) = std::env::var("AKYBA_AGENTS") {
        let named: Vec<Agent> = names.split(',').filter_map(|name| Agent::of_program(name.trim())).collect();
        let found = Agent::ALL
            .into_iter()
            .map(|agent| Found { agent, installed: named.contains(&agent), logged_in: named.contains(&agent) })
            .collect();
        cx.global_mut::<Agents>().found = Some(found);
        // A spike's Codex home is its own (`CODEX_HOME`); the person's is never written by one.
        if std::env::var_os("CODEX_HOME").is_some() && named.contains(&Agent::Codex) {
            cx.global_mut::<Agents>().codex_profile = install_codex_profile();
        }
        return;
    }
    cx.spawn(async move |cx| {
        // The login shell's PATH is read as Akyba starts; it can take a moment.
        for _ in 0..50 {
            if crate::LOGIN_PATH.get().is_some() {
                break;
            }
            cx.background_executor().timer(Duration::from_millis(100)).await;
        }
        let path = crate::LOGIN_PATH
            .get()
            .cloned()
            .unwrap_or_else(|| "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".to_owned());
        let (found, codex_profile) = cx
            .background_spawn(async move {
                let found = agents::detect(&path);
                let codex = found.iter().any(|found| found.agent == Agent::Codex && found.installed);
                (found, codex && install_codex_profile())
            })
            .await;
        cx.update(|cx| {
            let agents = cx.global_mut::<Agents>();
            agents.found = Some(found);
            agents.codex_profile = codex_profile;
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Writes Akyba's Codex profile (`$CODEX_HOME/akyba.config.toml`) when it isn't as it should be:
/// Codex's notifications to the terminal, and Akyba's `codex-notify` as each turn ends, unless the
/// person runs a `notify` of their own, which stays theirs. Says whether the profile is there.
pub fn install_codex_profile() -> bool {
    let Some(home) = Agent::Codex.home().filter(|home| home.is_dir()) else { return false };
    let own_notify = std::fs::read_to_string(home.join("config.toml"))
        .is_ok_and(|config| config.lines().any(|line| line.trim_start().starts_with("notify")));
    let script = crate::workspace::plugin_dir().map(|dir| dir.join("bin/codex-notify"));
    let notify = script.filter(|_| !own_notify).map(|script| script.to_string_lossy().into_owned());
    let file = home.join(format!("{}.config.toml", agents::CODEX_PROFILE));
    let now = std::fs::read_to_string(&file).unwrap_or_default();
    let text = agents::codex_profile(&now, notify.as_deref(), settings::load().codex_status_line);
    now == text || std::fs::write(&file, text).is_ok()
}

/// A command as Akyba types it: Codex with Akyba's profile, once it's in place.
pub fn with_profile(command: &str, cx: &App) -> String {
    let ready = cx.try_global::<Agents>().is_some_and(|agents| agents.codex_profile);
    if ready { agents::with_codex_profile(command) } else { command.to_owned() }
}

/// What's typed to run `command`, written as Akyba writes agents' commands (`claude …`): the
/// agent it starts, after any setup that runs first (`setup && codex '…'`), with Akyba's Codex
/// profile and started with the person's own command for it (Settings → Agents).
pub fn as_typed(command: &str, cx: &App) -> String {
    let own = settings::load().agent_commands;
    let typed = |command: &str| {
        let command = with_profile(command, cx);
        match Agent::of_command(&command) {
            Some(agent) => agents::with_own_command(&command, own.get(agent)),
            None => command,
        }
    };
    match command.rsplit_once("&& ") {
        Some((setup, last)) => format!("{setup}&& {}", typed(last)),
        None => typed(command),
    }
}

/// The agents that can start: installed and logged in. Before they're found, Claude Code, as
/// Akyba has always started.
pub fn ready(cx: &App) -> Vec<Agent> {
    match cx.try_global::<Agents>().and_then(|agents| agents.found.as_ref()) {
        Some(found) => found.iter().filter(|found| found.ready()).map(|found| found.agent).collect(),
        None => vec![Agent::Claude],
    }
}

/// The agent a new pane starts: the one picked last when it's ready, else the first ready one.
pub fn current(cx: &App) -> Agent {
    let last = settings::load().agent;
    match cx.try_global::<Agents>().and_then(|agents| agents.found.as_ref()) {
        Some(found) => agents::pick(found, last),
        None => last.unwrap_or(Agent::Claude),
    }
}

/// Remembers `agent` as the one to start next time.
pub fn remember(agent: Agent) {
    let mut settings = settings::load();
    if settings.agent != Some(agent) {
        settings.agent = Some(agent);
        settings::save(&settings);
    }
}

/// The Claude Code | Codex switch, with what it's for (`label`): picking one remembers it for every
/// later start. `None` with fewer than two agents ready, when there's nothing to pick.
pub fn row(id: &'static str, label: &'static str, cx: &App) -> Option<gpui_kit::AnyElement> {
    let track = switch(id, cx)?;
    Some(
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .pt(len(space::SPACE_2))
            .child(div().text_scale(text::LABEL).text_color(theme::color(color::INK_MUTED)).child(label))
            .child(track)
            .into_any_element(),
    )
}

/// The switch alone: a track with one item per ready agent, the current one raised.
pub fn switch(id: &'static str, cx: &App) -> Option<gpui_kit::AnyElement> {
    let ready = ready(cx);
    if ready.len() < 2 {
        return None;
    }
    let current = current(cx);
    let mut track = crate::chrome::track();
    for agent in ready {
        track = track.child(
            crate::chrome::track_item((id, agent as usize), agent == current)
                .px(len(space::SPACE_3))
                .text_scale(text::META)
                .child(agent.name())
                .on_click(move |_, window, _| {
                    remember(agent);
                    window.refresh();
                }),
        );
    }
    Some(track.into_any_element())
}
