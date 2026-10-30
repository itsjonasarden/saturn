//! Search sessions: every Claude Code and Codex session on this Mac, newest first, found by what you
//! asked in it, its title, its folder or its branch (`saturn_core::past_sessions`,
//! `saturn_core::codex_sessions`). Picking one resumes it in a new pane in its folder, with its own
//! agent, or goes to the pane it's already open in.

use std::sync::{LazyLock, Mutex};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, px,
};
use saturn_core::agents::Agent;
use saturn_core::layout::TabId;
use saturn_core::past_sessions::{Index, Session};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::{self, len};
use crate::workspace::Workspace;

/// The sessions read so far, kept for the next search so only changed transcripts are read again.
static INDEX: LazyLock<Mutex<Index>> = LazyLock::new(|| Mutex::new(Index::default()));
static CODEX_INDEX: LazyLock<Mutex<saturn_core::codex_sessions::Index>> =
    LazyLock::new(|| Mutex::new(saturn_core::codex_sessions::Index::default()));

/// The most rows shown; a search narrows them.
const SHOWN: usize = 80;

pub struct SessionsView {
    workspace: WeakEntity<Workspace>,
    query: Entity<InputState>,
    /// `None` while they're read.
    sessions: Option<Vec<Session>>,
    /// Sessions open in a pane, by their id.
    open: Vec<(String, TabId)>,
    /// Both agents are on this Mac, so each row says whose session it is.
    both: bool,
    _query_events: Subscription,
}

impl SessionsView {
    fn new(
        workspace: WeakEntity<Workspace>,
        open: Vec<(String, TabId)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A spike can type the search.
        let typed = std::env::var("SATURN_SESSIONS_QUERY").unwrap_or_default();
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search what you asked, titles, folders and branches")
                .default_value(typed)
        });
        query.update(cx, |query, cx| query.focus(window, cx));
        let query_events = cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        cx.spawn(async move |this, cx| {
            let sessions = cx
                .background_spawn(async move {
                    let home = saturn_core::prompts::claude_home();
                    let mut index = INDEX.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut sessions = home.map(|home| index.refresh(&home)).unwrap_or_default();
                    if let Some(home) = Agent::Codex.home().filter(|home| home.is_dir()) {
                        let mut codex = CODEX_INDEX.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        sessions.extend(codex.refresh(&home));
                        sessions.sort_by_key(|session| std::cmp::Reverse(session.modified));
                    }
                    sessions
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.sessions = Some(sessions);
                cx.notify();
            });
        })
        .detach();
        let both = crate::agent_choice::ready(cx).len() > 1;
        Self { workspace, query, sessions: None, open, both, _query_events: query_events }
    }

    fn row(&self, index: usize, session: &Session, query: &str) -> impl IntoElement + use<> {
        let pane = self.open.iter().find(|(id, _)| *id == session.id).map(|(_, tab)| *tab);
        let folder = session.cwd.clone().filter(|cwd| cwd.is_dir());
        let mut place: Vec<String> = Vec::new();
        // Whose, when this Mac runs both.
        if self.both {
            place.push(session.agent.name().to_owned());
        }
        if let Some(cwd) = &session.cwd {
            place.push(
                cwd.file_name().map_or_else(|| cwd.display().to_string(), |name| name.to_string_lossy().into_owned()),
            );
        }
        place.extend(session.branch.clone());
        place.extend(session.pull_request.map(|number| format!("#{number}")));
        let when = session.modified.map(when).unwrap_or_default();
        let (what, usable) = match (pane, &folder) {
            (Some(_), _) => ("Open in a pane", true),
            (None, Some(_)) => ("Resume", true),
            (None, None) => ("Its folder is gone", false),
        };
        let snippet = session.snippet(query).map(|prompt| prompt.lines().next().unwrap_or_default().to_owned());
        let row = div()
            .id(("session", index))
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(len(space::SPACE_2))
            .py(len(space::SPACE_1))
            .rounded(theme::corner(radius::RADIUS_SM))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_3))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(if usable { color::INK } else { color::INK_FAINT }))
                            .child(session.title.clone()),
                    )
                    .child(div().flex_none().text_size(px(12.)).text_color(theme::color(color::INK_FAINT)).child(when))
                    .child(
                        div()
                            .flex_none()
                            .w(px(110.))
                            .text_right()
                            .text_size(px(12.))
                            .text_color(theme::color(if pane.is_some() { color::BLUE } else { color::INK_MUTED }))
                            .child(what),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(len(space::SPACE_2))
                    .text_size(px(12.))
                    .child(div().flex_none().text_color(theme::color(color::INK_FAINT)).child(place.join(" · ")))
                    .children(snippet.map(|snippet| {
                        div().flex_1().min_w_0().truncate().text_color(theme::color(color::INK_MUTED)).child(snippet)
                    })),
            );
        if !usable {
            return row;
        }
        let workspace = self.workspace.clone();
        let (id, agent) = (session.id.clone(), session.agent);
        row.cursor_pointer().hover(|s| s.bg(theme::fill_hover())).on_click(move |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| match (pane, &folder) {
                (Some(tab), _) => workspace.go_to_tab(tab, window, cx),
                (None, Some(folder)) => workspace.resume_session(folder, agent, &id, window, cx),
                (None, None) => {}
            });
            window.close_dialog(cx);
        })
    }
}

impl Render for SessionsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query.read(cx).value().trim().to_owned();
        let mut list = div().id("sessions").flex().flex_col().gap(px(2.)).h(px(480.)).overflow_y_scroll();
        let quiet = |text: String| {
            div()
                .py(len(space::SPACE_3))
                .px(len(space::SPACE_2))
                .text_size(px(13.))
                .text_color(theme::color(color::INK_MUTED))
                .child(text)
        };
        match &self.sessions {
            None => list = list.child(quiet("Reading Claude's sessions…".to_owned())),
            Some(sessions) => {
                let found: Vec<&Session> = sessions.iter().filter(|session| session.matches(&query)).collect();
                if found.is_empty() {
                    let said =
                        if sessions.is_empty() { "No Claude sessions on this Mac yet." } else { "No session matches." };
                    list = list.child(quiet(said.to_owned()));
                }
                for (index, session) in found.iter().take(SHOWN).enumerate() {
                    list = list.child(self.row(index, session, &query));
                }
                if found.len() > SHOWN {
                    list = list.child(quiet(format!("{} more: search to narrow them down.", found.len() - SHOWN)));
                }
            }
        }
        div().flex().flex_col().gap(len(space::SPACE_3)).child(Input::new(&self.query)).child(list)
    }
}

/// How long ago something changed, in words.
fn when(modified: std::time::SystemTime) -> String {
    let seconds = modified.elapsed().map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX));
    match seconds {
        ..60 => "just now".to_owned(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{} h ago", seconds / 3_600),
        _ => crate::worktrees_view::ago(seconds),
    }
}

/// Opens Search sessions; `open` are the sessions panes run now, by id.
pub fn open(workspace: WeakEntity<Workspace>, open: Vec<(String, TabId)>, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|cx| SessionsView::new(workspace, open, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Search sessions")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(820.))
            .child(view.clone())
    });
}
