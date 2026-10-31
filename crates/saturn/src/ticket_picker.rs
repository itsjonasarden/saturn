//! New agent (⌘⇧N): name the work and Saturn starts an agent on it in a worktree of its own; with
//! ClickUp connected, the same field searches your open tickets, grouped by status, and picking one
//! starts an agent on it the way the project's ticket settings say. Return with nothing typed
//! starts the agent here, with no task.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    App, AppContext as _, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px, rgb,
};
use saturn_core::clickup::{self, Ticket};
use saturn_core::git;
use saturn_design::{color, radius, space};

use crate::theme::{self, len};
use crate::workspace::Workspace;

/// Opens the dialog. `connected` says whether ClickUp is, given by the caller: the workspace opens
/// this from inside its own update, where reading it would abort.
pub fn open(workspace: WeakEntity<Workspace>, connected: bool, window: &mut Window, cx: &mut App) {
    let placeholder = if connected { "Name the work, or search your tickets" } else { "Name the work, like fix-login" };
    // Screenshots type their name up front.
    let typed = std::env::var("SATURN_NEW_AGENT_QUERY").unwrap_or_default();
    let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(typed));
    search.update(cx, |search, cx| search.focus(window, cx));
    // Later: the button and the shortcut that open this run inside the workspace's own update, and
    // updating it again from there aborts.
    let refresh = workspace.clone();
    cx.defer(move |cx| {
        let _ = refresh.update(cx, Workspace::refresh_tickets);
    });
    window.open_dialog(cx, move |dialog, _, cx| {
        let Some(workspace) = workspace.upgrade() else { return dialog };
        let query = search.read(cx).value().to_string();
        let rows = picks(&workspace, &query, cx);
        let state = workspace.read(cx).clickup();
        let note = if let Some(problem) = state.problem.clone().filter(|_| state.connected()) {
            Some(problem)
        } else if state.loading && rows.is_empty() {
            Some("Reading your tickets…".to_owned())
        } else if rows.is_empty() && state.connected() {
            Some("No open tickets assigned to you. Type a name to start without one.".to_owned())
        } else {
            None
        };
        // Return is the dialog's OK: it starts the top row, so typing a name and pressing it is all
        // it takes; with nothing typed, the agent starts on its own.
        let confirm = (workspace.downgrade(), search.clone());
        dialog
            .on_ok(move |_, window, cx| {
                let (workspace, search) = &confirm;
                let Some(workspace) = workspace.upgrade() else { return true };
                let query = search.read(cx).value().to_string();
                if query.trim().is_empty() {
                    start(&workspace, None, window, cx);
                    return false;
                }
                let Some(pick) = picks(&workspace, &query, cx).into_iter().next() else { return false };
                start(&workspace, Some(&pick), window, cx);
                false
            })
            .title("New agent")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(crate::chrome::dialog_surface())
            .w(px(600.))
            // Room around the field inside the dialog, which clips at its edge, so the focus ring is
            // whole.
            .child(div().px(px(3.)).child(crate::chrome::input(&search)))
            .children(kind_row(cx))
            .children(also_in_row(&workspace, cx))
            .children(note.map(|note| {
                div().py(len(space::SPACE_4)).text_size(px(14.)).text_color(theme::color(color::INK_MUTED)).child(note)
            }))
            .child(list(&workspace, &rows, cx))
            .child(footer(&query, state.connected(), cx))
    });
}

/// The window's other repos, to start the agent in them too: the work gets a worktree in each on
/// the same branch, and the agent reaches them all. Only with more than one repo open.
fn also_in_row(workspace: &Entity<Workspace>, cx: &App) -> Option<impl IntoElement + use<>> {
    let others = workspace.read(cx).other_repos();
    if others.is_empty() {
        return None;
    }
    let picked = workspace.read(cx).also_in().to_vec();
    let mut track = crate::chrome::track();
    for (index, repo) in others.into_iter().enumerate() {
        let name = repo.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let weak = workspace.downgrade();
        track = track.child(
            crate::chrome::track_item(("also-in", index), picked.contains(&repo))
                .px(len(space::SPACE_3))
                .text_size(px(12.))
                .child(name)
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |workspace, cx| workspace.toggle_also_in(&repo, cx));
                }),
        );
    }
    Some(
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .pt(len(space::SPACE_2))
            .child(div().text_size(px(13.)).text_color(theme::color(color::INK_MUTED)).child("Also in"))
            .child(track)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::color(color::INK_FAINT))
                    .child("a worktree in each, on the same branch"),
            ),
    )
}

/// A row of the dialog: a ticket, or work named by what was typed.
#[derive(Clone)]
enum Pick {
    Ticket(Box<Ticket>),
    Name(String),
}

/// The rows for `query`: the tickets it matches, and the typed name as work of its own, first when
/// no ticket matches so that Return starts it.
fn picks(workspace: &Entity<Workspace>, query: &str, cx: &App) -> Vec<Pick> {
    let tickets = workspace.read(cx).clickup().tickets.iter().filter(|ticket| ticket.matches(query)).cloned();
    let mut picks: Vec<Pick> = tickets.map(|ticket| Pick::Ticket(Box::new(ticket))).collect();
    let name = query.trim();
    if !git::branch_for(name).is_empty() {
        picks.push(Pick::Name(name.to_owned()));
    }
    // With no ticket matching, the name is the only row and comes first; Return takes it.
    picks
}

/// Starts `pick` with the agent picked; with no pick, the agent on its own, here.
fn start(workspace: &Entity<Workspace>, pick: Option<&Pick>, window: &mut Window, cx: &mut App) {
    let agent = Some(crate::agent_choice::current(cx));
    workspace.update(cx, |workspace, cx| match pick {
        Some(Pick::Ticket(ticket)) => workspace.start_ticket(ticket, agent, window, cx),
        Some(Pick::Name(name)) => workspace.start_named(name, agent, window, cx),
        None => workspace.start_here(agent, window, cx),
    });
    window.close_dialog(cx);
}

/// The agent the pane runs (Claude Code | Codex), remembered for every later start; nothing with
/// only one on the Mac.
fn kind_row(cx: &App) -> Option<impl IntoElement + use<>> {
    let track = crate::agent_choice::switch("new-agent-agent", cx)?;
    Some(
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .pt(len(space::SPACE_3))
            .child(div().text_size(px(13.)).text_color(theme::color(color::INK_MUTED)).child("Agent"))
            .child(track),
    )
}

/// One quiet line at the foot: what Return does now, and, without ClickUp, where tickets come from.
fn footer(query: &str, connected: bool, cx: &App) -> impl IntoElement {
    let agent = crate::agent_choice::current(cx).name();
    let does = if query.trim().is_empty() {
        format!("↩ starts {agent} here, with no task")
    } else {
        "↩ starts the first row".to_owned()
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(len(space::SPACE_4))
        .pt(len(space::SPACE_3))
        .text_size(px(12.))
        .text_color(theme::color(color::INK_FAINT))
        .child(div().min_w_0().truncate().child(does))
        .when(!connected, |footer| footer.child(div().flex_none().child("Tickets: connect ClickUp in Settings")))
}

fn list(workspace: &Entity<Workspace>, picks: &[Pick], cx: &App) -> impl IntoElement {
    // The rows' text lines up with the labels above; their hover reaches out past it.
    let mut rows = div()
        .id("tickets")
        .flex()
        .flex_col()
        .max_h(px(440.))
        .overflow_y_scroll()
        .mt(len(space::SPACE_2))
        .mx(len(-space::SPACE_3));
    let tickets: Vec<Ticket> = picks
        .iter()
        .filter_map(|pick| if let Pick::Ticket(ticket) = pick { Some((**ticket).clone()) } else { None })
        .collect();
    let named = picks.iter().find_map(|pick| if let Pick::Name(name) = pick { Some(name.clone()) } else { None });
    let named_first = matches!(picks.first(), Some(Pick::Name(_)));
    if named_first && let Some(name) = &named {
        rows = rows.child(name_row(workspace, name, cx));
    }
    for (status, group) in clickup::by_status(&tickets) {
        rows = rows.child(
            div()
                .pt(len(space::SPACE_3))
                .pb(len(space::SPACE_1))
                .px(len(space::SPACE_3))
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::color(color::INK_FAINT))
                .child(status.to_uppercase()),
        );
        for ticket in group {
            rows = rows.child(row(workspace, ticket));
        }
    }
    if !named_first && let Some(name) = &named {
        rows = rows.child(div().h(len(space::SPACE_2))).child(name_row(workspace, name, cx));
    }
    rows
}

/// Work named by hand: where its worktree will be, so the name's effect is plain before Return.
fn name_row(workspace: &Entity<Workspace>, name: &str, cx: &App) -> impl IntoElement {
    let repo = workspace
        .read(cx)
        .project_path()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = format!("{repo}-{}", git::branch_for(name));
    let handle = workspace.downgrade();
    let pick = Pick::Name(name.to_owned());
    div()
        .id("named-work")
        .flex()
        .items_center()
        .gap(len(space::SPACE_3))
        .px(len(space::SPACE_3))
        .py(len(space::SPACE_2))
        .rounded(theme::corner(radius::RADIUS_SM))
        .cursor_pointer()
        .hover(|s| s.bg(theme::fill_hover()))
        .child(crate::chrome::icon("git-branch", 14., theme::color(color::INK_MUTED)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(14.))
                .text_color(theme::color(color::INK))
                .truncate()
                .child(format!("Start “{name}”")),
        )
        .child(
            div()
                .flex_none()
                .font_family(saturn_design::font::MONO)
                .text_size(px(12.))
                .text_color(theme::color(color::INK_FAINT))
                .child(format!("new worktree {folder}")),
        )
        .on_click(move |_, window, cx| {
            if let Some(workspace) = handle.upgrade() {
                start(&workspace, Some(&pick), window, cx);
            }
        })
}

fn row(workspace: &Entity<Workspace>, ticket: &Ticket) -> impl IntoElement {
    let workspace = workspace.downgrade();
    let picked = ticket.clone();
    div()
        .id(gpui_kit::SharedString::from(format!("ticket-{}", ticket.id)))
        .flex()
        .items_center()
        .gap(len(space::SPACE_3))
        .px(len(space::SPACE_3))
        .py(len(space::SPACE_2))
        .rounded(theme::corner(radius::RADIUS_SM))
        .cursor_pointer()
        .hover(|s| s.bg(theme::fill_hover()))
        .child(status_dot(ticket))
        .child(
            div()
                .flex_none()
                .w(px(84.))
                .font_family(saturn_design::font::MONO)
                .text_size(px(12.))
                .text_color(theme::color(color::INK_MUTED))
                .truncate()
                .child(ticket.key().to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(14.))
                .text_color(theme::color(color::INK))
                .truncate()
                .child(ticket.title.clone()),
        )
        .children(
            ticket.list.clone().map(|list| {
                div().flex_none().text_size(px(12.)).text_color(theme::color(color::INK_FAINT)).child(list)
            }),
        )
        .on_click(move |_, window, cx| {
            if let Some(workspace) = workspace.upgrade() {
                start(&workspace, Some(&Pick::Ticket(Box::new(picked.clone()))), window, cx);
            }
        })
}

/// A dot in the status's own ClickUp colour, so statuses read as they do in ClickUp.
pub fn status_dot(ticket: &Ticket) -> impl IntoElement {
    let colour = ticket
        .status_color
        .as_deref()
        .and_then(|hex| u32::from_str_radix(hex.trim_start_matches('#'), 16).ok())
        .map_or_else(|| theme::color(color::INK_FAINT), |value| rgb(value).into());
    div().flex_none().size(px(8.)).rounded_full().bg(colour)
}
