//! A terminal as a GPUI view: owns the [`Terminal`], turns its output into events for the app,
//! and handles keys, paste and scrolling.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use akyba_core::agent_state::AgentState;
use alacritty_terminal::index::Side;
use alacritty_terminal::term::TermMode;
use gpui_kit::{
    App, ClipboardItem, Context, CursorStyle, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyBinding, KeyDownEvent, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};

use crate::element::TerminalElement;
use crate::keys::{self, KeyPress};
use crate::pty::PtyEvent;
use crate::terminal::{GridSize, Link, Terminal, TerminalEvent};

gpui_kit::actions!(terminal, [SendTab, SendBackTab]);

/// Option acts as Meta in every terminal (Settings → Terminal).
pub static OPTION_AS_META: AtomicBool = AtomicBool::new(false);
/// Selecting text copies it when the mouse is let go (Settings → Terminal).
pub static COPY_ON_SELECT: AtomicBool = AtomicBool::new(false);

/// Tab and shift-tab belong to the shell (completion, Claude Code's suggestions). The window binds
/// them to moving focus between controls, and bindings run before a view sees its keys, so the
/// terminal binds them again, closer in: the nearer binding wins.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", SendTab, Some("Terminal")),
        KeyBinding::new("shift-tab", SendBackTab, Some("Terminal")),
    ]);
}

/// What a terminal tells the app.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalViewEvent {
    TitleChanged,
    StateChanged,
    Bell,
    /// The shell moved to another folder (OSC 7), such as an agent's worktree.
    CwdChanged,
    /// The shell ended; the tab should close.
    Exited,
    /// Someone typed in it or clicked in it: they've seen what it says.
    Used,
    /// A pull request was linked in it ([`TerminalView::pull_request_link`]).
    PullRequestLinked,
    /// The shell drew its prompt (OSC 133): whatever ran in the foreground has ended.
    PromptShown,
    /// The program sent a notification (OSC 9), with its text.
    Notified(String),
}

/// Where the grid was last drawn: enough to turn a mouse position into a cell.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub origin: Point<Pixels>,
    pub cell_width: Pixels,
    pub line_height: Pixels,
}

/// What a held mouse button is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Selecting,
    /// The program asked for mouse events (Claude's full-screen view does); they go to it.
    Reporting,
}

pub struct TerminalView {
    terminal: Terminal,
    focus: FocusHandle,
    /// The terminal's own title as set by the program.
    pub title: Option<String>,
    pub state: AgentState,
    /// Claude's current activity or question (OSC 7501 `msg`).
    pub message: Option<String>,
    /// How far Claude says it is, 0–100 (OSC 7501 `progress`), when it says.
    pub progress: Option<u8>,
    pub cwd: PathBuf,
    /// The pull request last linked in the terminal, by the agent or by you.
    pub pull_request_link: Option<String>,
    /// Claude reports its state itself (OSC 7501); hook-based state is then ignored.
    has_program_status: bool,
    scroll_remainder: f32,
    geometry: Rc<Cell<Option<Geometry>>>,
    drag: Drag,
    /// The cell a drag the program follows was last reported at, so each cell is reported once.
    reported: Option<(usize, usize)>,
    /// Where the mouse last was over the terminal, and whether Command was held.
    pointer: Option<(Point<Pixels>, bool)>,
    /// The link under the mouse while Command is held: underlined, and opened on click.
    link: Option<Link>,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    /// Starts a shell in `cwd` (with `extra_env` added to its environment) for a new view.
    pub fn spawn(
        cwd: &Path,
        extra_env: &[(&str, &str)],
    ) -> anyhow::Result<(Terminal, async_channel::Receiver<PtyEvent>)> {
        Terminal::spawn(cwd, GridSize { cols: 80, rows: 24 }, extra_env)
    }

    /// Shows a terminal started with [`TerminalView::spawn`].
    pub fn new(
        terminal: Terminal,
        output: async_channel::Receiver<PtyEvent>,
        cwd: &Path,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            while let Ok(first) = output.recv().await {
                // Everything already waiting is handled in the same frame.
                let mut batch = vec![first];
                while let Ok(next) = output.try_recv() {
                    batch.push(next);
                }
                if this.update(cx, |view, cx| view.handle_output(batch, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            terminal,
            focus: cx.focus_handle(),
            title: None,
            state: AgentState::Shell,
            message: None,
            progress: None,
            cwd: cwd.to_owned(),
            pull_request_link: None,
            has_program_status: false,
            scroll_remainder: 0.0,
            geometry: Rc::default(),
            drag: Drag::None,
            reported: None,
            pointer: None,
            link: None,
        }
    }

    fn handle_output(&mut self, batch: Vec<PtyEvent>, cx: &mut Context<Self>) {
        let mut events = Vec::new();
        for item in batch {
            match item {
                PtyEvent::Output(bytes) => events.extend(self.terminal.advance(&bytes)),
                PtyEvent::Exited(_) => {
                    cx.emit(TerminalViewEvent::Exited);
                    return;
                }
            }
        }
        for event in events {
            match event {
                TerminalEvent::Title(title) => {
                    self.title = title;
                    cx.emit(TerminalViewEvent::TitleChanged);
                }
                TerminalEvent::Bell => cx.emit(TerminalViewEvent::Bell),
                TerminalEvent::Copy(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                TerminalEvent::Status(status) => {
                    self.has_program_status = true;
                    self.state = AgentState::from_program_status(&status);
                    self.message = status.msg;
                    self.progress = status.progress;
                    cx.emit(TerminalViewEvent::StateChanged);
                }
                TerminalEvent::Cwd(path) => {
                    if path != self.cwd {
                        self.cwd = path;
                        cx.emit(TerminalViewEvent::CwdChanged);
                    }
                }
                TerminalEvent::PullRequestLink(link) => {
                    if self.pull_request_link.as_ref() != Some(&link) {
                        self.pull_request_link = Some(link);
                        cx.emit(TerminalViewEvent::PullRequestLinked);
                    }
                }
                TerminalEvent::PromptStart => {
                    // Back at the shell: an agent that reports no state of its own has ended.
                    if !self.has_program_status && self.state != AgentState::Shell {
                        self.state = AgentState::Shell;
                        cx.emit(TerminalViewEvent::StateChanged);
                    }
                    cx.emit(TerminalViewEvent::PromptShown);
                }
                TerminalEvent::Notification(text) => cx.emit(TerminalViewEvent::Notified(text)),
            }
        }
        self.schedule_sync_flush(cx);
        // New output can move the text under the mouse.
        self.refresh_link();
        cx.notify();
    }

    /// An open synchronized update must still show if its end marker never arrives.
    fn schedule_sync_flush(&self, cx: &mut Context<Self>) {
        let Some(deadline) = self.terminal.sync_deadline() else { return };
        let wait = deadline.saturating_duration_since(Instant::now());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |view, cx| {
                if view.terminal.sync_deadline().is_some_and(|due| due <= Instant::now()) {
                    view.terminal.flush_sync();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// State read from the title of an agent that sets one but reports nothing else (Codex), as
    /// [`akyba_core::agent_state::from_codex_title`] reads it. Codex empties the title as it quits:
    /// then it's a shell again, and this says so (`true`).
    pub fn codex_title_state(&mut self, cx: &mut Context<Self>) -> bool {
        let title = self.title.clone().unwrap_or_default();
        if title.trim().is_empty() {
            self.hook_state(AgentState::Shell, cx);
            return true;
        }
        let state = akyba_core::agent_state::from_codex_title(&title, self.state);
        self.hook_state(state, cx);
        false
    }

    /// State reported by a Claude Code hook. Used only when Claude doesn't report its state to the
    /// terminal itself (OSC 7501 is the more precise of the two).
    pub fn hook_state(&mut self, state: AgentState, cx: &mut Context<Self>) {
        if self.has_program_status || self.state == state {
            return;
        }
        self.state = state;
        cx.emit(TerminalViewEvent::StateChanged);
        cx.notify();
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    pub(crate) fn resize(&mut self, size: GridSize) {
        self.terminal.resize(size);
    }

    /// The program in the foreground, for "close this tab?" questions.
    pub fn foreground_pid(&self) -> Option<i32> {
        self.terminal.foreground_pid()
    }

    /// The terminal's shell, under which runs everything started in it.
    pub fn shell_pid(&self) -> Option<u32> {
        self.terminal.shell_pid()
    }

    pub fn write(&mut self, bytes: impl Into<Vec<u8>>) {
        self.terminal.write(bytes);
    }

    /// Shows a line from Akyba in the terminal, faint, without the shell seeing it: for a new
    /// terminal, before its prompt draws.
    pub fn note(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.replace('\n', "\r\n");
        let _ = self.terminal.advance(format!("\x1b[2m{text}\x1b[0m\r\n").as_bytes());
        cx.notify();
    }

    /// Pastes text, wrapped in bracketed-paste markers when the program asked for them.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.terminal.mode().contains(TermMode::BRACKETED_PASTE) {
            self.write(format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes());
        } else {
            self.write(text.into_bytes());
        }
    }

    /// Pastes the clipboard: text as text, copied files and images as paths an agent attaches.
    pub fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let folder = std::env::temp_dir().join("akyba-pasted");
        if let Some(text) = cx.read_from_clipboard().and_then(|item| keys::paste_text(&item, &folder)) {
            self.paste(&text);
            cx.emit(TerminalViewEvent::Used);
        }
    }

    /// Copies the selected text to the clipboard. Returns false when nothing is selected.
    pub fn copy_selection(&self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.terminal.selected_text() else { return false };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// The link under the mouse while Command is held.
    pub(crate) fn hovered_link(&self) -> Option<&Link> {
        self.link.as_ref()
    }

    /// Finds the link under the mouse again, after it moved or the screen changed.
    fn refresh_link(&mut self) -> bool {
        let link = self
            .pointer
            .filter(|(_, command)| *command)
            .and_then(|(position, _)| self.cell_inside(position))
            .and_then(|(row, col)| self.terminal.link_at(row, col));
        let changed = link != self.link;
        self.link = link;
        changed
    }

    fn move_pointer(&mut self, pointer: Option<(Point<Pixels>, bool)>, cx: &mut Context<Self>) {
        self.pointer = pointer;
        if self.refresh_link() {
            cx.notify();
        }
    }

    fn modifiers_changed(&mut self, event: &ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.pointer.is_some() {
            self.move_pointer(Some((window.mouse_position(), event.modifiers.platform)), cx);
        }
    }

    /// The cell under a window position, or `None` when it's outside the grid.
    fn cell_inside(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let geometry = self.geometry.get()?;
        let size = self.terminal.size();
        let (row, col, _) = self.cell_at(position)?;
        (position.x >= geometry.origin.x
            && position.y >= geometry.origin.y
            && row < usize::from(size.rows)
            && col < usize::from(size.cols))
        .then_some((row, col))
    }

    /// The cell under a window position: row, column, and which half of the cell.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "positions inside a small grid")]
    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize, Side)> {
        let geometry = self.geometry.get()?;
        let x = f32::from(position.x - geometry.origin.x).max(0.0);
        let y = f32::from(position.y - geometry.origin.y).max(0.0);
        let cell_width = f32::from(geometry.cell_width);
        let side = if x % cell_width > cell_width / 2.0 { Side::Right } else { Side::Left };
        Some(((y / f32::from(geometry.line_height)) as usize, (x / cell_width) as usize, side))
    }

    /// Whether mouse events go to the program rather than make a selection. Holding Option (or
    /// Shift) selects anyway, as in other terminals.
    fn program_wants_mouse(&self, modifiers: gpui_kit::Modifiers) -> bool {
        let mode = self.terminal.mode();
        mode.intersects(TermMode::MOUSE_MODE)
            && mode.contains(TermMode::SGR_MOUSE)
            && !modifiers.alt
            && !modifiers.shift
    }

    fn report_mouse(&mut self, row: usize, col: usize, pressed: bool) {
        let kind = if pressed { 'M' } else { 'm' };
        self.terminal.write(format!("\x1b[<0;{};{}{kind}", col + 1, row + 1).into_bytes());
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.emit(TerminalViewEvent::Used);
        // Command-click opens a link, as in other macOS terminals, even when the program wants the
        // mouse.
        if event.modifiers.platform
            && let Some(link) = self.cell_inside(event.position).and_then(|(row, col)| self.terminal.link_at(row, col))
        {
            cx.open_url(&link.target);
            return;
        }
        let Some((row, col, side)) = self.cell_at(event.position) else { return };
        if self.program_wants_mouse(event.modifiers) {
            self.drag = Drag::Reporting;
            self.reported = Some((row, col));
            self.report_mouse(row, col, true);
        } else {
            self.drag = Drag::Selecting;
            self.terminal.start_selection(row, col, side, event.click_count);
        }
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_pointer(Some((event.position, event.modifiers.platform)), cx);
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        // A program that follows drags (Claude Code selecting text) gets each cell the mouse moves
        // to, as a drag of the left button.
        if self.drag == Drag::Reporting {
            let follows = self.terminal.mode().intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION);
            if let Some((row, col, _)) = self.cell_at(event.position)
                && follows
                && self.reported != Some((row, col))
            {
                self.reported = Some((row, col));
                self.terminal.write(format!("\x1b[<32;{};{}M", col + 1, row + 1).into_bytes());
            }
            return;
        }
        if self.drag != Drag::Selecting {
            return;
        }
        if let Some((row, col, side)) = self.cell_at(event.position) {
            self.terminal.extend_selection(row, col, side);
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.drag == Drag::Selecting && COPY_ON_SELECT.load(Ordering::Relaxed) {
            self.copy_selection(cx);
        }
        if self.drag == Drag::Reporting
            && let Some((row, col, _)) = self.cell_at(event.position)
        {
            self.report_mouse(row, col, false);
        }
        self.drag = Drag::None;
    }

    pub fn kill(&mut self) {
        self.terminal.kill();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let press = KeyPress {
            key: &keystroke.key,
            text: keystroke.key_char.as_deref(),
            control: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
            command: keystroke.modifiers.platform,
            meta: OPTION_AS_META.load(Ordering::Relaxed),
            caps_lock: window.capslock().on,
        };
        if let Some(bytes) = keys::encode(&press, self.terminal.mode().contains(TermMode::APP_CURSOR)) {
            self.terminal.clear_selection();
            self.write(bytes);
            cx.emit(TerminalViewEvent::Used);
            cx.stop_propagation();
        }
    }

    /// Sends a key a binding took on its way here, as `key_down` would have.
    fn send_key(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) {
        let press = KeyPress { key, shift, ..KeyPress::default() };
        if let Some(bytes) = keys::encode(&press, self.terminal.mode().contains(TermMode::APP_CURSOR)) {
            self.terminal.clear_selection();
            self.write(bytes);
        }
        cx.notify();
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / f32::from(window.line_height()),
        } + self.scroll_remainder;
        #[expect(clippy::cast_possible_truncation, reason = "whole lines of a small scroll delta")]
        let whole = lines.trunc() as i32;
        self.scroll_remainder = lines.fract();
        if whole == 0 {
            return;
        }
        let mode = self.terminal.mode();
        if mode.intersects(TermMode::MOUSE_MODE) && mode.contains(TermMode::SGR_MOUSE) {
            // The program wants the wheel itself (Claude's full-screen view does).
            let button = if whole > 0 { 64 } else { 65 };
            let report = format!("\x1b[<{button};1;1M");
            self.terminal.write(report.repeat(whole.unsigned_abs() as usize).into_bytes());
        } else if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            let arrow = if whole > 0 { "\x1b[A" } else { "\x1b[B" };
            self.terminal.write(arrow.repeat(whole.unsigned_abs() as usize).into_bytes());
        } else {
            self.terminal.scroll(whole);
        }
        self.refresh_link();
        cx.notify();
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A change of appearance redraws every terminal; programs that asked hear of it here.
        self.terminal.report_scheme();
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .size_full()
            // Text can be selected here, so the pointer says so; over a link with Command held, it
            // says the link opens.
            .cursor(if self.link.is_some() { CursorStyle::PointingHand } else { CursorStyle::IBeam })
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::key_down))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            // A link stays underlined only while the mouse is over this terminal.
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered {
                    this.move_pointer(None, cx);
                }
            }))
            // Files dropped from Finder or a screenshot tool: their paths, typed in.
            .on_drop(cx.listener(|this, dropped: &gpui_kit::ExternalPaths, window, cx| {
                window.focus(&this.focus, cx);
                this.paste(&keys::dropped_paths(dropped.paths()));
                cx.emit(TerminalViewEvent::Used);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SendTab, _, cx| this.send_key("tab", false, cx)))
            .on_action(cx.listener(|this, _: &SendBackTab, _, cx| this.send_key("tab", true, cx)))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(TerminalElement::new(cx.entity(), focused, self.geometry.clone()))
            // While files are dragged over this terminal it lights up, so it's clear which one
            // they'll be typed into. Over the text, which paints its own background.
            .child(div().absolute().inset_0().drag_over::<gpui_kit::ExternalPaths>(|style, _, _, _| {
                style.bg(crate::element::hsla(crate::palette::selection(), crate::palette::selection_opacity() / 2.))
            }))
    }
}
