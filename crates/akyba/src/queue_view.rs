//! Waiting for you (the title bar's counts, or ⌘⇧P): every agent that waits, and for what, in one
//! list: a permission with the command or file it's for, a question, a finished turn with the start
//! of its last words, a failure. A permission can be answered from here; Akyba presses the key in
//! the pane (Return for Claude's highlighted "Yes", Escape for "No"), so the pane stays the truth.

use akyba_core::agent_state::AgentState;
use akyba_core::layout::TabId;
use akyba_core::waiting::Waiting;
use akyba_design::{color, radius, space};
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, px,
};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use akyba_design::text;

/// One agent that waits.
pub struct Row {
    pub tab: TabId,
    pub name: String,
    /// Where it works: the folder, and the branch.
    pub place: String,
    pub state: AgentState,
    pub waiting: Option<Waiting>,
}

pub fn open(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let rows = workspace.upgrade().map(|workspace| workspace.read(cx).waiting_rows(cx)).unwrap_or_default();
        let list = if rows.is_empty() {
            div()
                .py(len(space::SPACE_4))
                .text_scale(text::BODY)
                .text_color(theme::color(color::INK_MUTED))
                .child("No agent is waiting for you.")
                .into_any_element()
        } else {
            let mut list =
                div().id("waiting").flex().flex_col().gap(len(space::SPACE_1)).max_h(px(520.)).overflow_y_scroll();
            for row in rows {
                list = list.child(render_row(&workspace, row));
            }
            list.into_any_element()
        };
        chrome::dialog(dialog, "Waiting for you", akyba_design::size::DIALOG_MEDIUM, list)
    });
}

fn render_row(workspace: &WeakEntity<Workspace>, row: Row) -> impl IntoElement {
    let tab = row.tab;
    let (what, detail) = match &row.waiting {
        Some(Waiting::Permission { tool, detail }) => (format!("Asks to use {tool}"), Some(detail.clone())),
        Some(Waiting::Question(question)) => ("Asks you".to_owned(), Some(question.clone())),
        Some(Waiting::Done(said)) => ("Done".to_owned(), said.clone()),
        Some(Waiting::Failed(why)) => ("Failed".to_owned(), Some(why.clone())),
        None => match row.state {
            AgentState::NeedsYou => ("Needs you".to_owned(), None),
            AgentState::Error => ("Failed".to_owned(), None),
            _ => ("Your turn".to_owned(), None),
        },
    };
    let asks_permission = matches!(row.waiting, Some(Waiting::Permission { .. })) && row.state == AgentState::NeedsYou;
    let button = |id: &'static str, label: &'static str, primary: bool, act: Act| {
        let workspace = workspace.clone();
        chrome::button((id, tab.0), label, None, primary).on_click(move |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| match act {
                Act::Allow => {
                    workspace.answer_permission(tab, true, cx);
                }
                Act::Deny => {
                    workspace.answer_permission(tab, false, cx);
                }
                Act::Go => {
                    workspace.go_to_tab(tab, window, cx);
                    window.close_dialog(cx);
                }
            });
        })
    };
    div()
        .flex()
        .items_center()
        .gap(len(space::SPACE_3))
        .px(len(space::SPACE_3))
        .py(len(space::SPACE_2))
        .rounded(theme::corner(radius::RADIUS_SM))
        .bg(theme::fill_control())
        .child(
            div()
                .flex_none()
                .size(px(akyba_design::size::DOT_LARGE))
                .map(crate::chrome::pill)
                .bg(crate::workspace::state_color(row.state)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(len(space::SPACE_0))
                .child(
                    div()
                        .flex()
                        .gap(len(space::SPACE_2))
                        .text_scale(text::BODY)
                        .child(
                            div().font_weight(FontWeight::MEDIUM).text_color(theme::color(color::INK)).child(row.name),
                        )
                        .child(div().text_color(theme::color(color::INK_MUTED)).child(what)),
                )
                .children(detail.map(|detail| {
                    div()
                        .font_family(if asks_permission { akyba_design::font::MONO } else { akyba_design::look().sans })
                        .text_scale(text::META)
                        .text_color(theme::color(color::INK))
                        .truncate()
                        .child(detail)
                }))
                .child(
                    div().text_scale(text::META).text_color(theme::color(color::INK_FAINT)).truncate().child(row.place),
                ),
        )
        .children(asks_permission.then(|| button("waiting-deny", "Deny", false, Act::Deny)))
        .children(asks_permission.then(|| button("waiting-allow", "Allow", true, Act::Allow)))
        .child(button("waiting-go", "Go to pane", false, Act::Go))
}

#[derive(Clone, Copy)]
enum Act {
    Allow,
    Deny,
    Go,
}
