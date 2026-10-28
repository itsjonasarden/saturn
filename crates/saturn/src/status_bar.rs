//! The status bar along the window's foot: on the left, the errors and warnings and the file's
//! language server; on the right, the cursor, the file's language, and the branch with how far it
//! is ahead of and behind its upstream. A slim card, like the panes, so it reads over any
//! desktop; the counts and the branch are buttons.

use std::path::{Path, PathBuf};

use gpui_kit::component::input::EditorState;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EntityId, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, px,
};
use saturn_core::git;
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::editor_view::{EditorView, ServerState};
use crate::problems_view;
use crate::theme::{self, len};
use crate::workspace::Workspace;

pub const HEIGHT: f32 = 28.;

/// The branch checked out, and commits ahead of and behind its upstream.
#[derive(Clone, PartialEq, Eq)]
struct Branch {
    name: String,
    ahead_behind: Option<(usize, usize)>,
}

pub struct StatusBar {
    editor: Entity<EditorView>,
    workspace: WeakEntity<Workspace>,
    root: PathBuf,
    branch: Option<Branch>,
    /// The file in front, watched so the cursor's place stays current.
    watched: Option<EntityId>,
    editor_changes: Subscription,
    file_watch: Option<Subscription>,
}

impl StatusBar {
    pub fn new(
        editor: Entity<EditorView>,
        workspace: WeakEntity<Workspace>,
        root: &Path,
        cx: &mut Context<Self>,
    ) -> Self {
        let observed = cx.observe(&editor, |this, _, cx| {
            this.watch_front(cx);
            cx.notify();
        });
        let mut bar = Self {
            editor,
            workspace,
            root: root.to_owned(),
            branch: None,
            watched: None,
            editor_changes: observed,
            file_watch: None,
        };
        bar.refresh(cx);
        bar
    }

    /// Follows another editor: the one of the repo now showing.
    pub fn set_editor(&mut self, editor: Entity<EditorView>, cx: &mut Context<Self>) {
        self.editor_changes = cx.observe(&editor, |this, _, cx| {
            this.watch_front(cx);
            cx.notify();
        });
        self.editor = editor;
        self.watch_front(cx);
        cx.notify();
    }

    /// Follows the file in front: a new one is watched, so moving its cursor redraws the bar.
    fn watch_front(&mut self, cx: &mut Context<Self>) {
        let front: Option<Entity<EditorState>> = self.editor.read(cx).front_state();
        let id = front.as_ref().map(Entity::entity_id);
        if id == self.watched {
            return;
        }
        self.watched = id;
        self.file_watch = front.map(|state| cx.observe(&state, |_, _, cx| cx.notify()));
    }

    /// Shows the branch of `root` (the folder the sidebar shows).
    pub fn set_root(&mut self, root: &Path, cx: &mut Context<Self>) {
        if self.root != root {
            root.clone_into(&mut self.root);
            self.refresh(cx);
        }
    }

    /// Reads the branch again, off the main thread: after a checkout, a commit, a pull.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let branch = cx
                .background_spawn(async move {
                    let name = git::branch(&root)?;
                    Some(Branch { name, ahead_behind: git::ahead_behind(&root) })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.branch != branch {
                    this.branch = branch;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn item(id: impl Into<ElementId>) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(len(space::SPACE_1))
            .h(px(20.))
            .px(len(space::SPACE_2))
            // 4px inside the bar's card (radius 10).
            .rounded(len(radius::RADIUS_SM))
    }

    /// The repos' Docker Compose services with ports: shared by every terminal, so they sit here
    /// rather than on a pane. Read-only: they aren't anything to click.
    fn render_services(&self, cx: &App) -> Vec<AnyElement> {
        let Some(workspace) = self.workspace.upgrade() else { return Vec::new() };
        workspace
            .read(cx)
            .services()
            .iter()
            .enumerate()
            .map(|(index, service)| {
                let ports = service.ports.iter().map(|port| format!(":{port}")).collect::<Vec<_>>().join(" ");
                let tip = format!("Docker Compose service, shared by every terminal ({})", service.folder.display());
                Self::item(SharedString::from(format!("service-{index}")))
                    .child(service.name.clone())
                    .child(div().text_color(theme::color(color::INK_FAINT)).child(ports))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .into_any_element()
            })
            .collect()
    }

    fn render_problems(errors: usize, warnings: usize, cx: &mut Context<Self>) -> AnyElement {
        let count = |severity, count: usize| {
            let (icon, colour) = problems_view::mark(severity);
            let tint = if count == 0 { color::INK_FAINT } else { colour };
            div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_1))
                .child(chrome::icon(icon, 12., theme::color(tint)))
                .child(count.to_string())
        };
        Self::item("status-problems")
            .cursor_pointer()
            .hover(|s| s.bg(theme::fill_hover()))
            .child(count(lsp_types::DiagnosticSeverity::ERROR, errors))
            .child(count(lsp_types::DiagnosticSeverity::WARNING, warnings))
            .tooltip(chrome::tip("Errors and warnings the language servers found"))
            .on_click(cx.listener(|this, _, _, cx| {
                let _ = this.workspace.update(cx, Workspace::show_problems);
            }))
            .into_any_element()
    }

    fn render_branch(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let branch = self.branch.as_ref()?;
        let sync = match branch.ahead_behind {
            Some((0, 0)) | None => None,
            Some((ahead, behind)) => Some(
                [(ahead > 0).then(|| format!("↑{ahead}")), (behind > 0).then(|| format!("↓{behind}"))]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        };
        Some(
            Self::item("status-branch")
                .cursor_pointer()
                .hover(|s| s.bg(theme::fill_hover()))
                .child(chrome::icon("git-branch", 12., theme::color(color::INK_FAINT)))
                .child(branch.name.clone())
                .children(sync.map(|sync| div().text_color(theme::color(color::INK_FAINT)).child(sync)))
                .tooltip(chrome::tip("The branch, and commits to push (↑) and pull (↓); click for History"))
                .on_click(cx.listener(|this, _, _, cx| {
                    let _ = this.workspace.update(cx, Workspace::show_history);
                }))
                .into_any_element(),
        )
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.editor.read(cx).status(cx);
        let server = status.server.clone().filter(|(name, _)| !name.is_empty()).map(|(name, state)| {
            let (dot, tip) = match state {
                ServerState::Ready => (color::TERM_GREEN, "The language server is ready"),
                ServerState::Starting => (color::BLUE, "The language server is starting"),
                ServerState::Absent => (color::INK_FAINT, "No language server: the editor's strip says why"),
            };
            Self::item("status-server")
                .child(div().size(px(6.)).rounded_full().bg(theme::color(dot)))
                .child(name)
                .tooltip(chrome::tip(tip))
        });
        chrome::card()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(HEIGHT))
            .mt(len(space::SPACE_2))
            .mb(len(space::SPACE_2))
            .px(len(space::SPACE_1))
            .text_size(px(12.))
            .text_color(theme::color(color::INK_MUTED))
            .child(Self::render_problems(status.errors, status.warnings, cx))
            .children(server)
            .children(self.render_services(cx))
            .child(div().flex_1())
            .children(
                status
                    .cursor
                    .map(|(line, column)| Self::item("status-cursor").child(format!("Ln {line}, Col {column}"))),
            )
            .children(status.language.map(|language| Self::item("status-language").child(language)))
            .children(self.render_branch(cx))
    }
}
