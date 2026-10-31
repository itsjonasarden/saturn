//! ClickUp's tickets as Akyba needs them: an id to start an agent on, a title, and a status to
//! show on the pane. Parsed from ClickUp's API v2 (<https://developer.clickup.com>); the HTTP calls
//! live in the app.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// One task, as the picker and the pane header show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// ClickUp's own id (`86c1abc2`), for API calls and links.
    pub id: String,
    /// The workspace's custom id (`ABC-236`) when it uses them; what people call the ticket.
    pub custom_id: Option<String>,
    pub title: String,
    pub status: String,
    /// The status's colour in ClickUp, `#rrggbb`.
    pub status_color: Option<String>,
    pub url: String,
    pub list: Option<String>,
    /// The space it's in: its id from the task, its name once the space list is read.
    pub space_id: Option<String>,
    pub space: Option<String>,
    /// `urgent`, `high`, `normal` or `low`, when it has one.
    pub priority: Option<String>,
    pub tags: Vec<String>,
    /// When it's due, in milliseconds since 1970.
    pub due: Option<i64>,
}

impl Ticket {
    /// What the ticket is called: its custom id, else ClickUp's id.
    pub fn key(&self) -> &str {
        self.custom_id.as_deref().unwrap_or(&self.id)
    }

    /// Whether a search matches its key, title or status, ignoring case.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || [self.key(), &self.title, &self.status].iter().any(|text| text.to_lowercase().contains(&query))
    }
}

#[derive(Deserialize)]
struct TaskJson {
    id: String,
    #[serde(default)]
    custom_id: Option<String>,
    name: String,
    status: StatusJson,
    url: String,
    #[serde(default)]
    list: Option<NamedJson>,
    #[serde(default)]
    space: Option<IdJson>,
    #[serde(default)]
    priority: Option<PriorityJson>,
    #[serde(default)]
    tags: Vec<NamedJson>,
    /// ClickUp sends dates as strings of milliseconds.
    #[serde(default)]
    due_date: Option<String>,
}

#[derive(Deserialize)]
struct IdJson {
    id: String,
}

#[derive(Deserialize)]
struct PriorityJson {
    priority: String,
}

#[derive(Deserialize)]
struct StatusJson {
    status: String,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Deserialize)]
struct NamedJson {
    name: String,
}

impl From<TaskJson> for Ticket {
    fn from(task: TaskJson) -> Self {
        Self {
            id: task.id,
            custom_id: task.custom_id.filter(|id| !id.is_empty()),
            title: task.name,
            status: task.status.status,
            status_color: task.status.color.filter(|color| color.starts_with('#')),
            url: task.url,
            list: task.list.map(|list| list.name),
            space_id: task.space.map(|space| space.id),
            space: None,
            priority: task.priority.map(|priority| priority.priority),
            tags: task.tags.into_iter().map(|tag| tag.name).collect(),
            due: task.due_date.and_then(|due| due.parse().ok()),
        }
    }
}

/// How many tasks ClickUp returns per page.
const PAGE_SIZE: usize = 100;

/// A page of `GET /team/{team_id}/task`: its tickets, and whether more pages follow.
pub fn parse_task_page(json: &str) -> Result<(Vec<Ticket>, bool), serde_json::Error> {
    #[derive(Deserialize)]
    struct Page {
        tasks: Vec<TaskJson>,
        #[serde(default)]
        last_page: Option<bool>,
    }
    let page: Page = serde_json::from_str(json)?;
    // ClickUp says whether this was the last page; without that, a full page (100) may have more.
    let more = page.last_page.map_or(page.tasks.len() >= PAGE_SIZE, |last| !last);
    Ok((page.tasks.into_iter().map(Ticket::from).collect(), more))
}

/// One task from `GET /task/{id}`.
pub fn parse_task(json: &str) -> Result<Ticket, serde_json::Error> {
    serde_json::from_str::<TaskJson>(json).map(Ticket::from)
}

/// The signed-in user's id, from `GET /user`.
pub fn parse_user_id(json: &str) -> Result<u64, serde_json::Error> {
    #[derive(Deserialize)]
    struct Body {
        user: User,
    }
    #[derive(Deserialize)]
    struct User {
        id: u64,
    }
    serde_json::from_str::<Body>(json).map(|body| body.user.id)
}

/// The workspaces the token can see, from `GET /team`, as (id, name).
pub fn parse_workspaces(json: &str) -> Result<Vec<(String, String)>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Body {
        teams: Vec<Team>,
    }
    #[derive(Deserialize)]
    struct Team {
        id: String,
        name: String,
    }
    serde_json::from_str::<Body>(json).map(|body| body.teams.into_iter().map(|team| (team.id, team.name)).collect())
}

/// ClickUp's priorities, most pressing first.
pub const PRIORITIES: [&str; 4] = ["urgent", "high", "normal", "low"];

/// When a ticket is due, as the filter offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Due {
    Overdue,
    NextWeek,
    NoDate,
}

impl Due {
    pub const ALL: [Self; 3] = [Self::Overdue, Self::NextWeek, Self::NoDate];

    pub fn name(self) -> &'static str {
        match self {
            Self::Overdue => "Overdue",
            Self::NextWeek => "Next 7 days",
            Self::NoDate => "No due date",
        }
    }

    /// Where a ticket falls, at `now` (milliseconds); `None` when due later than a week out.
    pub fn of(ticket: &Ticket, now: i64) -> Option<Self> {
        const WEEK: i64 = 7 * 24 * 60 * 60 * 1000;
        match ticket.due {
            None => Some(Self::NoDate),
            Some(due) if due < now => Some(Self::Overdue),
            Some(due) if due < now + WEEK => Some(Self::NextWeek),
            Some(_) => None,
        }
    }
}

/// Which tickets the Tickets panel shows. Within a kind any chosen value matches (to do or
/// review); across kinds all must (to do, and urgent). An empty kind lets everything through.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    pub statuses: BTreeSet<String>,
    pub priorities: BTreeSet<String>,
    pub spaces: BTreeSet<String>,
    pub lists: BTreeSet<String>,
    pub tags: BTreeSet<String>,
    pub due: BTreeSet<Due>,
}

impl Filter {
    /// How many values are chosen, for the filter button's badge.
    pub fn count(&self) -> usize {
        self.statuses.len()
            + self.priorities.len()
            + self.spaces.len()
            + self.lists.len()
            + self.tags.len()
            + self.due.len()
    }

    pub fn keeps(&self, ticket: &Ticket, now: i64) -> bool {
        let one_of = |chosen: &BTreeSet<String>, value: Option<&String>| {
            chosen.is_empty() || value.is_some_and(|value| chosen.contains(value))
        };
        one_of(&self.statuses, Some(&ticket.status))
            && one_of(&self.priorities, ticket.priority.as_ref())
            && one_of(&self.spaces, ticket.space.as_ref())
            && one_of(&self.lists, ticket.list.as_ref())
            && (self.tags.is_empty() || ticket.tags.iter().any(|tag| self.tags.contains(tag)))
            && (self.due.is_empty() || Due::of(ticket, now).is_some_and(|due| self.due.contains(&due)))
    }
}

/// Adds `value` to `set`, or takes it out if it's there.
pub fn toggle<T: Ord>(set: &mut BTreeSet<T>, value: T) {
    if !set.remove(&value) {
        set.insert(value);
    }
}

/// A workspace's spaces, from `GET /team/{team_id}/space`, as (id, name).
pub fn parse_spaces(json: &str) -> Result<Vec<(String, String)>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Body {
        spaces: Vec<Space>,
    }
    #[derive(Deserialize)]
    struct Space {
        id: String,
        name: String,
    }
    serde_json::from_str::<Body>(json).map(|body| body.spaces.into_iter().map(|space| (space.id, space.name)).collect())
}

/// Groups tickets by status for the picker, keeping ClickUp's order of first appearance.
pub fn by_status(tickets: &[Ticket]) -> Vec<(&str, Vec<&Ticket>)> {
    let mut groups: Vec<(&str, Vec<&Ticket>)> = Vec::new();
    for ticket in tickets {
        match groups.iter_mut().find(|(status, _)| *status == ticket.status) {
            Some((_, group)) => group.push(ticket),
            None => groups.push((&ticket.status, vec![ticket])),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r##"{
        "tasks": [
            {"id": "86c1abc2", "custom_id": "ABC-236", "name": "Dark mode toggle",
             "status": {"status": "building", "color": "#7b68ee", "type": "custom"},
             "url": "https://app.clickup.com/t/86c1abc2", "list": {"id": "9", "name": "Inbox"},
             "assignees": [{"id": 1}]},
            {"id": "86c1abc3", "custom_id": null, "name": "Filter on every list page",
             "status": {"status": "ready", "color": "#d3d3d3"},
             "url": "https://app.clickup.com/t/86c1abc3"}
        ],
        "last_page": false
    }"##;

    #[test]
    fn a_page_of_tasks_becomes_tickets() {
        let (tickets, more) = parse_task_page(PAGE).expect("parses");
        assert!(more);
        assert_eq!(tickets[0].key(), "ABC-236");
        assert_eq!(tickets[0].status, "building");
        assert_eq!(tickets[0].list.as_deref(), Some("Inbox"));
        assert_eq!(tickets[1].key(), "86c1abc3", "without a custom id, ClickUp's id names it");
    }

    #[test]
    fn search_looks_at_the_key_the_title_and_the_status() {
        let (tickets, _) = parse_task_page(PAGE).expect("parses");
        assert!(tickets[0].matches("abc-236"));
        assert!(tickets[0].matches("dark mode"));
        assert!(tickets[1].matches("READY"));
        assert!(!tickets[1].matches("dark mode"));
    }

    #[test]
    fn reads_priority_tags_and_due_date_and_filters_by_them() {
        let json = r##"{"id": "1", "name": "Pay out refunds", "status": {"status": "to do"}, "url": "u",
            "list": {"name": "Web"}, "priority": {"priority": "urgent", "color": "#f50000"},
            "tags": [{"name": "billing"}], "due_date": "1000"}"##;
        let ticket = parse_task(json).expect("parses");
        assert_eq!(
            (ticket.priority.as_deref(), ticket.tags.as_slice(), ticket.due),
            (Some("urgent"), &["billing".to_owned()][..], Some(1000))
        );
        let mut filter = Filter::default();
        assert!(filter.keeps(&ticket, 0));
        toggle(&mut filter.statuses, "to do".to_owned());
        toggle(&mut filter.statuses, "review".to_owned());
        assert!(filter.keeps(&ticket, 0), "any chosen status matches");
        toggle(&mut filter.priorities, "low".to_owned());
        assert!(!filter.keeps(&ticket, 0), "every kind must match");
        toggle(&mut filter.priorities, "low".to_owned());
        filter.due.insert(Due::Overdue);
        assert!(filter.keeps(&ticket, 5000) && !filter.keeps(&ticket, 0));
        assert_eq!(filter.count(), 3);
    }

    #[test]
    fn tickets_group_by_status_in_order() {
        let (tickets, _) = parse_task_page(PAGE).expect("parses");
        let groups = by_status(&tickets);
        assert_eq!(groups.iter().map(|(status, _)| *status).collect::<Vec<_>>(), ["building", "ready"]);
    }

    #[test]
    fn a_tickets_space_is_named_from_the_space_list() {
        let ticket =
            parse_task(r#"{"id": "1", "name": "n", "status": {"status": "to do"}, "url": "u", "space": {"id": "77"}}"#)
                .expect("parses");
        assert_eq!(ticket.space_id.as_deref(), Some("77"));
        let spaces = parse_spaces(r#"{"spaces": [{"id": "77", "name": "Shop", "private": false}]}"#).expect("parses");
        assert_eq!(spaces, [("77".to_owned(), "Shop".to_owned())]);
    }

    #[test]
    fn the_user_and_workspaces_are_read() {
        assert_eq!(parse_user_id(r#"{"user": {"id": 4242, "username": "j"}}"#).expect("parses"), 4242);
        let teams = parse_workspaces(r#"{"teams": [{"id": "9001", "name": "Acme", "members": []}]}"#).expect("parses");
        assert_eq!(teams, [("9001".to_owned(), "Acme".to_owned())]);
    }
}
