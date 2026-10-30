//! Pictures of the app (⌘⇧P, "Pictures of the app"): the pages of a pane's running server, drawn at
//! a desktop's size and a phone's by a browser already on the Mac (`saturn_core::pictures`), kept
//! per repository with their branch and time. Two sets side by side show what a branch changed:
//! take one where the default branch runs and one where the agent's does, or one before a turn and
//! one after. The folder opens in Finder, for a pull request or a ticket.

use std::path::PathBuf;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img, px,
};
use saturn_core::pictures::{self, Set};
use saturn_core::servers::Server;
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::{self, len};

pub struct PicturesView {
    /// Where this repository's sets are kept.
    root: PathBuf,
    /// The branch the pane is on, for a new set's name.
    branch: String,
    /// The servers the pane runs, to take pictures of.
    servers: Vec<Server>,
    server: usize,
    pages: Entity<InputState>,
    sets: Vec<Set>,
    /// The two sets shown side by side.
    left: Option<usize>,
    right: Option<usize>,
    taking: bool,
    said: Option<String>,
}

impl PicturesView {
    fn new(root: PathBuf, branch: String, servers: Vec<Server>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pages = cx.new(|cx| InputState::new(window, cx).placeholder("Pages: /, /cart, /orders").default_value("/"));
        let sets = pictures::sets(&root);
        let (left, right) = match sets.len() {
            0 => (None, None),
            1 => (Some(0), None),
            // The newest two: usually the one just taken, against the one before.
            _ => (Some(1), Some(0)),
        };
        let mut view = Self { root, branch, servers, server: 0, pages, sets, left, right, taking: false, said: None };
        if std::env::var_os("SATURN_TAKE_PICTURES").is_some() {
            view.take(cx);
        }
        view
    }

    fn take(&mut self, cx: &mut Context<Self>) {
        let Some(server) = self.servers.get(self.server).cloned() else { return };
        let Some(browser) = pictures::browser() else {
            self.said = Some("Pictures need Chrome, Chromium, Brave, Edge or Arc installed.".to_owned());
            cx.notify();
            return;
        };
        let routes = pictures::routes(&self.pages.read(cx).value());
        let (root, branch) = (self.root.clone(), self.branch.clone());
        self.taking = true;
        self.said = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let failed = cx
                .background_spawn(async move {
                    let now = std::process::Command::new("date")
                        .arg("+%Y-%m-%dT%H.%M.%S")
                        .output()
                        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
                        .unwrap_or_default();
                    let folder = pictures::set_folder(&root, &branch, &now);
                    let mut failed = Vec::new();
                    for route in &routes {
                        for (viewport, width, height) in pictures::VIEWPORTS {
                            let url = format!("http://localhost:{}{route}", server.port);
                            let out = folder.join(pictures::file_name(viewport, route));
                            if let Err(problem) = pictures::capture(&browser, &url, (width, height), &out) {
                                failed.push(problem);
                            }
                        }
                    }
                    failed
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.taking = false;
                this.sets = pictures::sets(&this.root);
                // The new set on the right, the one before it on the left.
                this.right = (!this.sets.is_empty()).then_some(0);
                this.left = (this.sets.len() > 1).then_some(1);
                this.said = failed.first().cloned();
                cx.notify();
            });
        })
        .detach();
    }

    fn render_set_row(&self, index: usize, set: &Set, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let side = |id: &'static str, label: &'static str, chosen: bool, cx: &mut Context<Self>| {
            chrome::track_item((id, index), chosen).px(len(space::SPACE_2)).text_size(px(12.)).child(label).on_click(
                cx.listener(move |this, _, _, cx| {
                    if id == "left" {
                        this.left = Some(index);
                    } else {
                        this.right = Some(index);
                    }
                    cx.notify();
                }),
            )
        };
        let pages = set.files.iter().filter(|file| file.starts_with("desktop-")).count();
        let s = if pages == 1 { "" } else { "s" };
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .py(px(2.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(set.label.clone()))
            .child(
                div().text_size(px(12.)).text_color(theme::color(color::INK_FAINT)).child(format!("{pages} page{s}")),
            )
            .child(chrome::track().child(side("left", "Left", self.left == Some(index), cx)).child(side(
                "right",
                "Right",
                self.right == Some(index),
                cx,
            )))
    }

    /// The chosen sets' pictures, side by side, page by page and size by size.
    fn render_comparison(&self) -> impl IntoElement + use<> {
        let left = self.left.and_then(|index| self.sets.get(index));
        let right = self.right.and_then(|index| self.sets.get(index));
        let mut names: Vec<String> = left.iter().chain(right.iter()).flat_map(|set| set.files.clone()).collect();
        names.sort();
        names.dedup();
        let picture = |set: Option<&Set>, name: &str| {
            let cell = div()
                .flex_1()
                .min_w_0()
                .flex()
                .justify_center()
                .rounded(theme::corner(radius::RADIUS_SM))
                .overflow_hidden()
                .bg(theme::fill_control());
            // Each at its viewport's shape, 320 high, so a phone's tall page isn't cut to a strip.
            #[expect(clippy::cast_precision_loss, reason = "a picture's size on screen")]
            let width = pictures::VIEWPORTS
                .iter()
                .find(|(viewport, _, _)| name.starts_with(viewport))
                .map_or(320., |(_, width, height)| 320. * *width as f32 / *height as f32);
            match set.map(|set| set.folder.join(name)).filter(|path| path.is_file()) {
                Some(path) => cell.child(img(path).w(px(width)).h(px(320.)).object_fit(ObjectFit::Contain)),
                None => cell
                    .h(px(60.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(theme::color(color::INK_FAINT))
                    .child("Not in this set"),
            }
        };
        let mut list =
            div().id("comparison").flex().flex_col().gap(len(space::SPACE_3)).max_h(px(560.)).overflow_y_scroll();
        for name in names {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(len(space::SPACE_1))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::color(color::INK_MUTED))
                            .child(name.trim_end_matches(".png").to_owned()),
                    )
                    .child(
                        div().flex().gap(len(space::SPACE_2)).child(picture(left, &name)).child(picture(right, &name)),
                    ),
            );
        }
        list
    }
}

impl Render for PicturesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut servers = chrome::track();
        for (index, server) in self.servers.iter().enumerate() {
            servers = servers.child(
                chrome::track_item(("picture-server", index), self.server == index)
                    .px(len(space::SPACE_3))
                    .text_size(px(12.))
                    .child(format!("{} :{}", server.name, server.port))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.server = index;
                        cx.notify();
                    })),
            );
        }
        let source = if self.servers.is_empty() {
            div()
                .text_size(px(13.))
                .text_color(theme::color(color::INK_MUTED))
                .child("This pane runs no server to take pictures of. Start the app (⌘⇧P) and open this again.")
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_2))
                .child(servers)
                .child(div().flex_1().min_w_0().child(Input::new(&self.pages)))
                .child(
                    chrome::button("take-pictures", if self.taking { "Taking…" } else { "Take pictures" }, None, true)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.taking {
                                this.take(cx);
                            }
                        })),
                )
                .into_any_element()
        };
        let mut sets = div().flex().flex_col().max_h(px(150.)).id("picture-sets").overflow_y_scroll();
        for (index, set) in self.sets.iter().enumerate() {
            sets = sets.child(self.render_set_row(index, set, cx));
        }
        let root = self.root.clone();
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_3))
            .child(source)
            .children(
                self.said
                    .clone()
                    .map(|said| div().text_size(px(13.)).text_color(theme::color(color::INK_MUTED)).child(said)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child("Sets"))
                    .child(chrome::button("open-pictures", "Open folder", None, false).on_click(move |_, _, _| {
                        let _ = std::fs::create_dir_all(&root);
                        let _ = std::process::Command::new("open").arg(&root).spawn();
                    })),
            )
            .child(sets)
            .child(self.render_comparison())
    }
}

/// Opens the pictures of the app whose server `servers` the pane runs, kept in `root`.
pub fn open(root: PathBuf, branch: String, servers: Vec<Server>, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|cx| PicturesView::new(root, branch, servers, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Pictures of the app")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(1100.))
            .child(view.clone())
    });
}
