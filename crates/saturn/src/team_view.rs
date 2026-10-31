//! A pane's agent team (its Team chip): who's in it, the shared task list with who has each task,
//! and the messages they've sent each other, oldest first. Read again every few seconds while open,
//! from what Claude Code writes (`saturn_core::teams`).

use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    App, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use saturn_core::layout::TabId;
use saturn_core::teams::{Message, Task};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use saturn_design::text;

pub struct TeamView {
    workspace: WeakEntity<Workspace>,
    tab: TabId,
}

impl TeamView {
    fn new(workspace: WeakEntity<Workspace>, tab: TabId, cx: &mut Context<Self>) -> Self {
        // The workspace reads the team every few seconds; show each new reading.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self { workspace, tab }
    }
}

impl Render for TeamView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let team = self.workspace.upgrade().and_then(|workspace| workspace.read(cx).team(self.tab).cloned());
        let Some(team) = team else {
            return div()
                .text_scale(text::LABEL)
                .text_color(theme::color(color::INK_MUTED))
                .child("The team has finished: Claude Code removes it when its session ends.")
                .into_any_element();
        };
        let mut members = div().flex().flex_wrap().gap(len(space::SPACE_1));
        for member in &team.members {
            let lead = member.kind.as_deref() == Some("team-lead");
            members = members.child(
                div()
                    .px(len(space::SPACE_2))
                    .py(len(space::SPACE_0))
                    .rounded(theme::corner(radius::RADIUS_XS))
                    .bg(theme::fill_control())
                    .text_scale(text::META)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(color::INK))
                    .child(if lead { format!("{} (lead)", member.name) } else { member.name.clone() }),
            );
        }
        let mut list = div()
            .id("team")
            .flex()
            .flex_col()
            .gap(len(space::SPACE_0))
            .max_h(px(560.))
            .overflow_y_scroll()
            .child(members);
        list = list.child(heading("Tasks"));
        if team.tasks.is_empty() {
            list = list.child(quiet("No tasks yet."));
        }
        for task in &team.tasks {
            list = list.child(task_row(task));
        }
        list = list.child(heading("Messages"));
        if team.messages.is_empty() {
            list = list.child(quiet("No messages yet."));
        }
        for message in &team.messages {
            list = list.child(message_row(message));
        }
        list.into_any_element()
    }
}

fn task_row(task: &Task) -> impl IntoElement {
    let (status, tone) = match task.status.as_str() {
        "completed" => ("Done", color::INK_FAINT),
        "in_progress" => ("Working", color::BLUE),
        _ => ("To do", color::INK_MUTED),
    };
    div()
        .flex()
        .items_center()
        .gap(len(space::SPACE_3))
        .px(len(space::SPACE_2))
        .py(px(3.)) // off-scale: a status row, 3px a side for 22px in all
        .child(div().flex_none().w(px(60.)).text_scale(text::META).text_color(theme::color(tone)).child(status))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_scale(text::LABEL)
                .text_color(theme::color(if task.status == "completed" { color::INK_MUTED } else { color::INK }))
                .child(task.subject.clone()),
        )
        .children(task.owner.clone().map(|owner| {
            div().flex_none().text_scale(text::META).text_color(theme::color(color::INK_FAINT)).child(owner)
        }))
}

fn message_row(message: &Message) -> impl IntoElement {
    // `2026-10-20T10:02:00Z` is shown as `10:02`.
    let time = message.at.get(11..16).unwrap_or_default().to_owned();
    div()
        .flex()
        .flex_col()
        .gap(len(space::SPACE_0))
        .px(len(space::SPACE_2))
        .py(len(space::SPACE_1))
        .child(
            div()
                .flex()
                .gap(len(space::SPACE_2))
                .text_scale(text::META)
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::color(color::INK))
                        .child(format!("{} → {}", message.from, message.to)),
                )
                .child(div().text_color(theme::color(color::INK_FAINT)).child(time))
                .children((!message.read).then(|| div().text_color(theme::color(color::BLUE)).child("Unread"))),
        )
        .child(div().text_scale(text::LABEL).text_color(theme::color(color::INK_MUTED)).child(message.text.clone()))
}

fn heading(text: &str) -> impl IntoElement {
    div()
        .pt(len(space::SPACE_3))
        .pb(len(space::SPACE_1))
        .px(len(space::SPACE_2))
        .text_scale(text::MICRO)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_FAINT))
        .child(text.to_uppercase())
}

fn quiet(text: &str) -> impl IntoElement {
    div()
        .px(len(space::SPACE_2))
        .text_scale(text::LABEL)
        .text_color(theme::color(color::INK_MUTED))
        .child(text.to_owned())
}

/// Opens the team the session in `tab` leads, named `title`.
pub fn open(workspace: WeakEntity<Workspace>, tab: TabId, title: String, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| TeamView::new(workspace, tab, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(format!("Agent team {title}"))
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(720.))
            .child(view.clone())
    });
}
