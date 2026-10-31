//! The window's root: the welcome screen until a project is picked, then the workspace for it.
//! Started from a terminal inside a folder, Saturn skips the welcome and opens that folder.

use std::path::{Path, PathBuf};

use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::settings;
use crate::spike;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::{OpenProject, Placement, Workspace};
use saturn_design::icon;
use saturn_design::text;

pub struct AppView {
    workspace: Option<Entity<Workspace>>,
    focus: FocusHandle,
}

impl AppView {
    pub fn new(project: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self { workspace: None, focus: cx.focus_handle() };
        spike::start_window(window, cx);
        match project {
            Some(project) => view.open(&project, window, cx),
            None => window.focus(&view.focus, cx),
        }
        view
    }

    /// Opens `project`: its saved grid, else one shell in it.
    fn open(&mut self, project: &Path, window: &mut Window, cx: &mut Context<Self>) {
        settings::remember_project(project);
        let project = project.to_owned();
        self.workspace = Some(cx.new(|cx| {
            let mut workspace = Workspace::new(project, window, cx);
            match spike::panes() {
                Some(panes) => {
                    for index in 0..panes {
                        workspace.open_terminal(spike::placement(index, panes), window, cx);
                    }
                }
                None => {
                    if !workspace.restore(window, cx) {
                        workspace.open_terminal(Placement::New, window, cx);
                    }
                }
            }
            spike::start(&workspace, window, cx);
            workspace
        }));
        cx.notify();
    }

    fn pick_folder(window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(project) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, window, cx| this.open(&project, window, cx));
        })
        .detach();
    }

    fn render_recent(recent: &[PathBuf], cx: &mut Context<Self>) -> impl IntoElement {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        div().flex().flex_col().p(len(space::SPACE_1)).children(recent.iter().enumerate().map(|(index, project)| {
            let name =
                project.file_name().map_or_else(|| project.display().to_string(), |n| n.to_string_lossy().into_owned());
            let folder = match home.as_deref().and_then(|home| project.parent()?.strip_prefix(home).ok()) {
                Some(rest) => format!("~/{}", rest.display()),
                None => project.parent().map_or_else(String::new, |parent| parent.display().to_string()),
            };
            let project = project.clone();
            div()
                .id(("recent", index))
                .flex()
                .items_center()
                .gap(len(space::SPACE_3))
                .px(len(space::SPACE_4))
                .py(len(space::SPACE_3))
                .rounded(theme::corner(radius::RADIUS_SM))
                .cursor_pointer()
                .hover(|s| s.bg(theme::fill_hover()))
                .child(chrome::icon("folder", icon::LG, theme::color(color::INK_MUTED)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_scale(text::SUBTITLE)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::color(color::INK))
                                .truncate()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_scale(text::META)
                                .text_color(theme::color(color::INK_FAINT))
                                .truncate()
                                .child(folder),
                        ),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open(&project, window, cx)))
        }))
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recent = settings::load().recent();
        let open = chrome::button("open-folder", "Open folder…", Some("⌘O"), true)
            .on_click(cx.listener(|_, _, window, cx| Self::pick_folder(window, cx)));
        div()
            .id("welcome")
            .size_full()
            .flex()
            .flex_col()
            .font_family(saturn_design::look().sans)
            .track_focus(&self.focus)
            .key_context("Welcome")
            .on_action(cx.listener(|_, _: &OpenProject, window, cx| Self::pick_folder(window, cx)))
            // The title bar's empty space, which zooms the window on a double-click as everywhere.
            .child(div().id("welcome-titlebar").flex_none().h(px(48.)).on_click(|event: &ClickEvent, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            }))
            .child(
                // A little above centre, where the eye takes the middle to be.
                div().flex_1().flex().items_center().justify_center().pb(len(2. * space::SPACE_6)).child(
                    div()
                        .w(px(saturn_design::size::DIALOG_SMALL))
                        .flex()
                        .flex_col()
                        .gap(len(space::SPACE_6))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(len(space::SPACE_2))
                                .child(chrome::headline("Open a", "project", text::HEADLINE.size))
                                .child(div().text_scale(text::BODY).text_color(theme::color(color::INK_MUTED)).child(
                                    if recent.is_empty() {
                                        "Pick the folder you want your agents to work in."
                                    } else {
                                        "Pick up where you left off, or open another folder."
                                    },
                                )),
                        )
                        .children((!recent.is_empty()).then(|| chrome::card().child(Self::render_recent(&recent, cx))))
                        .child(div().flex().child(open)),
                ),
            )
    }
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.workspace {
            Some(workspace) => workspace.clone().into_any_element(),
            None => self.render_welcome(cx).into_any_element(),
        };
        // Saturn's own background, not the desktop: a deep blue in dark mode, a soft periwinkle in
        // light, the same over any wallpaper. The cards stand on it.
        let backdrop = gpui_kit::linear_gradient(
            155.,
            gpui_kit::linear_color_stop(theme::color(color::BACKDROP), 0.),
            gpui_kit::linear_color_stop(theme::color(color::BACKDROP_DEEP), 1.),
        )
        .color_space(gpui_kit::ColorSpace::Oklab);
        div().relative().size_full().bg(backdrop).child(content)
    }
}
