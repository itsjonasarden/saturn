//! The Tickets panel, on the window's right (⌘⌥B): the ClickUp tickets assigned to you, by status,
//! searchable and filtered by status, priority, list, tag and due date. Clicking one starts an agent
//! on it, as the ticket picker does; one that already has a pane says so. Without ClickUp it says
//! how to connect it.

use std::collections::{BTreeSet, HashSet};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};
use saturn_core::clickup::{self, Due, Filter, Ticket};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::{self, len};
use crate::ticket_picker::status_dot;
use crate::workspace::{OpenSettings, Workspace};

pub struct TicketsView {
    workspace: WeakEntity<Workspace>,
    search: Entity<InputState>,
    /// The filters are open under the search.
    filtering: bool,
    _search_events: Subscription,
    /// Redraws when the workspace's tickets change.
    _workspace: Option<Subscription>,
}

impl TicketsView {
    pub fn new(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search tickets"));
        let search_events = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let observed = workspace.upgrade().map(|workspace| cx.observe(&workspace, |_, _, cx| cx.notify()));
        // Spikes open the filters (`SATURN_TICKET_FILTERS=1`).
        let filtering = std::env::var_os("SATURN_TICKET_FILTERS").is_some();
        Self { workspace, search, filtering, _search_events: search_events, _workspace: observed }
    }

    /// Changes the filter, which Saturn remembers.
    fn change_filter(&self, cx: &mut App, change: impl FnOnce(&mut Filter)) {
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.change_settings(|settings| change(&mut settings.ticket_filter), cx);
        });
    }

    /// One value to filter by: raised in blue while chosen.
    fn chip(&self, id: String, label: String, on: bool, change: impl Fn(&mut Filter) + 'static) -> AnyElement {
        let view = self.workspace.clone();
        div()
            .id(gpui_kit::SharedString::from(id))
            .flex()
            .items_center()
            .h(px(24.))
            .px(len(space::SPACE_2))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .text_size(px(12.))
            .map(|chip| {
                if on {
                    chip.bg(theme::color(color::BLUE).opacity(0.16)).text_color(theme::color(color::BLUE))
                } else {
                    chip.bg(theme::fill_control())
                        .hover(|s| s.bg(theme::fill_pressed()))
                        .text_color(theme::color(color::INK_MUTED))
                }
            })
            .child(label)
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |workspace, cx| {
                    workspace.change_settings(|settings| change(&mut settings.ticket_filter), cx);
                });
            })
            .into_any_element()
    }

    /// The filters: for each kind, the values the tickets have, as chips.
    fn render_filters(&self, tickets: &[Ticket], filter: &Filter, cx: &mut Context<Self>) -> impl IntoElement {
        let mut statuses: Vec<String> = Vec::new();
        for ticket in tickets {
            if !statuses.contains(&ticket.status) {
                statuses.push(ticket.status.clone());
            }
        }
        statuses
            .extend(filter.statuses.iter().filter(|status| !statuses.contains(status)).cloned().collect::<Vec<_>>());
        let present = |values: &mut dyn Iterator<Item = &String>, chosen: &BTreeSet<String>| -> Vec<String> {
            let mut all: BTreeSet<String> = values.cloned().collect();
            all.extend(chosen.iter().cloned());
            all.into_iter().collect()
        };
        let priorities: Vec<String> = clickup::PRIORITIES
            .iter()
            .map(|&priority| priority.to_owned())
            .filter(|priority| {
                filter.priorities.contains(priority) || tickets.iter().any(|t| t.priority.as_ref() == Some(priority))
            })
            .collect();
        let spaces = present(&mut tickets.iter().filter_map(|ticket| ticket.space.as_ref()), &filter.spaces);
        let lists = present(&mut tickets.iter().filter_map(|ticket| ticket.list.as_ref()), &filter.lists);
        let tags = present(&mut tickets.iter().flat_map(|ticket| ticket.tags.iter()), &filter.tags);
        let group = |title: &'static str, chips: Vec<AnyElement>| {
            (!chips.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(len(space::SPACE_1))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(color::INK_FAINT))
                            .child(title.to_uppercase()),
                    )
                    .child(div().flex().flex_wrap().gap(len(space::SPACE_1)).children(chips))
            })
        };
        let chips = |kind: &'static str,
                     values: Vec<String>,
                     chosen: &BTreeSet<String>,
                     pick: fn(&mut Filter) -> &mut BTreeSet<String>| {
            values
                .into_iter()
                .map(|value| {
                    let on = chosen.contains(&value);
                    let label = if kind == "priority" { capitalised(&value) } else { value.clone() };
                    self.chip(format!("filter-{kind}-{value}"), label, on, move |filter| {
                        clickup::toggle(pick(filter), value.clone());
                    })
                })
                .collect::<Vec<_>>()
        };
        let due = Due::ALL
            .into_iter()
            .map(|due| {
                self.chip(
                    format!("filter-due-{}", due.name()),
                    due.name().to_owned(),
                    filter.due.contains(&due),
                    move |filter| {
                        clickup::toggle(&mut filter.due, due);
                    },
                )
            })
            .collect();
        let count = filter.count();
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_3))
            .px(len(space::SPACE_2))
            .py(len(space::SPACE_2))
            .border_b_1()
            .border_color(theme::separator())
            .children(group("Status", chips("status", statuses, &filter.statuses, |filter| &mut filter.statuses)))
            .children(group(
                "Priority",
                chips("priority", priorities, &filter.priorities, |filter| &mut filter.priorities),
            ))
            .children(group("Due", due))
            .children(group("Space", chips("space", spaces, &filter.spaces, |filter| &mut filter.spaces)))
            .children(group("List", chips("list", lists, &filter.lists, |filter| &mut filter.lists)))
            .children(group("Tags", chips("tag", tags, &filter.tags, |filter| &mut filter.tags)))
            .when(count > 0, |filters| {
                filters.child(
                    div()
                        .id("filter-clear")
                        .text_size(px(12.))
                        .text_color(theme::color(color::BLUE))
                        .cursor_pointer()
                        .child("Clear filters")
                        .on_click(
                            cx.listener(|this, _, _, cx| this.change_filter(cx, |filter| *filter = Filter::default())),
                        ),
                )
            })
    }

    fn render_ticket(ticket: &Ticket, running: bool, cx: &mut Context<Self>) -> AnyElement {
        let (picked, url) = (ticket.clone(), ticket.url.clone());
        let group = gpui_kit::SharedString::from(format!("ticket-{}", ticket.id));
        div()
            .id(gpui_kit::SharedString::from(format!("panel-ticket-{}", ticket.id)))
            .group(group.clone())
            .flex()
            .items_start()
            .gap(len(space::SPACE_2))
            .px(len(space::SPACE_2))
            .py(len(space::SPACE_2))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .hover(|s| s.bg(theme::fill_hover()))
            .child(div().pt(px(5.)).child(status_dot(ticket)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(len(space::SPACE_2))
                            .text_size(px(11.))
                            .font_family(saturn_design::font::MONO)
                            .text_color(theme::color(color::INK_MUTED))
                            .child(ticket.key().to_owned())
                            .when(running, |line| {
                                line.child(
                                    div()
                                        .font_family(saturn_design::look().sans)
                                        .text_color(theme::color(color::BLUE))
                                        .child("● agent running"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .text_color(theme::color(color::INK))
                            .child(ticket.title.clone()),
                    ),
            )
            .child(
                chrome::icon_button(gpui_kit::SharedString::from(format!("open-{}", ticket.id)), "external-link", true)
                    .invisible()
                    .group_hover(group, gpui_kit::Styled::visible)
                    .tooltip(chrome::tip("Open in ClickUp"))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                    }),
            )
            .tooltip(chrome::tip("Start an agent on this ticket"))
            .on_click(cx.listener(move |this, _, window, cx| {
                let agent = crate::agent_choice::current(cx);
                let _ =
                    this.workspace.update(cx, |workspace, cx| workspace.start_ticket(&picked, Some(agent), window, cx));
            }))
            .into_any_element()
    }
}

impl TicketsView {
    /// Opens and closes the filters; a badge says how many are on.
    fn render_filter_button(&self, count: usize, cx: &mut Context<Self>) -> impl IntoElement {
        chrome::icon_button("tickets-filter", "list-filter", true)
            .relative()
            .when(self.filtering, |button| button.bg(theme::fill_pressed()))
            .when(count > 0, |button| {
                button.child(
                    div()
                        .absolute()
                        .top(px(1.))
                        .right(px(1.))
                        .min_w(px(14.))
                        .h(px(14.))
                        .px(px(3.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme::color(color::BLUE))
                        .text_size(px(9.))
                        .text_color(theme::color(color::INK_ON_ACCENT))
                        .child(count.to_string()),
                )
            })
            .tooltip(chrome::tip("Filter by status, priority, due date, space, list or tag"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.filtering = !this.filtering;
                cx.notify();
            }))
    }
}

impl TicketsView {
    /// The tickets, under a heading for each status.
    fn render_list(tickets: &[Ticket], running: &HashSet<String>, cx: &mut Context<Self>) -> AnyElement {
        let groups = clickup::by_status(tickets);
        let mut list = div().id("tickets-list").flex_1().min_h_0().overflow_y_scroll().flex().flex_col();
        for (status, group) in groups {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .pt(len(space::SPACE_3))
                    .pb(len(space::SPACE_1))
                    .px(len(space::SPACE_2))
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(color::INK_FAINT))
                    .child(status.to_uppercase())
                    .child(group.len().to_string()),
            );
            for ticket in group {
                list = list.child(Self::render_ticket(ticket, running.contains(ticket.key()), cx));
            }
        }
        list.into_any_element()
    }
}

/// `urgent` → `Urgent`.
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

impl Render for TicketsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(workspace) = self.workspace.upgrade() else { return div().into_any_element() };
        let state = workspace.read(cx).clickup();
        let query = self.search.read(cx).value().to_string();
        let filter = workspace.read(cx).settings().ticket_filter.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0));
        let all = state.tickets.clone();
        let tickets: Vec<Ticket> =
            all.iter().filter(|ticket| ticket.matches(&query) && filter.keeps(ticket, now)).cloned().collect();
        let running: HashSet<String> = workspace.read(cx).tickets_with_agents(cx);
        let note = if !state.connected() {
            Some("Connect ClickUp in Settings to see the tickets assigned to you here.")
        } else if state.loading {
            Some("Reading your tickets…")
        } else if tickets.is_empty() && all.is_empty() {
            Some("No open tickets assigned to you.")
        } else if tickets.is_empty() {
            Some(if filter.count() > 0 { "No ticket matches the search and filters." } else { "No ticket matches." })
        } else {
            None
        };
        let problem = state.problem.clone();
        let connected = state.connected();
        let list = Self::render_list(&tickets, &running, cx);
        chrome::panel()
            .pb(len(space::SPACE_1))
            .gap(len(space::SPACE_1))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .h(len(chrome::CONTROL_HEIGHT))
                    .pl(len(space::SPACE_2))
                    .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child("Tickets"))
                    .when(connected, |head| {
                        head.child(self.render_filter_button(filter.count(), cx)).child(
                            chrome::icon_button("tickets-refresh", "refresh-cw", true)
                                .tooltip(chrome::tip("Read the tickets again"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let _ = this.workspace.update(cx, Workspace::refresh_tickets);
                                })),
                        )
                    }),
            )
            .when(connected, |panel| {
                panel.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(len(space::SPACE_2))
                        .h(len(chrome::CONTROL_HEIGHT))
                        .px(len(space::SPACE_2))
                        .rounded(theme::corner(radius::RADIUS_SM))
                        .bg(theme::fill_control())
                        .child(chrome::icon("search", 13., theme::color(color::INK_FAINT)))
                        .child(div().flex_1().child(Input::new(&self.search).appearance(false))),
                )
            })
            .when(connected && self.filtering, |panel| panel.child(self.render_filters(&all, &filter, cx)))
            .children(problem.map(|problem| {
                div().px(len(space::SPACE_2)).text_size(px(12.)).text_color(theme::color(color::RED)).child(problem)
            }))
            .children(note.map(|note| {
                div()
                    .flex()
                    .flex_col()
                    .gap(len(space::SPACE_3))
                    .p(len(space::SPACE_2))
                    .text_size(px(13.))
                    .text_color(theme::color(color::INK_MUTED))
                    .child(note)
                    .when(!connected, |note| {
                        note.child(
                            chrome::button("tickets-connect", "Open Settings", Some("⌘,"), false)
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
                        )
                    })
            }))
            .child(list)
            .into_any_element()
    }
}
