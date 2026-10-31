//! The pull request's description (⌘⇧P, "Edit the pull request's description"): as it is on GitHub,
//! or the repository's template when it's empty, in an editor, with the template's sections it
//! leaves empty or out named before it's saved. "Save to GitHub" puts it there; "Ask the agent to
//! write it" hands the job to the pane's agent, with the template to follow.

use std::path::PathBuf;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, px,
};
use saturn_core::pr_body;
use saturn_design::{color, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use saturn_design::text;

pub struct PrBodyView {
    workspace: WeakEntity<Workspace>,
    checkout: PathBuf,
    number: u64,
    text: Entity<TextareaState>,
    /// The repository's template, and where it is.
    template: Option<(String, String)>,
    loaded: bool,
    saving: bool,
    said: Option<String>,
}

impl PrBodyView {
    fn new(
        workspace: WeakEntity<Workspace>,
        checkout: PathBuf,
        number: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = cx.new(|cx| TextareaState::new(window, cx).placeholder("Reading the description…"));
        let template = pr_body::template(&checkout);
        let reading = checkout.clone();
        cx.spawn_in(window, async move |this, cx| {
            let body = cx.background_spawn(async move { crate::github::pr_body(&reading, number) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // An empty description starts from the template, as GitHub's own form does.
                let start = match body.filter(|body| !body.trim().is_empty()) {
                    Some(body) => body,
                    None => this.template.as_ref().map(|(_, text)| text.clone()).unwrap_or_default(),
                };
                this.text.update(cx, |text, cx| text.set_value(start, window, cx));
                this.loaded = true;
                cx.notify();
            });
        })
        .detach();
        Self { workspace, checkout, number, text, template, loaded: false, saving: false, said: None }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let body = self.text.read(cx).value().to_string();
        let (checkout, number) = (self.checkout.clone(), self.number);
        self.saving = true;
        self.said = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let done = cx.background_spawn(async move { crate::github::set_pr_body(&checkout, number, &body) }).await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                this.said = Some(match done {
                    Ok(()) => format!("Saved to #{number} on GitHub."),
                    Err(problem) => problem,
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn ask_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = pr_body::ask_message(self.number, self.template.as_ref());
        let (workspace, checkout) = (self.workspace.clone(), self.checkout.clone());
        let _ = workspace.update(cx, |workspace, cx| {
            workspace.send_to_agent(&checkout, &message, window, cx);
        });
        window.close_dialog(cx);
    }
}

impl Render for PrBodyView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let gaps = self
            .template
            .as_ref()
            .filter(|_| self.loaded)
            .map(|(_, template)| pr_body::gaps(template, &self.text.read(cx).value()));
        let warning = gaps.and_then(|(missing, empty)| {
            let mut parts = Vec::new();
            if !empty.is_empty() {
                parts.push(format!("Empty: {}", empty.join(", ")));
            }
            if !missing.is_empty() {
                parts.push(format!("Left out: {}", missing.join(", ")));
            }
            (!parts.is_empty()).then(|| format!("{} (from the template).", parts.join(". ")))
        });
        let said = self.said.clone().or(warning);
        div().flex().flex_col().gap(len(space::SPACE_3)).child(Textarea::new(&self.text).h(px(380.))).child(
            div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_2))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_scale(text::LABEL)
                        .text_color(theme::color(color::INK_MUTED))
                        .children(said),
                )
                .child(
                    chrome::button("pr-body-agent", "Ask the agent to write it", None, false)
                        .on_click(cx.listener(|this, _, window, cx| this.ask_agent(window, cx))),
                )
                .child(
                    chrome::button("pr-body-save", if self.saving { "Saving…" } else { "Save to GitHub" }, None, true)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.loaded && !this.saving {
                                this.save(cx);
                            }
                        })),
                ),
        )
    }
}

/// Opens the description of pull request `number`, worked on in `checkout`.
pub fn open(
    workspace: WeakEntity<Workspace>,
    checkout: PathBuf,
    number: u64,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let view = cx.new(|cx| PrBodyView::new(workspace, checkout, number, window, cx));
    let title = format!("Description of #{number}");
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(760.))
            .child(view.clone())
    });
}
