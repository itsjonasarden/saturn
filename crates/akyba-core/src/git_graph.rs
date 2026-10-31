//! The commit graph for the History view: commits from `git log --all` (newest first, parents
//! after their children), each placed in a lane, with the lines each row draws between lanes.
//!
//! The layout walks the commits keeping the lanes' "next expected commit". A commit takes the lane
//! that expects it (the leftmost, when several do: those branches meet here), or a free lane; then
//! its first parent continues in its lane and other parents get the lanes that expect them, or
//! new ones. Each row then knows its lines: from the top of the row down to its commit, straight
//! through for the lanes passing by, and from its commit down to its parents' lanes.

use std::path::Path;

/// The fields Akyba asks `git log` for, separated by unit separators, one commit per record
/// separator: hash, parents, refs, author, author time, subject.
const FORMAT: &str = "--format=%H%x1f%P%x1f%D%x1f%an%x1f%at%x1f%s%x1e";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    /// Branches and tags pointing here, as git decorates them (`HEAD -> main`, `origin/main`,
    /// `tag: v1.2`).
    pub refs: Vec<String>,
    pub author: String,
    /// Seconds since the epoch.
    pub time: i64,
    pub subject: String,
}

impl Commit {
    pub fn short_hash(&self) -> &str {
        &self.hash[..self.hash.len().min(7)]
    }
}

/// The newest `limit` commits of every branch in `repo`.
pub fn log(repo: &Path, limit: usize) -> Vec<Commit> {
    let output = crate::git::command(repo)
        .args(["--no-optional-locks", "log", "--all", "--topo-order", FORMAT, &format!("-n{limit}")])
        .output();
    match output {
        Ok(output) if output.status.success() => parse(&String::from_utf8_lossy(&output.stdout)),
        _ => Vec::new(),
    }
}

pub fn parse(raw: &str) -> Vec<Commit> {
    raw.split('\u{1e}')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').split('\u{1f}');
            let hash = fields.next()?.trim().to_owned();
            if hash.is_empty() {
                return None;
            }
            let parents = fields.next()?.split_whitespace().map(str::to_owned).collect();
            let refs = fields.next()?.split(", ").filter(|r| !r.is_empty()).map(str::to_owned).collect();
            let author = fields.next()?.to_owned();
            let time = fields.next()?.parse().unwrap_or_default();
            let subject = fields.next().unwrap_or_default().to_owned();
            Some(Commit { hash, parents, refs, author, time, subject })
        })
        .collect()
}

/// Where a line in a row starts and ends: the top edge, the commit's dot, or the bottom edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    /// From the top of the row to the commit (a child's line arriving).
    TopToDot,
    /// Straight through the row (a lane passing by), possibly shifting lane.
    TopToBottom,
    /// From the commit to the bottom of the row (towards a parent).
    DotToBottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub from: usize,
    pub to: usize,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The commit's lane.
    pub lane: usize,
    pub lines: Vec<Line>,
    /// How many lanes this row touches, for the graph column's width.
    pub width: usize,
}

pub fn layout(commits: &[Commit]) -> Vec<Row> {
    let mut lanes: Vec<Option<&str>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let before = lanes.clone();
        let expecting: Vec<usize> = (0..lanes.len()).filter(|&i| lanes[i] == Some(commit.hash.as_str())).collect();
        let lane = match expecting.first() {
            Some(&lane) => lane,
            None => free_lane(&mut lanes),
        };
        // Branches that meet here end in this commit's lane.
        for &other in expecting.iter().skip(1) {
            lanes[other] = None;
        }
        lanes[lane] = None;
        // The first parent continues in this lane, even when another lane expects it too: the
        // lanes join where it appears. A merge's other parents join a lane that already expects
        // them, or open one.
        for (index, parent) in commit.parents.iter().enumerate() {
            if index == 0 {
                lanes[lane] = Some(parent.as_str());
            } else if !lanes.contains(&Some(parent.as_str())) {
                let target = free_lane(&mut lanes);
                lanes[target] = Some(parent.as_str());
            }
        }
        while lanes.last() == Some(&None) {
            lanes.pop();
        }

        // Where `hash` continues below this row: in `near` when that lane still expects it, else in
        // the leftmost lane that does.
        let below = |hash: &str, near: usize| {
            if lanes.get(near) == Some(&Some(hash)) { Some(near) } else { lanes.iter().position(|l| *l == Some(hash)) }
        };
        let mut drawn = Vec::new();
        for (from, expected) in before.iter().enumerate() {
            let Some(expected) = expected else { continue };
            if *expected == commit.hash {
                drawn.push(Line { from, to: lane, span: Span::TopToDot });
            } else if let Some(to) = below(expected, from) {
                drawn.push(Line { from, to, span: Span::TopToBottom });
            }
        }
        for parent in &commit.parents {
            if let Some(to) = below(parent, lane) {
                drawn.push(Line { from: lane, to, span: Span::DotToBottom });
            }
        }
        let width = before.len().max(lanes.len()).max(lane + 1);
        rows.push(Row { lane, lines: drawn, width });
    }
    rows
}

/// The first lane nothing expects, or a new one.
fn free_lane(lanes: &mut Vec<Option<&str>>) -> usize {
    if let Some(free) = lanes.iter().position(Option::is_none) {
        free
    } else {
        lanes.push(None);
        lanes.len() - 1
    }
}

/// When a commit was made, as the History view shows it: `just now`, `12m ago`, `3h ago`, `5d ago`,
/// then the date (`18 Oct 2026`). `now` and `time` are seconds since the epoch, in UTC.
pub fn when(time: i64, now: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let age = now - time;
    match age {
        ..60 => "just now".to_owned(),
        60..3_600 => format!("{}m ago", age / 60),
        3_600..86_400 => format!("{}h ago", age / 3_600),
        86_400..604_800 => format!("{}d ago", age / 86_400),
        _ => {
            let (year, month, day) = civil_date(time.div_euclid(86_400));
            format!("{day} {} {year}", MONTHS[usize::try_from(month - 1).unwrap_or_default()])
        }
    }
}

/// The calendar date of a day counted from 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_date(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.to_owned(),
            parents: parents.iter().map(|p| (*p).to_owned()).collect(),
            refs: Vec::new(),
            author: "a".to_owned(),
            time: 0,
            subject: hash.to_owned(),
        }
    }

    #[test]
    fn says_when_in_words_then_as_a_date() {
        let now = 1_792_339_200; // 18 Oct 2026, 00:00 UTC
        assert_eq!(when(now - 30, now), "just now");
        assert_eq!(when(now - 12 * 60, now), "12m ago");
        assert_eq!(when(now - 3 * 3_600, now), "3h ago");
        assert_eq!(when(now - 5 * 86_400, now), "5d ago");
        assert_eq!(when(now, now + 30 * 86_400), "18 Oct 2026");
        assert_eq!(when(951_782_400, now), "29 Feb 2000");
    }

    #[test]
    fn parses_git_log_records() {
        let raw = "aaa\u{1f}bbb ccc\u{1f}HEAD -> main, origin/main, tag: v1\u{1f}Ada\u{1f}1700000000\u{1f}fix: a thing\u{1e}\nbbb\u{1f}\u{1f}\u{1f}Bo\u{1f}1690000000\u{1f}init\u{1e}\n";
        let commits = parse(raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, ["bbb", "ccc"]);
        assert_eq!(commits[0].refs, ["HEAD -> main", "origin/main", "tag: v1"]);
        assert_eq!(commits[0].time, 1_700_000_000);
        assert_eq!(commits[0].subject, "fix: a thing");
        assert!(commits[1].parents.is_empty() && commits[1].refs.is_empty());
    }

    #[test]
    fn a_straight_history_stays_in_one_lane() {
        let rows = layout(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert!(rows.iter().all(|row| row.lane == 0 && row.width == 1));
        assert_eq!(
            rows[1].lines,
            [Line { from: 0, to: 0, span: Span::TopToDot }, Line { from: 0, to: 0, span: Span::DotToBottom }]
        );
        assert_eq!(rows[2].lines, [Line { from: 0, to: 0, span: Span::TopToDot }]);
    }

    #[test]
    fn a_branch_and_its_merge_take_a_second_lane() {
        // m merges f into main: m(b, f), f(a), b(a), a.
        let rows = layout(&[commit("m", &["b", "f"]), commit("f", &["a"]), commit("b", &["a"]), commit("a", &[])]);
        assert_eq!(rows.iter().map(|row| row.lane).collect::<Vec<_>>(), [0, 1, 0, 0]);
        // The merge sends a line to each parent's lane.
        assert!(rows[0].lines.contains(&Line { from: 0, to: 1, span: Span::DotToBottom }));
        // While f's row is drawn, main's lane passes by.
        assert!(rows[1].lines.contains(&Line { from: 0, to: 0, span: Span::TopToBottom }));
        // Both lanes meet in a: f's lane arrives from lane 1.
        assert!(rows[3].lines.contains(&Line { from: 1, to: 0, span: Span::TopToDot }));
        assert_eq!(rows[3].width, 2);
    }

    #[test]
    fn two_tips_get_two_lanes_and_meet_at_their_base() {
        let rows = layout(&[commit("x", &["a"]), commit("y", &["a"]), commit("a", &[])]);
        assert_eq!(rows.iter().map(|row| row.lane).collect::<Vec<_>>(), [0, 1, 0]);
        assert!(rows[2].lines.contains(&Line { from: 1, to: 0, span: Span::TopToDot }));
    }
}
