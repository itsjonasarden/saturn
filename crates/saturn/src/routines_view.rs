//! Routines (⌘⇧P): agents Saturn starts on a schedule (`saturn_core::routines`), and what each
//! did: every run, started or missed. A routine opens a pane in its folder, named after it, and
//! starts Claude with its prompt; a run whose time passed while Saturn was closed shows as missed,
//! to run by hand. The schedule is looked at every half minute, by one loop for the whole app.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputState, Textarea, TextareaState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use saturn_core::routines::{self, Book, Routine, Run, Schedule};
use saturn_design::{color, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use saturn_design::text;

/// How often the schedule is looked at.
const EVERY: Duration = Duration::from_secs(30);

/// One change to the book at a time: the loop and the list both write it.
static BOOK: Mutex<()> = Mutex::new(());

fn book_file() -> Option<PathBuf> {
    crate::settings::data_dir().map(|dir| dir.join("routines.json"))
}

/// Reads the book. Off the main thread.
fn read_book() -> Book {
    let _held = BOOK.lock().unwrap_or_else(PoisonError::into_inner);
    book_file().map(|file| Book::load(&file)).unwrap_or_default()
}

/// Changes the book on disk. Off the main thread.
fn change_book<T>(change: impl FnOnce(&mut Book) -> T) -> Option<T> {
    let _held = BOOK.lock().unwrap_or_else(PoisonError::into_inner);
    let file = book_file()?;
    let mut book = Book::load(&file);
    let result = change(&mut book);
    let _ = book.save(&file);
    Some(result)
}

/// Starts looking at the schedule, for as long as Saturn runs. Automated runs don't, unless
/// `SATURN_ROUTINES=1` asks.
pub fn start(cx: &mut App) {
    if crate::spike::is_automated() && std::env::var_os("SATURN_ROUTINES").is_none() {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            let due = cx
                .background_spawn(async {
                    change_book(|book| {
                        let due = routines::check_now(book);
                        due.into_iter()
                            .filter_map(|(id, at)| book.routine(&id).cloned().map(|routine| (routine, at)))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
                })
                .await;
            for (routine, at) in due {
                cx.update(|cx| run_routine(routine, at, cx));
            }
            cx.background_executor().timer(EVERY).await;
        }
    })
    .detach();
}

/// Starts a routine for its time `due` in the window in front (else the first one), and writes the
/// run down; with no window open, it's missed.
pub fn run_routine(routine: Routine, due: i64, cx: &mut App) {
    let started =
        crate::workspace::in_any(cx, |workspace, window, cx| workspace.start_routine(&routine, due, window, cx));
    let now = crate::worktrees_view::now();
    cx.background_spawn(async move {
        change_book(|book| {
            if started {
                book.started(&routine.id, due, now);
            } else {
                book.runs.push(Run { routine: routine.id.clone(), due, ran: None, session: None });
            }
        })
    })
    .detach();
}

/// Ties a routine's run to the Claude session it started, so the list can resume it.
pub fn tie_session(routine: String, due: i64, session: String, cx: &mut App) {
    cx.background_spawn(async move { change_book(|book| book.tie_session(&routine, due, &session)) }).detach();
}

/// How a new routine runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Daily,
    Weekdays,
    Every,
}

pub struct RoutinesView {
    workspace: WeakEntity<Workspace>,
    /// Where a new routine runs: the focused pane's checkout.
    folder: PathBuf,
    /// `None` while it's read.
    book: Option<Book>,
    name: Entity<InputState>,
    prompt: Entity<TextareaState>,
    at: Entity<InputState>,
    kind: Kind,
    hours: u32,
    said: Option<String>,
}

impl RoutinesView {
    fn new(workspace: WeakEntity<Workspace>, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name, e.g. Morning dependency check"));
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What Claude is asked each time, e.g. Update outdated packages and run the tests")
        });
        let at = cx.new(|cx| InputState::new(window, cx).default_value("09:00"));
        let view = Self { workspace, folder, book: None, name, prompt, at, kind: Kind::Daily, hours: 4, said: None };
        Self::reload(cx);
        view
    }

    fn reload(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let book = cx.background_spawn(async { read_book() }).await;
            let _ = this.update(cx, |this, cx| {
                this.book = Some(book);
                cx.notify();
            });
        })
        .detach();
    }

    /// Changes the book, then shows it again.
    fn change(change: impl FnOnce(&mut Book) + Send + 'static, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move { change_book(change) }).await;
            let _ = this.update(cx, |_, cx| Self::reload(cx));
        })
        .detach();
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_owned();
        let prompt = self.prompt.read(cx).value().trim().to_owned();
        let at = self.at.read(cx).value().trim().to_owned();
        let schedule = match self.kind {
            Kind::Daily => Schedule::Daily { at, weekdays: false },
            Kind::Weekdays => Schedule::Daily { at, weekdays: true },
            Kind::Every => Schedule::Every { hours: self.hours },
        };
        let problem = if name.is_empty() {
            Some("Give it a name.")
        } else if prompt.is_empty() {
            Some("Say what Claude is asked.")
        } else if !schedule.is_valid() {
            Some("The time reads as hours and minutes, like 09:00.")
        } else {
            None
        };
        if let Some(problem) = problem {
            self.said = Some(problem.to_owned());
            cx.notify();
            return;
        }
        // Unique enough: two routines aren't added in the same nanosecond.
        let made = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let id = format!("routine-{}", made.as_nanos());
        let routine = Routine {
            id,
            name,
            folder: self.folder.clone(),
            prompt,
            schedule,
            on: true,
            agent: crate::agent_choice::current(cx),
        };
        self.said = Some(format!("Added {}.", routine.name));
        self.name.update(cx, |name, cx| name.set_value("", window, cx));
        self.prompt.update(cx, |prompt, cx| prompt.set_value("", window, cx));
        Self::change(move |book| book.routines.push(routine), cx);
    }

    fn routine_row(index: usize, routine: &Routine, now: i64, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let next = routine
            .on
            .then(|| routines::next_time(routine, now))
            .flatten()
            .map_or_else(|| "Paused".to_owned(), |next| format!("Next {}", routines::local_label(next)));
        let mut detail = format!("{} · in {}", routine.schedule.describe(), folder_name(&routine.folder));
        // Which agent, once there's more than one to tell apart.
        if crate::agent_choice::ready(cx).len() > 1 {
            let _ = write!(detail, " · {}", routine.agent.name());
        }
        let (toggle_id, remove_id) = (routine.id.clone(), routine.id.clone());
        let picked = routine.clone();
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .px(len(space::SPACE_2))
            .py(len(space::SPACE_1))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_scale(text::LABEL)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(if routine.on { color::INK } else { color::INK_FAINT }))
                            .child(routine.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_scale(text::META)
                            .text_color(theme::color(color::INK_FAINT))
                            .child(detail),
                    ),
            )
            .child(div().flex_none().text_scale(text::META).text_color(theme::color(color::INK_MUTED)).child(next))
            .child(chrome::button(("routine-run", index), "Run now", None, false).on_click(cx.listener(
                move |_, _, window, cx| {
                    let routine = picked.clone();
                    window.close_dialog(cx);
                    run_routine(routine, crate::worktrees_view::now(), cx);
                },
            )))
            .child(
                chrome::button(("routine-toggle", index), if routine.on { "Pause" } else { "Resume" }, None, false)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let id = toggle_id.clone();
                        Self::change(
                            move |book| {
                                if let Some(routine) = book.routines.iter_mut().find(|routine| routine.id == id) {
                                    routine.on = !routine.on;
                                }
                            },
                            cx,
                        );
                    })),
            )
            .child(
                chrome::icon_button(("routine-remove", index), "trash", true)
                    .tooltip(chrome::tip("Remove the routine; its runs stay listed"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let id = remove_id.clone();
                        Self::change(move |book| book.routines.retain(|routine| routine.id != id), cx);
                    })),
            )
    }

    fn run_row(&self, index: usize, run: &Run, book: &Book) -> impl IntoElement + use<> {
        let routine = book.routine(&run.routine).cloned();
        let name = routine.as_ref().map_or_else(|| "A removed routine".to_owned(), |routine| routine.name.clone());
        let (what, tone) = match run.ran {
            Some(_) => ("Ran", color::INK_MUTED),
            None => ("Missed: Saturn wasn't open", color::INK_FAINT),
        };
        let action = match (&run.ran, &run.session, &routine) {
            (Some(_), Some(session), Some(routine)) if routine.folder.is_dir() => {
                let (workspace, folder, session) = (self.workspace.clone(), routine.folder.clone(), session.clone());
                let agent = routine.agent;
                Some(chrome::button(("run-resume", index), "Resume", None, false).on_click(move |_, window, cx| {
                    let _ = workspace
                        .update(cx, |workspace, cx| workspace.resume_session(&folder, agent, &session, window, cx));
                    window.close_dialog(cx);
                }))
            }
            (None, _, Some(routine)) => {
                let routine = routine.clone();
                Some(chrome::button(("run-again", index), "Run now", None, false).on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    run_routine(routine.clone(), crate::worktrees_view::now(), cx);
                }))
            }
            _ => None,
        };
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .px(len(space::SPACE_2))
            .min_h(len(chrome::CONTROL_HEIGHT))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_scale(text::LABEL)
                    .text_color(theme::color(color::INK))
                    .child(name),
            )
            .child(
                div()
                    .flex_none()
                    .text_scale(text::META)
                    .text_color(theme::color(color::INK_FAINT))
                    .child(routines::local_label(run.due)),
            )
            .child(div().flex_none().w(px(180.)).text_scale(text::META).text_color(theme::color(tone)).child(what))
            .child(div().flex_none().w(px(96.)).flex().justify_end().children(action))
    }

    fn form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        const KINDS: [(Kind, &str); 3] =
            [(Kind::Daily, "Every day"), (Kind::Weekdays, "Weekdays"), (Kind::Every, "Every few hours")];
        const HOURS: [u32; 5] = [1, 2, 4, 6, 12];
        let view = cx.weak_entity();
        let kinds = chrome::selector(
            "routine-kind",
            &KINDS.map(|(_, label)| gpui_kit::SharedString::from(label)),
            KINDS.iter().position(|(kind, _)| *kind == self.kind).unwrap_or(0),
            move |index, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.kind = KINDS[index].0;
                    cx.notify();
                });
            },
        );
        let when = if self.kind == Kind::Every {
            let view = cx.weak_entity();
            chrome::selector(
                "routine-hours",
                &HOURS.map(|count| gpui_kit::SharedString::from(format!("{count} h"))),
                HOURS.iter().position(|count| *count == self.hours).unwrap_or(0),
                move |index, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.hours = HOURS[index];
                        cx.notify();
                    });
                },
            )
        } else {
            div().w(px(90.)).child(crate::chrome::input(&self.at)).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_2))
            .child(crate::chrome::input(&self.name))
            .child(Textarea::new(&self.prompt).h(px(80.)))
            .children(crate::agent_choice::row("routine-agent", "Agent", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(kinds)
                    .child(when)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_scale(text::META)
                            .text_color(theme::color(color::INK_MUTED))
                            .child(
                                self.said.clone().unwrap_or_else(|| format!("Runs in {}", folder_name(&self.folder))),
                            ),
                    )
                    .child(
                        chrome::button("routine-add", "Add routine", None, true)
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            )
    }
}

impl Render for RoutinesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let quiet = |text: &str| {
            div()
                .px(len(space::SPACE_2))
                .py(len(space::SPACE_1))
                .text_scale(text::LABEL)
                .text_color(theme::color(color::INK_MUTED))
                .child(text.to_owned())
        };
        let mut list =
            div().id("routines").flex().flex_col().gap(len(space::SPACE_0)).max_h(px(520.)).overflow_y_scroll();
        let now = crate::worktrees_view::now();
        match self.book.clone() {
            None => list = list.child(quiet("Reading the routines…")),
            Some(book) => {
                list = list.child(heading("Routines"));
                if book.routines.is_empty() {
                    list = list.child(quiet("None yet. Saturn starts each one on its schedule while it's open."));
                }
                for (index, routine) in book.routines.iter().enumerate() {
                    list = list.child(Self::routine_row(index, routine, now, cx));
                }
                list = list.child(heading("Add a routine")).child(div().px(len(space::SPACE_2)).child(self.form(cx)));
                if !book.runs.is_empty() {
                    list = list.child(heading("Runs"));
                    for (index, run) in book.runs.iter().rev().take(30).enumerate() {
                        list = list.child(self.run_row(index, run, &book));
                    }
                }
            }
        }
        list
    }
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

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Opens Routines; new ones run in `folder`.
pub fn open(workspace: WeakEntity<Workspace>, folder: PathBuf, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| RoutinesView::new(workspace, folder, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Routines")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(saturn_design::size::DIALOG_LARGE))
            .child(view.clone())
    });
}
