//! Teach the repo (⌘⇧P): a correction you gave an agent, kept for every session after it. The pane's
//! recent prompts, corrections marked first; pick one and say it as a lesson, then add it under the
//! instruction file's `## Lessons` (`saturn_core::instructions::add_lesson`), where it shows in
//! Changes to review like any edit, or ask the agent to word it and add it itself. Each line of a
//! good instruction file is a mistake that won't happen again.

use std::path::PathBuf;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use saturn_core::instructions;
use saturn_core::prompts;
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use saturn_design::text;

pub struct TeachView {
    workspace: WeakEntity<Workspace>,
    checkout: PathBuf,
    /// The pane's prompts, newest first, corrections marked.
    prompts: Vec<(String, bool)>,
    lesson: Entity<InputState>,
    /// The files a lesson can go in, and which is picked.
    files: Vec<PathBuf>,
    file: usize,
    said: Option<String>,
}

impl TeachView {
    fn new(
        workspace: WeakEntity<Workspace>,
        checkout: PathBuf,
        asked: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut prompts: Vec<(String, bool)> = asked
            .into_iter()
            .rev()
            .take(40)
            .map(|prompt| {
                let correction = prompts::is_correction(&prompt);
                (prompt, correction)
            })
            .collect();
        // Corrections first, each group newest first.
        prompts.sort_by_key(|(_, correction)| !correction);
        let lesson = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("The lesson, as an instruction: \"Keep loops readable; don't use reduce.\"")
        });
        // The repository's instruction files that exist; else a new AGENTS.md, which Claude Code and
        // Codex both read. The personal CLAUDE.local.md only where Claude reads CLAUDE files anyway:
        // making one beside a lone AGENTS.md would stop Claude Code reading that.
        let mut files: Vec<PathBuf> =
            ["AGENTS.md", "CLAUDE.md"].iter().map(|name| checkout.join(name)).filter(|path| path.is_file()).collect();
        if files.is_empty() {
            files.push(checkout.join("AGENTS.md"));
        }
        let local = checkout.join("CLAUDE.local.md");
        if local.is_file() || checkout.join("CLAUDE.md").is_file() {
            files.push(local);
        }
        Self { workspace, checkout, prompts, lesson, files, file: 0, said: None }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lesson = self.lesson.read(cx).value().to_string();
        let Some(file) = self.files.get(self.file).cloned() else { return };
        match instructions::add_lesson(&file, &lesson) {
            Ok(_) => {
                let _ = self.workspace.update(cx, |workspace, cx| workspace.open_file(&file, window, cx));
                window.close_dialog(cx);
            }
            Err(problem) => {
                self.said = Some(problem);
                cx.notify();
            }
        }
    }

    fn ask_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lesson = self.lesson.read(cx).value().trim().to_owned();
        let Some(file) = self.files.get(self.file) else { return };
        let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let message = format!(
            "Turn this into one short, general instruction for {name}, under its \"{}\" heading (add the heading if it isn't there), so future sessions don't repeat the mistake. Say it as a rule, not about this task alone:\n\n{lesson}\n",
            instructions::LESSONS
        );
        let checkout = self.checkout.clone();
        let _ = self.workspace.update(cx, |workspace, cx| workspace.send_to_agent(&checkout, &message, window, cx));
        window.close_dialog(cx);
    }
}

impl Render for TeachView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list =
            div().id("teach-prompts").flex().flex_col().gap(len(space::SPACE_0)).max_h(px(300.)).overflow_y_scroll();
        if self.prompts.is_empty() {
            list = list.child(
                div()
                    .py(len(space::SPACE_2))
                    .text_scale(text::LABEL)
                    .text_color(theme::color(color::INK_MUTED))
                    .child("No agent session in this pane yet: write the lesson yourself below."),
            );
        }
        for (index, (prompt, correction)) in self.prompts.iter().enumerate() {
            let picked = prompt.clone();
            list = list.child(
                div()
                    .id(("teach-prompt", index))
                    .flex()
                    .items_start()
                    .gap(len(space::SPACE_2))
                    .px(len(space::SPACE_2))
                    .py(len(space::SPACE_1))
                    .rounded(theme::corner(radius::RADIUS_SM))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::fill_hover()))
                    .child(
                        div()
                            .flex_none()
                            .w(px(80.))
                            .text_scale(text::MICRO)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(if *correction { color::AMBER_TEXT } else { color::INK_FAINT }))
                            .child(if *correction { "Correction" } else { "Prompt" }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_scale(text::LABEL)
                            .text_color(theme::color(color::INK))
                            .child(prompt.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let picked = picked.clone();
                        this.lesson.update(cx, |lesson, cx| lesson.set_value(picked, window, cx));
                    })),
            );
        }
        let mut files = chrome::track();
        for (index, file) in self.files.iter().enumerate() {
            files = files.child(
                chrome::track_item(("teach-file", index), self.file == index)
                    .px(len(space::SPACE_3))
                    .text_scale(text::META)
                    .child(file_label(file))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.file = index;
                        cx.notify();
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_3))
            .child(
                div()
                    .text_scale(text::LABEL)
                    .text_color(theme::color(color::INK_MUTED))
                    .child("Pick what you told the agent, then say it as a lesson for every session after this one."),
            )
            .child(list)
            .child(crate::chrome::input(&self.lesson))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(files)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_scale(text::LABEL)
                            .text_color(theme::color(color::INK_MUTED))
                            .children(self.said.clone()),
                    )
                    .child(
                        chrome::button("teach-agent", "Ask the agent to word it", None, false)
                            .on_click(cx.listener(|this, _, window, cx| this.ask_agent(window, cx))),
                    )
                    .child(
                        chrome::button("teach-add", "Add it", None, true)
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            )
    }
}

/// A file a lesson can go in, as the choice reads: new or not, shared or only yours.
fn file_label(file: &std::path::Path) -> String {
    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let new = if file.is_file() { "" } else { " (new)" };
    let whose = if name == "CLAUDE.local.md" { ", only yours" } else { "" };
    format!("{name}{new}{whose}")
}

/// Opens Teach the repo for the pane working in `checkout`, with the prompts typed in its session.
pub fn open(
    workspace: WeakEntity<Workspace>,
    checkout: PathBuf,
    asked: Vec<String>,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let view = cx.new(|cx| TeachView::new(workspace, checkout, asked, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Teach the repo")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(saturn_design::size::DIALOG_LARGE))
            .child(view.clone())
    });
}
