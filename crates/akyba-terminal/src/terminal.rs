//! One terminal: a shell in a pty, alacritty's engine keeping the screen, and the signal tap for
//! what alacritty doesn't read.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use akyba_core::links::{can_open, url_at};
use akyba_core::program_status::{PROBE_REPLY, ProgramStatus};
use akyba_core::pull_request::LinkScanner;
use alacritty_terminal::event::{Event as EngineEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::Processor;

use crate::palette;
use crate::pty::{Pty, PtyEvent};
use crate::signals::{Signal, SignalTap};

pub const SCROLLBACK_LINES: usize = 10_000;

/// What happened in a terminal that the app cares about.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalEvent {
    /// The terminal title changed; `None` when the program reset it.
    Title(Option<String>),
    Bell,
    /// Claude Code reported its state (OSC 7501).
    Status(ProgramStatus),
    Cwd(PathBuf),
    PromptStart,
    /// A notification the program sent (OSC 9).
    Notification(String),
    /// The program put text on the clipboard (OSC 52), as Claude Code does when you select in it.
    /// Reading the clipboard stays refused.
    Copy(String),
    /// A GitHub pull request was linked: an agent printed the one it opened, or you pasted one.
    PullRequestLink(String),
}

/// A link on screen: the cells it covers, for underlining, and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub start: Point,
    pub end: Point,
    pub target: String,
}

impl Link {
    pub fn contains(&self, point: Point) -> bool {
        self.start <= point && point <= self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.cols)
    }
}

/// Collects alacritty's events while it parses; drained right after.
#[derive(Clone, Default)]
pub struct EngineEvents(Arc<Mutex<Vec<EngineEvent>>>);

impl EventListener for EngineEvents {
    fn send_event(&self, event: EngineEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

impl EngineEvents {
    fn drain(&self) -> Vec<EngineEvent> {
        self.0.lock().map(|mut events| std::mem::take(&mut *events)).unwrap_or_default()
    }
}

pub struct Terminal {
    term: Term<EngineEvents>,
    processor: Processor,
    tap: SignalTap,
    links: LinkScanner,
    events: EngineEvents,
    pty: Pty,
    size: GridSize,
    /// The program asked to hear when the terminal turns light or dark (mode 2031), and the last
    /// scheme it was told.
    scheme_reports: Option<bool>,
}

impl Terminal {
    /// Starts a shell in `cwd`. The receiver delivers its output for [`Terminal::advance`].
    pub fn spawn(
        cwd: &Path,
        size: GridSize,
        extra_env: &[(&str, &str)],
    ) -> anyhow::Result<(Self, async_channel::Receiver<PtyEvent>)> {
        let (pty, output) = Pty::spawn(cwd, size.cols, size.rows, extra_env)?;
        let events = EngineEvents::default();
        let config = Config { scrolling_history: SCROLLBACK_LINES, ..Config::default() };
        let term = Term::new(config, &size, events.clone());
        let tap = SignalTap::default();
        Ok((
            Self {
                term,
                processor: Processor::new(),
                tap,
                links: LinkScanner::default(),
                events,
                pty,
                size,
                scheme_reports: None,
            },
            output,
        ))
    }

    /// The colour scheme, light or dark, as programs ask for it: once (`CSI ? 996 n`), or every time
    /// it changes once they turn reports on (`CSI ? 2031 h`). The engine doesn't know these.
    fn answer_scheme_requests(&mut self, bytes: &[u8]) {
        let has = |sequence: &[u8]| bytes.windows(sequence.len()).any(|window| window == sequence);
        if has(b"\x1b[?2031h") {
            self.scheme_reports = Some(palette::is_dark());
        }
        if has(b"\x1b[?2031l") {
            self.scheme_reports = None;
        }
        if has(b"\x1b[?996n") {
            self.pty.write(scheme_report(palette::is_dark()));
        }
    }

    /// Tells a program that asked for reports that the terminal turned light or dark. Called as
    /// frames are drawn; does nothing until the scheme actually changes.
    pub fn report_scheme(&mut self) {
        let dark = palette::is_dark();
        if self.scheme_reports.is_some_and(|told| told != dark) {
            self.scheme_reports = Some(dark);
            self.pty.write(scheme_report(dark));
        }
    }

    /// Feeds program output to the screen and returns what the app should know about.
    pub fn advance(&mut self, bytes: &[u8]) -> Vec<TerminalEvent> {
        let mut out = Vec::new();
        for signal in self.tap.advance(bytes) {
            match signal {
                Signal::StatusProbe => self.pty.write(PROBE_REPLY),
                Signal::Status(status) => out.push(TerminalEvent::Status(status)),
                Signal::Cwd(path) => out.push(TerminalEvent::Cwd(path)),
                Signal::PromptStart => out.push(TerminalEvent::PromptStart),
                Signal::Notification(text) => out.push(TerminalEvent::Notification(text)),
            }
        }
        out.extend(self.links.feed(bytes).map(TerminalEvent::PullRequestLink));
        self.answer_scheme_requests(bytes);
        self.processor.advance(&mut self.term, bytes);
        for event in self.events.drain() {
            match event {
                // Replies the program asked for: device attributes, cursor position…
                EngineEvent::PtyWrite(reply) => self.pty.write(reply.into_bytes()),
                // "What colour is your background?": how programs tell a light terminal from a dark one.
                EngineEvent::ColorRequest(index, reply) => {
                    let rgb = palette::requested(index, self.term.colors());
                    self.pty.write(reply(rgb).into_bytes());
                }
                EngineEvent::Title(title) => out.push(TerminalEvent::Title(Some(title))),
                EngineEvent::ResetTitle => out.push(TerminalEvent::Title(None)),
                EngineEvent::Bell => out.push(TerminalEvent::Bell),
                EngineEvent::ClipboardStore(_, text) => out.push(TerminalEvent::Copy(text)),
                _ => {}
            }
        }
        out
    }

    /// When a synchronized update (DEC 2026) is open: the moment to show it even if its end never
    /// arrives.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.processor.sync_timeout().sync_timeout()
    }

    /// Shows a synchronized update whose end marker did not arrive in time.
    pub fn flush_sync(&mut self) {
        self.processor.stop_sync(&mut self.term);
    }

    pub fn term(&self) -> &Term<EngineEvents> {
        &self.term
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size || size.cols == 0 || size.rows == 0 {
            return;
        }
        self.size = size;
        self.term.resize(size);
        self.pty.resize(size.cols, size.rows);
    }

    pub fn write(&mut self, bytes: impl Into<Vec<u8>>) {
        // Typing jumps back to the live screen, as in every terminal.
        if self.term.grid().display_offset() != 0 {
            self.term.scroll_display(Scroll::Bottom);
        }
        self.pty.write(bytes);
    }

    /// Scrolls the scrollback by whole lines; positive is up.
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    /// The grid point for a cell on screen (row and column counted from the top-left of the view).
    fn grid_point(&self, row: usize, col: usize) -> Point {
        let row = row.min(self.term.screen_lines().saturating_sub(1));
        let col = col.min(self.term.columns().saturating_sub(1));
        viewport_to_point(self.term.grid().display_offset(), Point::new(row, Column(col)))
    }

    /// Starts selecting at a cell: by character, by word (double click) or by line (triple click).
    pub fn start_selection(&mut self, row: usize, col: usize, side: Side, clicks: usize) {
        let kind = match clicks {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.selection = Some(Selection::new(kind, self.grid_point(row, col), side));
    }

    pub fn extend_selection(&mut self, row: usize, col: usize, side: Side) {
        let point = self.grid_point(row, col);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// The selected text, or `None` when nothing (or only empty space) is selected.
    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|text| !text.trim().is_empty())
    }

    /// The link at a cell on screen: one the program marked (OSC 8), as Claude Code does, or an
    /// address written out in the text.
    pub fn link_at(&self, row: usize, col: usize) -> Option<Link> {
        let point = self.grid_point(row, col);
        let grid = self.term.grid();
        if let Some(target) = grid[point].hyperlink().map(|link| link.uri().to_owned()) {
            // A marked link can run on over the next rows; it ends where the marking does.
            let same = |at: &Point| grid[*at].hyperlink().is_some_and(|link| link.uri() == target);
            let (mut start, mut end) = (point, point);
            while let Some(previous) = self.continues(start, false, same) {
                start = previous;
            }
            while let Some(next) = self.continues(end, true, same) {
                end = next;
            }
            return can_open(&target).then_some(Link { start, end, target });
        }

        // The whole line, across the rows it wrapped onto, since long addresses wrap.
        let wraps = |line: Line| grid[line][self.term.last_column()].flags.contains(Flags::WRAPLINE);
        let (mut first, mut last) = (point.line, point.line);
        while first > self.term.topmost_line() && wraps(first - 1) {
            first -= 1;
        }
        while last < self.term.bottommost_line() && wraps(last) {
            last += 1;
        }
        let columns = self.term.columns();
        let text: Vec<char> =
            (first.0..=last.0).flat_map(|line| grid[Line(line)].into_iter().map(|cell| cell.c)).collect();
        let index = usize::try_from(point.line.0 - first.0).ok()? * columns + point.column.0;
        let (range, target) = url_at(&text, index)?;
        let at = |index: usize| Some(Point::new(first + i32::try_from(index / columns).ok()?, Column(index % columns)));
        Some(Link { start: at(range.start)?, end: at(range.end - 1)?, target })
    }

    /// The next cell (or the one before) that is still the link `same` tells, from `at`: the very
    /// next one, or one on the next row past blank cells. A program that lays out its own text,
    /// as Claude Code does, breaks a long link's words onto a new row itself, leaving the rest of
    /// the row blank and perhaps indenting the next.
    fn continues(&self, at: Point, forward: bool, same: impl Fn(&Point) -> bool) -> Option<Point> {
        let grid = self.term.grid();
        let blank = |point: &Point| grid[*point].hyperlink().is_none() && matches!(grid[*point].c, ' ' | '\0');
        let mut cursor = self.neighbour(at, forward)?;
        while blank(&cursor) && (cursor.line.0 - at.line.0).abs() <= 1 {
            cursor = self.neighbour(cursor, forward)?;
        }
        // Blanks are skipped only to reach another row: within a row they end the link.
        let reached = cursor == self.neighbour(at, forward)? || cursor.line != at.line;
        (reached && (cursor.line.0 - at.line.0).abs() <= 1 && same(&cursor)).then_some(cursor)
    }

    /// The cell after (or before) a point, reading left to right and on to the next row.
    fn neighbour(&self, at: Point, forward: bool) -> Option<Point> {
        let last_column = self.term.last_column();
        match (forward, at.column) {
            (true, column) if column < last_column => Some(Point::new(at.line, column + 1)),
            (true, _) => (at.line < self.term.bottommost_line()).then(|| Point::new(at.line + 1, Column(0))),
            (false, Column(0)) => (at.line > self.term.topmost_line()).then(|| Point::new(at.line - 1, last_column)),
            (false, column) => Some(Point::new(at.line, column - 1)),
        }
    }

    pub fn foreground_pid(&self) -> Option<i32> {
        self.pty.foreground_pid()
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.pty.shell_pid()
    }

    pub fn kill(&mut self) {
        self.pty.kill();
    }
}

/// `CSI ? 997 ; 1 n` for dark, `; 2 n` for light.
fn scheme_report(dark: bool) -> Vec<u8> {
    format!("\x1b[?997;{}n", if dark { 1 } else { 2 }).into_bytes()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_the_colour_scheme_as_programs_expect() {
        assert_eq!(super::scheme_report(true), b"\x1b[?997;1n");
        assert_eq!(super::scheme_report(false), b"\x1b[?997;2n");
    }

    use super::*;

    #[test]
    fn selects_and_copies_screen_text() {
        let size = GridSize { cols: 40, rows: 5 };
        let (mut terminal, _output) = Terminal::spawn(Path::new("/tmp"), size, &[]).expect("a shell starts");
        terminal.advance(b"\x1b[2J\x1b[Hfirst line\r\nsecond word here");
        terminal.start_selection(1, 7, Side::Left, 2);
        assert_eq!(terminal.selected_text().as_deref(), Some("word"));
        terminal.start_selection(0, 0, Side::Left, 1);
        terminal.extend_selection(1, 5, Side::Right);
        assert_eq!(terminal.selected_text().as_deref(), Some("first line\nsecond"));
        terminal.clear_selection();
        assert_eq!(terminal.selected_text(), None);
    }

    #[test]
    fn finds_links_in_text_and_links_the_program_marked() {
        let size = GridSize { cols: 20, rows: 5 };
        let (mut terminal, _output) = Terminal::spawn(Path::new("/tmp"), size, &[]).expect("a shell starts");
        // A written-out address that wraps onto the next row, then a marked link with other words.
        terminal.advance(b"\x1b[2J\x1b[Hsee https://acme.dev/a/long/path ok\r\n");
        terminal.advance(b"\x1b]8;;https://acme.dev/docs\x1b\\the docs\x1b]8;;\x1b\\ here");

        let written = terminal.link_at(1, 2).expect("the wrapped address");
        assert_eq!(written.target, "https://acme.dev/a/long/path");
        assert_eq!((written.start, written.end), (Point::new(Line(0), Column(4)), Point::new(Line(1), Column(11))));
        assert_eq!(terminal.link_at(0, 1), None);

        let marked = terminal.link_at(2, 5).expect("the marked link");
        assert_eq!(marked.target, "https://acme.dev/docs");
        assert_eq!((marked.start, marked.end), (Point::new(Line(2), Column(0)), Point::new(Line(2), Column(7))));
        assert_eq!(terminal.link_at(2, 10), None);
    }

    #[test]
    fn a_marked_link_broken_onto_a_new_row_is_one_link() {
        let size = GridSize { cols: 20, rows: 5 };
        let (mut terminal, _output) = Terminal::spawn(Path::new("/tmp"), size, &[]).expect("a shell starts");
        // As Claude Code writes a long link's words: the row ends early, the next is indented.
        terminal.advance(b"\x1b[2J\x1b[H\x1b]8;;https://acme.dev/plan\x1b\\Manufacturing plan\x1b]8;;\x1b\\\r\n");
        terminal.advance(b"  \x1b]8;;https://acme.dev/plan\x1b\\upload\x1b]8;;\x1b\\ done");

        for (row, col) in [(0, 2), (1, 4)] {
            let link = terminal.link_at(row, col).expect("the link, from either row");
            assert_eq!((link.start, link.end), (Point::new(Line(0), Column(0)), Point::new(Line(1), Column(7))));
        }
        assert_eq!(terminal.link_at(1, 10), None);
    }

    #[test]
    fn runs_a_shell_and_reads_its_output_and_signals() {
        let size = GridSize { cols: 80, rows: 24 };
        let (mut terminal, output) =
            Terminal::spawn(Path::new("/tmp"), size, &[("TERM", "xterm-256color")]).expect("a shell starts");
        terminal.write(
            // A title, Claude's status, text for the clipboard ("copied", base64) and a pull request
            // link, which isn't typed whole: zsh holds a typed link back, and the output should find it.
            b"printf '\\033]0;ABC-236\\007\\033]7501;state=done:app=claude-code\\007\\033]52;c;Y29waWVk\\007'; echo akyba-$((40+2)); printf 'htt\\160s://github.com/acme/acme-web/pull/42\\n'; exit\r"
                .to_vec(),
        );

        let mut events = Vec::new();
        let mut screen = String::new();
        while let Ok(event) = output.recv_blocking() {
            match event {
                PtyEvent::Output(bytes) => events.extend(terminal.advance(&bytes)),
                PtyEvent::Exited(_) => break,
            }
        }
        for line in 0..terminal.term().screen_lines() {
            let row = &terminal.term().grid()[alacritty_terminal::index::Line(i32::try_from(line).expect("small"))];
            screen.extend(row.into_iter().map(|cell| cell.c));
        }
        assert!(screen.contains("akyba-42"), "screen: {screen}");
        assert!(events.contains(&TerminalEvent::Title(Some("ABC-236".into()))), "events: {events:?}");
        assert!(
            events.iter().any(|e| matches!(e, TerminalEvent::Status(s) if s.state == "done")),
            "events: {events:?}"
        );
        assert!(events.contains(&TerminalEvent::Copy("copied".into())), "events: {events:?}");
        assert!(
            events.contains(&TerminalEvent::PullRequestLink("https://github.com/acme/acme-web/pull/42".into())),
            "events: {events:?}"
        );
    }
}
