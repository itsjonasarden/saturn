//! Send to several agents (⌘⇧P): one message, typed into every pane you pick. A pane running an
//! agent gets it as if you'd typed it there; a pane that's only a shell starts the agent with it; a
//! pane busy with something else (a server, a build) can't be picked. Panes with an agent are
//! picked to begin with.

use std::collections::HashSet;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use saturn_core::git::Staged;
use saturn_core::layout::TabId;
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use saturn_design::text;

/// One pane a message could go to.
pub struct Pane {
    pub tab: TabId,
    pub name: String,
    /// Where it works: the folder, and the branch.
    pub place: String,
    pub reach: Reach,
}

/// What a message does in a pane.
#[derive(Clone, PartialEq, Eq)]
pub enum Reach {
    /// An agent runs there and reads it as typed.
    Agent,
    /// Only a shell: the chosen agent starts with the message.
    Shell,
    /// Another program has the pane, named here.
    Busy(String),
}

pub struct BroadcastView {
    workspace: WeakEntity<Workspace>,
    panes: Vec<Pane>,
    picked: HashSet<TabId>,
    message: Entity<TextareaState>,
    said: Option<String>,
}

impl BroadcastView {
    fn new(workspace: WeakEntity<Workspace>, panes: Vec<Pane>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let picked = panes.iter().filter(|pane| pane.reach == Reach::Agent).map(|pane| pane.tab).collect();
        // A spike can fill the message in.
        let typed = std::env::var("SATURN_BROADCAST").unwrap_or_default();
        let message = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("What every picked agent should do").default_value(typed)
        });
        message.update(cx, |message, cx| message.focus(window, cx));
        if std::env::var_os("SATURN_BROADCAST_SEND").is_some() {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                let _ = this.update_in(cx, Self::send);
            })
            .detach();
        }
        Self { workspace, panes, picked, message, said: None }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().trim().to_owned();
        if message.is_empty() {
            self.said = Some("Write the message first.".to_owned());
            cx.notify();
            return;
        }
        let tabs: Vec<TabId> = self.panes.iter().map(|pane| pane.tab).filter(|tab| self.picked.contains(tab)).collect();
        if tabs.is_empty() {
            self.said = Some("Pick a pane to send it to.".to_owned());
            cx.notify();
            return;
        }
        let _ = self.workspace.update(cx, |workspace, cx| workspace.send_to_panes(&tabs, &message, cx));
        window.close_dialog(cx);
    }
}

impl Render for BroadcastView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list =
            div().id("broadcast-panes").flex().flex_col().gap(len(space::SPACE_0)).max_h(px(320.)).overflow_y_scroll();
        if self.panes.is_empty() {
            list = list.child(
                div()
                    .py(len(space::SPACE_2))
                    .text_scale(text::LABEL)
                    .text_color(theme::color(color::INK_MUTED))
                    .child("No panes are open."),
            );
        }
        for (index, pane) in self.panes.iter().enumerate() {
            list = list.child(self.row(index, pane, cx));
        }
        let count = self.picked.len();
        let label = match count {
            1 => "Send to 1 pane".to_owned(),
            count => format!("Send to {count} panes"),
        };
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_3))
            .child(div().text_scale(text::LABEL).text_color(theme::color(color::INK_MUTED)).child(
                "Pick the panes, then write the message once. A pane with only a shell starts your agent with it.",
            ))
            .child(list)
            .child(Textarea::new(&self.message).h(px(140.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_scale(text::LABEL)
                            .text_color(theme::color(color::INK_MUTED))
                            .children(self.said.clone()),
                    )
                    .child(
                        chrome::button("broadcast-send", label, None, true)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
    }
}

impl BroadcastView {
    fn row(&self, index: usize, pane: &Pane, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tab = pane.tab;
        let picked = self.picked.contains(&tab);
        let busy = matches!(pane.reach, Reach::Busy(_));
        let (what, tone) = match &pane.reach {
            Reach::Agent => ("Agent".to_owned(), color::INK_MUTED),
            Reach::Shell => (format!("Starts {}", crate::agent_choice::current(cx).name()), color::INK_FAINT),
            Reach::Busy(program) => (format!("Busy: {program}"), color::INK_FAINT),
        };
        let row = div()
            .id(("broadcast-pane", index))
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .px(len(space::SPACE_2))
            .py(len(space::SPACE_1))
            .rounded(theme::corner(radius::RADIUS_SM))
            .child(crate::changes_view::checkbox(
                ("broadcast-check", index),
                if picked { Staged::Yes } else { Staged::No },
            ))
            .child(
                div()
                    .flex_none()
                    .text_scale(text::LABEL)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(if busy { color::INK_FAINT } else { color::INK }))
                    .child(pane.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_scale(text::META)
                    .text_color(theme::color(color::INK_FAINT))
                    .child(pane.place.clone()),
            )
            .child(div().flex_none().text_scale(text::META).text_color(theme::color(tone)).child(what));
        if busy {
            return row;
        }
        row.cursor_pointer().hover(|s| s.bg(theme::fill_hover())).on_click(cx.listener(move |this, _, _, cx| {
            if !this.picked.remove(&tab) {
                this.picked.insert(tab);
            }
            this.said = None;
            cx.notify();
        }))
    }
}

/// Opens Send to several agents over the workspace's `panes`.
pub fn open(workspace: WeakEntity<Workspace>, panes: Vec<Pane>, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|cx| BroadcastView::new(workspace, panes, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Send to several agents")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(saturn_design::size::DIALOG_MEDIUM))
            .child(view.clone())
    });
}
