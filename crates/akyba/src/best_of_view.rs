//! Best of N (⌘⇧P): one task, given to two to four agents at once, each in a worktree of its own
//! (`akyba_core::best_of`). Compare attempts lays what each one changed side by side: its files,
//! lines and commits, and whether its agent is still at it. Keeping one archives the others, so
//! none is lost: Worktrees can restore any of them.

use std::path::{Path, PathBuf};

use akyba_core::agent_state::AgentState;
use akyba_core::agents::Agent;
use akyba_core::best_of::{self, Group, Outcome};
use akyba_core::layout::TabId;
use akyba_core::worktrees;
use akyba_design::{color, radius, space};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use akyba_design::text;

/// The task and how many attempts, before they start.
pub struct StartView {
    workspace: WeakEntity<Workspace>,
    repo_name: String,
    task: Entity<TextareaState>,
    /// Each attempt's agent; as many as attempts. All the current agent to begin with; with both
    /// agents ready, each can be switched, to see Claude Code and Codex take on the same task.
    agents: Vec<Agent>,
    said: Option<String>,
}

impl StartView {
    fn new(workspace: WeakEntity<Workspace>, repo: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A spike can fill the task in, and start it.
        let typed = std::env::var("AKYBA_BEST_OF").unwrap_or_default();
        let task = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("The task, as you'd give it to one agent").default_value(typed)
        });
        task.update(cx, |task, cx| task.focus(window, cx));
        if std::env::var_os("AKYBA_BEST_OF_START").is_some() {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                let _ = this.update_in(cx, Self::begin);
            })
            .detach();
        }
        let repo_name = repo.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let agent = crate::agent_choice::current(cx);
        // A spike sets each attempt's agent (`AKYBA_BEST_OF_AGENTS=claude,codex,claude`).
        let agents = std::env::var("AKYBA_BEST_OF_AGENTS")
            .ok()
            .map(|names| names.split(',').filter_map(|name| Agent::of_program(name.trim())).collect::<Vec<_>>())
            .filter(|agents| (2..=best_of::MOST).contains(&agents.len()))
            .unwrap_or_else(|| vec![agent; 3]);
        Self { workspace, repo_name, task, agents, said: None }
    }

    fn begin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.task.read(cx).value().trim().to_owned();
        if task.is_empty() {
            self.said = Some("Write the task first.".to_owned());
            cx.notify();
            return;
        }
        let agents = self.agents.clone();
        let _ = self.workspace.update(cx, |workspace, cx| workspace.start_attempts(task, agents, window, cx));
        window.close_dialog(cx);
    }
}

impl StartView {
    /// One switch per attempt, its agent raised: Claude Code against Codex on the same task.
    fn attempt_agents(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut row = div().flex().flex_wrap().items_center().gap(len(space::SPACE_2));
        for (index, chosen) in self.agents.iter().copied().enumerate() {
            let mut track = chrome::track();
            for agent in Agent::ALL {
                track = track.child(
                    chrome::track_item(("attempt-agent", index * 10 + agent as usize), chosen == agent)
                        .px(len(space::SPACE_2))
                        .text_scale(text::META)
                        .child(agent.name())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(slot) = this.agents.get_mut(index) {
                                *slot = agent;
                            }
                            cx.notify();
                        })),
                );
            }
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_1))
                    .child(
                        div()
                            .text_scale(text::META)
                            .text_color(theme::color(color::INK_FAINT))
                            .child(format!("{}", index + 1)),
                    )
                    .child(track),
            );
        }
        row
    }
}

impl Render for StartView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels: Vec<gpui_kit::SharedString> =
            (2..=best_of::MOST).map(|count| format!("{count} agents").into()).collect();
        let view = cx.weak_entity();
        let counts =
            chrome::selector("best-of-count", &labels, self.agents.len().saturating_sub(2), move |index, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    let last = this.agents.last().copied().unwrap_or(Agent::Claude);
                    this.agents.resize(index + 2, last);
                    cx.notify();
                });
            });
        let mixing = crate::agent_choice::ready(cx).len() > 1;
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_3))
            .child(div().text_scale(text::LABEL).text_color(theme::color(color::INK_MUTED)).child(format!(
                "Each agent gets a worktree of {} and the same task. Compare attempts shows what each changed, side by side.",
                self.repo_name
            )))
            .child(Textarea::new(&self.task).h(px(140.)))
            .when(mixing, |view| view.child(self.attempt_agents(cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(counts)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_scale(text::LABEL)
                            .text_color(theme::color(color::INK_MUTED))
                            .children(self.said.clone()),
                    )
                    .child(
                        chrome::button("best-of-start", format!("Start {} attempts", self.agents.len()), None, true)
                            .on_click(cx.listener(|this, _, window, cx| this.begin(window, cx))),
                    ),
            )
    }
}

/// Opens Best of N for the main checkout `repo`.
pub fn start(workspace: WeakEntity<Workspace>, repo: &Path, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|cx| StartView::new(workspace, repo, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        chrome::dialog(dialog, "Best of N", akyba_design::size::DIALOG_MEDIUM, view.clone())
    });
}

/// A group of attempts, as read.
struct Read {
    group: Group,
    outcomes: Vec<Outcome>,
}

pub struct CompareView {
    workspace: WeakEntity<Workspace>,
    /// `None` while they're read.
    groups: Option<Vec<Read>>,
    picked: usize,
    said: Option<String>,
}

impl CompareView {
    fn new(workspace: WeakEntity<Workspace>, repo: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A spike can keep an attempt by its number, once they're read.
        let keep: Option<usize> = std::env::var("AKYBA_KEEP_ATTEMPT").ok().and_then(|number| number.parse().ok());
        cx.spawn_in(window, async move |this, cx| {
            let groups = cx.background_spawn(async move { read_groups(&repo) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.groups = Some(groups);
                if let Some(number) = keep {
                    this.keep(number, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        Self { workspace, groups: None, picked: 0, said: None }
    }

    fn keep(&mut self, number: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(read) = self.groups.as_ref().and_then(|groups| groups.get(self.picked)) else { return };
        let others =
            read.group.attempts.iter().filter(|(other, _)| *other != number).map(|(_, worktree)| worktree.clone());
        let others: Vec<_> = others.collect();
        let _ = self.workspace.update(cx, |workspace, cx| workspace.keep_attempt(others, cx));
        window.close_dialog(cx);
    }

    /// The files an attempt changed, each opening in the editor.
    fn file_list(&self, index: usize, folder: &Path, outcome: &Outcome) -> gpui_kit::Stateful<gpui_kit::Div> {
        let mut list = div().id(("attempt-files", index)).flex().flex_col().flex_1().min_h_0().overflow_y_scroll();
        if outcome.files.is_empty() {
            list = list.child(
                div().text_scale(text::META).text_color(theme::color(color::INK_FAINT)).child("Nothing changed yet."),
            );
        }
        for (row, file) in outcome.files.iter().enumerate() {
            let path = folder.join(&file.path);
            let workspace = self.workspace.clone();
            list = list.child(
                div()
                    .id(("attempt-file", index * 10_000 + row))
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .px(len(space::SPACE_1))
                    .py(len(space::SPACE_0))
                    .rounded(theme::corner(radius::RADIUS_XS))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::fill_hover()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_scale(text::META)
                            .text_color(theme::color(color::INK))
                            .child(file.path.clone()),
                    )
                    .child(crate::changes_view::stats(file.added, file.removed))
                    .on_click(move |_, window, cx| {
                        let _ = workspace.update(cx, |workspace, cx| workspace.open_file(&path, window, cx));
                        window.close_dialog(cx);
                    }),
            );
        }
        list
    }

    fn column(
        &self,
        index: usize,
        (number, worktree): &(usize, worktrees::Worktree),
        outcome: &Outcome,
        pane: Option<(TabId, AgentState)>,
        agent: Option<Agent>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let number = *number;
        let state = match pane {
            Some((_, AgentState::Shell)) => "No agent".to_owned(),
            Some((_, state)) => state.label().to_owned(),
            None => "No pane".to_owned(),
        };
        let working = matches!(pane, Some((_, AgentState::Working)));
        let files = outcome.files.len();
        let commits = match outcome.commits {
            0 => "nothing committed".to_owned(),
            1 => "1 commit".to_owned(),
            count => format!("{count} commits"),
        };
        let summary = format!("{files} file{} · {commits}", if files == 1 { "" } else { "s" });
        let list = self.file_list(index, &worktree.path, outcome);
        let go = pane.map(|(tab, _)| {
            let workspace = self.workspace.clone();
            chrome::button(("attempt-go", index), "Go to it", None, false).on_click(move |_, window, cx| {
                let _ = workspace.update(cx, |workspace, cx| workspace.go_to_tab(tab, window, cx));
                window.close_dialog(cx);
            })
        });
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h(px(420.))
            .gap(len(space::SPACE_2))
            .p(len(space::SPACE_3))
            .rounded(theme::corner(radius::RADIUS_SM))
            .border_1()
            .border_color(theme::separator())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(
                        div()
                            .flex_1()
                            .text_scale(text::LABEL)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(color::INK))
                            .child(match agent {
                                Some(agent) => format!("Attempt {number} · {}", agent.name()),
                                None => format!("Attempt {number}"),
                            }),
                    )
                    .child(
                        div()
                            .text_scale(text::META)
                            .text_color(theme::color(if working { color::BLUE } else { color::INK_MUTED }))
                            .child(state),
                    ),
            )
            .child(
                div()
                    .truncate()
                    .text_scale(text::META)
                    .text_color(theme::color(color::INK_FAINT))
                    .child(worktree.branch.clone().unwrap_or_default()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(crate::changes_view::stats(outcome.added(), outcome.removed()))
                    .child(div().text_scale(text::META).text_color(theme::color(color::INK_MUTED)).child(summary)),
            )
            .child(list)
            .child(
                div().flex().gap(len(space::SPACE_2)).children(go).child(div().flex_1()).child(
                    chrome::button(("attempt-keep", index), "Keep this one", None, false)
                        .tooltip(chrome::tip("Closes the other attempts' panes and archives their worktrees"))
                        .on_click(cx.listener(move |this, _, window, cx| this.keep(number, window, cx))),
                ),
            )
    }
}

impl Render for CompareView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let quiet = |text: &str| {
            div()
                .py(len(space::SPACE_4))
                .text_scale(text::BODY)
                .text_color(theme::color(color::INK_MUTED))
                .child(text.to_owned())
        };
        let Some(groups) = &self.groups else { return quiet("Reading the attempts…").into_any_element() };
        let Some(read) = groups.get(self.picked) else {
            return quiet("No attempts in this repo. Best of N (⌘⇧P) starts some.").into_any_element();
        };
        let mut tasks = chrome::track();
        for (index, other) in groups.iter().enumerate() {
            tasks = tasks.child(
                chrome::track_item(("attempt-group", index), self.picked == index)
                    .px(len(space::SPACE_3))
                    .text_scale(text::META)
                    .child(other.group.stem.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.picked = index;
                        cx.notify();
                    })),
            );
        }
        let folders = best_of::folders(&read.group);
        let panes = self
            .workspace
            .upgrade()
            .map_or_else(|| vec![None; folders.len()], |workspace| workspace.read(cx).attempt_panes(&folders, cx));
        let attempts: Vec<_> = read.group.attempts.clone();
        let outcomes: Vec<_> = read.outcomes.clone();
        // Which agent made each, once there's more than one to tell apart.
        let agents: Vec<Option<Agent>> = match self.workspace.upgrade() {
            Some(workspace) if crate::agent_choice::ready(cx).len() > 1 => {
                panes.iter().map(|pane| pane.and_then(|(tab, _)| workspace.read(cx).agent_in(tab))).collect()
            }
            _ => vec![None; panes.len()],
        };
        let mut columns = div().flex().gap(len(space::SPACE_2));
        for (index, (attempt, outcome)) in attempts.iter().zip(&outcomes).enumerate() {
            let pane = panes.get(index).copied().flatten();
            let agent = agents.get(index).copied().flatten();
            columns = columns.child(self.column(index, attempt, outcome, pane, agent, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_3))
            .when(groups.len() > 1, |this| this.child(div().flex().child(tasks)))
            .child(columns)
            .children(
                self.said
                    .clone()
                    .map(|said| div().text_scale(text::LABEL).text_color(theme::color(color::INK_MUTED)).child(said)),
            )
            .into_any_element()
    }
}

/// The repo's attempts, each with what it changed. Slow: off the main thread.
fn read_groups(repo: &Path) -> Vec<Read> {
    best_of::groups(&worktrees::list(repo))
        .into_iter()
        .map(|group| {
            let start = best_of::start(repo, &group);
            let outcomes = group
                .attempts
                .iter()
                .map(|(_, worktree)| {
                    start.as_deref().map(|start| best_of::outcome(&worktree.path, start)).unwrap_or_default()
                })
                .collect();
            Read { group, outcomes }
        })
        .collect()
}

/// Opens Compare attempts for the main checkout `repo`.
pub fn compare(workspace: WeakEntity<Workspace>, repo: PathBuf, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|cx| CompareView::new(workspace, repo, window, cx));
    window.open_dialog(cx, move |dialog, _, _| chrome::dialog(dialog, "Compare attempts", 1080., view.clone()));
}
