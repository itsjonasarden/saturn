//! The Preview tab of the editor panel (⌘⇧P, "Preview the app"): a pane's running page, drawn by a
//! browser already on the Mac (`saturn_core::pictures`) and drawn again every few seconds while the
//! tab is open, at a desktop's size or a phone's. Saturn can't hold a browser engine, so it's a
//! picture, not a page to click in; "Open in browser" is for that.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Task, Window, div, img, px,
};
use saturn_core::pictures;
use saturn_core::servers::Server;
use saturn_design::{color, space};

use crate::chrome;
use crate::theme::{self, len};

/// How often the page is drawn again while the tab is open.
const EVERY: Duration = Duration::from_secs(8);

pub struct PreviewView {
    servers: Vec<Server>,
    server: usize,
    path: Entity<InputState>,
    mobile: bool,
    /// The latest picture of the page.
    shot: Option<PathBuf>,
    drawing: bool,
    said: Option<String>,
    /// Counts pictures, so each has a file of its own and is shown afresh.
    taken: u64,
    _again: Task<()>,
    _path_events: Subscription,
}

impl PreviewView {
    pub fn new(servers: Vec<Server>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = cx.new(|cx| InputState::new(window, cx).default_value("/"));
        let path_events = cx.subscribe(&path, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.draw(cx);
            }
        });
        let again = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, Self::draw).is_err() {
                    break;
                }
                cx.background_executor().timer(EVERY).await;
            }
        });
        Self {
            servers,
            server: 0,
            path,
            mobile: false,
            shot: None,
            drawing: false,
            said: None,
            taken: 0,
            _again: again,
            _path_events: path_events,
        }
    }

    /// Shows another pane's servers, keeping the page and size.
    pub fn set_servers(&mut self, servers: Vec<Server>, cx: &mut Context<Self>) {
        self.servers = servers;
        self.server = 0;
        self.draw(cx);
    }

    fn url(&self, cx: &gpui_kit::App) -> Option<String> {
        let server = self.servers.get(self.server)?;
        let path = self.path.read(cx).value().trim().to_owned();
        let path = if path.starts_with('/') { path } else { format!("/{path}") };
        Some(format!("http://localhost:{}{path}", server.port))
    }

    /// Draws the page again, off the main thread.
    fn draw(&mut self, cx: &mut Context<Self>) {
        if self.drawing {
            return;
        }
        let Some(url) = self.url(cx) else { return };
        let Some(browser) = pictures::browser() else {
            self.said = Some("The preview needs Chrome, Chromium, Brave, Edge or Arc installed.".to_owned());
            cx.notify();
            return;
        };
        let Some(folder) = crate::settings::data_dir().map(|dir| dir.join("preview")) else { return };
        self.taken += 1;
        let out = folder.join(format!("page-{}-{}.png", std::process::id(), self.taken));
        let size = if self.mobile { (390, 844) } else { (1280, 800) };
        let before = self.shot.clone();
        self.drawing = true;
        cx.spawn(async move |this, cx| {
            let drawn = cx
                .background_spawn({
                    let out = out.clone();
                    async move { pictures::capture(&browser, &url, size, &out) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.drawing = false;
                match drawn {
                    Ok(()) => {
                        this.shot = Some(out);
                        this.said = None;
                        // The picture it replaces isn't needed any more.
                        if let Some(before) = before {
                            let _ = std::fs::remove_file(before);
                        }
                    }
                    Err(problem) => this.said = Some(problem),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for PreviewView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut servers = chrome::track();
        for (index, server) in self.servers.iter().enumerate() {
            servers = servers.child(
                chrome::track_item(("preview-server", index), self.server == index)
                    .px(len(space::SPACE_3))
                    .text_size(px(12.))
                    .child(format!("{} :{}", server.name, server.port))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.server = index;
                        this.draw(cx);
                    })),
            );
        }
        let size = |id: &'static str, label: &'static str, mobile: bool, cx: &mut Context<Self>| {
            chrome::track_item(id, self.mobile == mobile)
                .px(len(space::SPACE_3))
                .text_size(px(12.))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mobile = mobile;
                    this.draw(cx);
                }))
        };
        let url = self.url(cx);
        let body = if self.servers.is_empty() {
            div()
                .text_size(px(13.))
                .text_color(theme::color(color::INK_MUTED))
                .child("The focused pane runs no server. Start the app (⌘⇧P), then open the preview again.")
                .into_any_element()
        } else if let Some(shot) = &self.shot {
            img(shot.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element()
        } else {
            div()
                .text_size(px(13.))
                .text_color(theme::color(color::INK_MUTED))
                .child(self.said.clone().unwrap_or_else(|| "Drawing the page…".to_owned()))
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap(len(space::SPACE_2))
            .p(len(space::SPACE_2))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .child(servers)
                    .child(div().flex_1().min_w_0().child(crate::chrome::input(&self.path)))
                    .child(chrome::track().child(size("preview-desktop", "Desktop", false, cx)).child(size(
                        "preview-mobile",
                        "Phone",
                        true,
                        cx,
                    )))
                    .child(
                        chrome::icon_button("preview-refresh", "refresh-cw", true)
                            .tooltip(chrome::tip("Draw it again"))
                            .on_click(cx.listener(|this, _, _, cx| this.draw(cx))),
                    )
                    .child(
                        chrome::icon_button("preview-open", "external-link", true)
                            .tooltip(chrome::tip("Open in your browser"))
                            .on_click(move |_, _, _| {
                                if let Some(url) = &url {
                                    let _ = std::process::Command::new("open").arg(url).spawn();
                                }
                            }),
                    ),
            )
            .child(div().id("preview-page").flex_1().min_h_0().flex().items_center().justify_center().child(body))
    }
}
