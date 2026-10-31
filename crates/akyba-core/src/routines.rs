//! Scheduled agents: routines Akyba starts on its own, an agent with a prompt in a folder, once a
//! day or every few hours. Akyba can only start what's due while it's open, so the book keeps every
//! run, and the ones whose time passed while it was closed (or the Mac slept) show as missed, to run
//! by hand. Kept in `routines.json` beside Akyba's settings.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike as _, Duration, NaiveTime, TimeZone, Weekday};
use serde::{Deserialize, Serialize};

/// A run this much later than its time is missed, not started: a routine for nine in the morning
/// shouldn't start itself in the evening when Akyba opens.
pub const GRACE: i64 = 15 * 60;

/// How many runs the book keeps.
const KEPT_RUNS: usize = 300;

/// When a routine runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Schedule {
    /// Once a day at a time (`09:00`), every day or on weekdays only.
    Daily {
        at: String,
        #[serde(default)]
        weekdays: bool,
    },
    /// Every few hours, counted from midnight.
    Every { hours: u32 },
}

impl Schedule {
    /// The schedule in words: "Every day at 09:00", "Weekdays at 09:00", "Every 4 hours".
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Daily { at, weekdays: false } => format!("Every day at {at}"),
            Self::Daily { at, weekdays: true } => format!("Weekdays at {at}"),
            Self::Every { hours: 1 } => "Every hour".to_owned(),
            Self::Every { hours } => format!("Every {hours} hours"),
        }
    }

    /// Whether it can ever run: a time that reads, a number of hours from 1 to 24.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Daily { at, .. } => parse_time(at).is_some(),
            Self::Every { hours } => (1..=24).contains(hours),
        }
    }

    /// Its times after `after`, up to and including `until`, in order.
    #[must_use]
    pub fn times<Tz: TimeZone>(&self, after: &DateTime<Tz>, until: &DateTime<Tz>) -> Vec<DateTime<Tz>> {
        let zone = after.timezone();
        let clock: Vec<NaiveTime> = match self {
            Self::Daily { at, .. } => parse_time(at).into_iter().collect(),
            Self::Every { hours } if (1..=24).contains(hours) => {
                (0..24).step_by(*hours as usize).filter_map(|hour| NaiveTime::from_hms_opt(hour, 0, 0)).collect()
            }
            Self::Every { .. } => Vec::new(),
        };
        let weekdays_only = matches!(self, Self::Daily { weekdays: true, .. });
        let mut times = Vec::new();
        let mut day = after.date_naive();
        while day <= until.date_naive() {
            let workday = !matches!(day.weekday(), Weekday::Sat | Weekday::Sun);
            if workday || !weekdays_only {
                for time in &clock {
                    // A time the clocks skip (the spring change) is left out that day.
                    if let Some(at) = zone.from_local_datetime(&day.and_time(*time)).earliest()
                        && at > *after
                        && at <= *until
                    {
                        times.push(at);
                    }
                }
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        times
    }

    /// Its next time after `now`, within the week ahead.
    #[must_use]
    pub fn next<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
        self.times(now, &(now.clone() + Duration::days(8))).into_iter().next()
    }
}

/// A moment as this Mac's clock shows it, for lists: `Mon 20 Oct 09:00`.
#[must_use]
pub fn local_label(seconds: i64) -> String {
    DateTime::from_timestamp(seconds, 0)
        .map(|at| at.with_timezone(&chrono::Local).format("%a %-d %b %H:%M").to_string())
        .unwrap_or_default()
}

/// A routine's next time after `now` (seconds since 1970), on this Mac's clock.
#[must_use]
pub fn next_time(routine: &Routine, now: i64) -> Option<i64> {
    let now = DateTime::from_timestamp(now, 0)?.with_timezone(&chrono::Local);
    routine.schedule.next(&now).map(|at| at.timestamp())
}

/// Looks at what came due, by this Mac's clock (see `Book::check`).
pub fn check_now(book: &mut Book) -> Vec<(String, i64)> {
    book.check(&chrono::Local::now())
}

/// `9:00` or `09:00`, a time of day.
fn parse_time(at: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(at.trim(), "%H:%M").ok()
}

/// An agent Akyba starts on a schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    pub name: String,
    /// Where it runs.
    pub folder: PathBuf,
    /// What the agent is asked, as its first prompt.
    pub prompt: String,
    pub schedule: Schedule,
    /// Paused routines keep their place but don't run.
    #[serde(default = "on")]
    pub on: bool,
    /// The agent it starts; routines made before Codex start Claude Code.
    #[serde(default = "claude")]
    pub agent: crate::agents::Agent,
}

fn claude() -> crate::agents::Agent {
    crate::agents::Agent::Claude
}

fn on() -> bool {
    true
}

/// One time a routine was due: started, or missed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub routine: String,
    /// When it was due, in seconds since 1970.
    pub due: i64,
    /// When it started; `None` when it was missed.
    pub ran: Option<i64>,
    /// The Claude session it started, once known, to resume.
    #[serde(default)]
    pub session: Option<String>,
}

/// The routines and their runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Book {
    #[serde(default)]
    pub routines: Vec<Routine>,
    /// Oldest first.
    #[serde(default)]
    pub runs: Vec<Run>,
    /// When what was due was last looked at; runs due after it haven't been seen yet.
    #[serde(default)]
    pub checked: i64,
}

impl Book {
    /// The book in `file`, or an empty one.
    #[must_use]
    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    /// # Errors
    /// When the file can't be written.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(file, text)
    }

    /// Looks at what came due since the last look, up to `now`: each routine's latest time is
    /// returned to start, unless it's more than `GRACE` late; every other is written down as missed.
    /// The first look only starts the clock.
    pub fn check<Tz: TimeZone>(&mut self, now: &DateTime<Tz>) -> Vec<(String, i64)> {
        let now_seconds = now.timestamp();
        let first = self.checked == 0;
        let since = now.timezone().timestamp_opt(self.checked, 0).single();
        self.checked = now_seconds;
        let Some(since) = since.filter(|_| !first) else { return Vec::new() };
        let mut start = Vec::new();
        for routine in self.routines.iter().filter(|routine| routine.on) {
            let times = routine.schedule.times(&since, now);
            for (index, time) in times.iter().enumerate() {
                let due = time.timestamp();
                if index + 1 == times.len() && now_seconds - due <= GRACE {
                    start.push((routine.id.clone(), due));
                } else {
                    self.runs.push(Run { routine: routine.id.clone(), due, ran: None, session: None });
                }
            }
        }
        self.trim();
        start
    }

    /// Writes down that a routine started, for its time `due`, at `now`.
    pub fn started(&mut self, routine: &str, due: i64, now: i64) {
        self.runs.push(Run { routine: routine.to_owned(), due, ran: Some(now), session: None });
        self.trim();
    }

    /// Ties the run of `routine` due at `due` to the Claude session it started.
    pub fn tie_session(&mut self, routine: &str, due: i64, session: &str) {
        if let Some(run) = self.runs.iter_mut().rev().find(|run| run.routine == routine && run.due == due) {
            run.session = Some(session.to_owned());
        }
    }

    /// A routine's name, for a run of it.
    #[must_use]
    pub fn routine(&self, id: &str) -> Option<&Routine> {
        self.routines.iter().find(|routine| routine.id == id)
    }

    fn trim(&mut self) {
        let extra = self.runs.len().saturating_sub(KEPT_RUNS);
        self.runs.drain(..extra);
    }
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn at(text: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(text).expect("a time")
    }

    fn routine(id: &str, schedule: Schedule) -> Routine {
        Routine {
            id: id.to_owned(),
            name: id.to_owned(),
            folder: PathBuf::from("/code/acme-web"),
            prompt: "Update the dependencies".to_owned(),
            schedule,
            on: true,
            agent: crate::agents::Agent::Claude,
        }
    }

    #[test]
    fn finds_daily_weekday_and_hourly_times() {
        let daily = Schedule::Daily { at: "09:00".to_owned(), weekdays: false };
        // Friday 8:00 to Monday 10:00, an hour ahead of UTC.
        let (friday, monday) = (at("2026-10-16T08:00:00+01:00"), at("2026-10-19T10:00:00+01:00"));
        assert_eq!(daily.times(&friday, &monday).len(), 4);
        let weekdays = Schedule::Daily { at: "9:00".to_owned(), weekdays: true };
        let times = weekdays.times(&friday, &monday);
        assert_eq!(
            times.iter().map(DateTime::to_rfc3339).collect::<Vec<_>>(),
            ["2026-10-16T09:00:00+01:00", "2026-10-19T09:00:00+01:00"]
        );
        let every = Schedule::Every { hours: 6 };
        let times = every.times(&at("2026-10-16T05:00:00+01:00"), &at("2026-10-16T13:00:00+01:00"));
        assert_eq!(
            times.iter().map(DateTime::to_rfc3339).collect::<Vec<_>>(),
            ["2026-10-16T06:00:00+01:00", "2026-10-16T12:00:00+01:00"]
        );
        assert_eq!(weekdays.next(&at("2026-10-17T12:00:00+01:00")), Some(at("2026-10-19T09:00:00+01:00")));
        assert_eq!(weekdays.describe(), "Weekdays at 9:00");
        assert!(!Schedule::Daily { at: "nine".to_owned(), weekdays: false }.is_valid());
        assert!(!Schedule::Every { hours: 0 }.is_valid());
    }

    #[test]
    fn starts_what_is_due_and_writes_down_what_was_missed() {
        let mut book = Book::default();
        book.routines.push(routine("deps", Schedule::Daily { at: "09:00".to_owned(), weekdays: false }));
        book.routines.push(Routine { on: false, ..routine("paused", Schedule::Every { hours: 1 }) });
        // The first look only starts the clock.
        assert_eq!(book.check(&at("2026-10-16T08:00:00+00:00")), Vec::new());
        // Closed over the weekend, opened on Monday at 9:05: Saturday and Sunday were missed.
        let due = book.check(&at("2026-10-19T09:05:00+00:00"));
        assert_eq!(due, [("deps".to_owned(), at("2026-10-19T09:00:00+00:00").timestamp())]);
        let missed: Vec<i64> = book.runs.iter().filter(|run| run.ran.is_none()).map(|run| run.due).collect();
        assert_eq!(missed.len(), 3);
        // Opened again at noon on Tuesday: too late to start Tuesday's.
        assert_eq!(book.check(&at("2026-10-20T12:00:00+00:00")), Vec::new());
        assert_eq!(book.runs.len(), 4);
        book.started("deps", due[0].1, due[0].1 + 5);
        book.tie_session("deps", due[0].1, "0b6f7c2e-1a2b-4c3d-9e8f-001122334455");
        assert_eq!(
            book.runs.last().and_then(|run| run.session.as_deref()),
            Some("0b6f7c2e-1a2b-4c3d-9e8f-001122334455")
        );
    }

    #[test]
    fn keeps_the_book_in_a_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("routines.json");
        let mut book = Book::default();
        book.routines.push(routine("deps", Schedule::Every { hours: 4 }));
        book.started("deps", 100, 105);
        book.save(&file).expect("saved");
        assert_eq!(Book::load(&file), book);
        assert_eq!(Book::load(&dir.path().join("none.json")), Book::default());
    }
}
