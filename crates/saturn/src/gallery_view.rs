//! The gallery: every component of the design system, in every state, in the theme and appearance
//! Settings has. Saturn's Storybook. Open it with `SATURN_GALLERY=1 pnpm dev`; with
//! `SATURN_SCREENSHOT` it draws offscreen like any spike, so each theme can be pictured side by
//! side and a change to a component is seen everywhere it shows.

use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};
use saturn_design::{color, radius, shadow, space, text};

use crate::chrome;
use crate::theme::{self, TypeScale as _, len};

pub struct GalleryView {
    input: Entity<InputState>,
}

impl GalleryView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::spike::start_window(window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("A field, as tall as a button"));
        Self { input }
    }
}

/// One section of the gallery: its name, then what it shows.
fn section(name: &'static str, content: impl IntoElement) -> impl IntoElement {
    // As tall as what it shows: a card otherwise fills its column.
    chrome::card()
        .h_auto()
        .flex_none()
        .p(len(space::SPACE_4))
        .gap(len(space::SPACE_3))
        .child(div().text_scale(text::META).text_color(theme::color(color::INK_FAINT)).child(name))
        .child(content)
}

fn row() -> gpui_kit::Div {
    div().flex().flex_wrap().items_center().gap(len(space::SPACE_3))
}

fn type_scale() -> impl IntoElement {
    let styles = [
        ("headline", text::HEADLINE),
        ("title", text::TITLE),
        ("subtitle", text::SUBTITLE),
        ("body", text::BODY),
        ("label", text::LABEL),
        ("meta", text::META),
        ("micro", text::MICRO),
        ("badge", text::BADGE),
    ];
    div().flex().flex_col().gap(len(space::SPACE_1)).children(styles.map(|(name, style)| {
        div()
            .flex()
            .items_baseline()
            .gap(len(space::SPACE_3))
            .child(div().w(px(64.)).text_scale(text::MICRO).text_color(theme::color(color::INK_FAINT)).child(name))
            .child(div().text_scale(style).child(format!("Run many agents · {}px", style.size)))
    }))
}

fn spacing() -> impl IntoElement {
    let steps = [
        space::SPACE_0,
        space::SPACE_1,
        space::SPACE_2,
        space::SPACE_3,
        space::SPACE_4,
        space::SPACE_5,
        space::SPACE_6,
        space::SPACE_7,
    ];
    row().children(steps.map(|step| {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(len(space::SPACE_1))
            .child(div().w(len(step)).h(px(16.)).bg(theme::color(color::BLUE)))
            .child(div().text_scale(text::MICRO).text_color(theme::color(color::INK_FAINT)).child(format!("{step}")))
    }))
}

fn colours() -> impl IntoElement {
    let tokens = [
        ("ink", color::INK),
        ("muted", color::INK_MUTED),
        ("faint", color::INK_FAINT),
        ("surface", color::SURFACE),
        ("raised", color::SURFACE_RAISED),
        ("blue", color::BLUE),
        ("orange", color::ORANGE),
        ("green", color::TERM_GREEN),
        ("red", color::RED),
    ];
    row().children(tokens.map(|(name, token)| {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(len(space::SPACE_1))
            .child(
                div()
                    .size(px(32.))
                    .rounded(theme::corner(radius::RADIUS_XS))
                    .border_1()
                    .border_color(theme::overlay(0.12))
                    .bg(theme::color(token)),
            )
            .child(div().text_scale(text::MICRO).text_color(theme::color(color::INK_FAINT)).child(name))
    }))
}

fn buttons() -> impl IntoElement {
    row()
        .child(chrome::button("plain", "Check now", None, false))
        .child(chrome::button("shortcut", "Open folder…", Some("⌘O"), false))
        .child(chrome::button("primary", "Add repo", None, true))
        .child(chrome::icon_button("icon", "settings", false))
        .child(chrome::icon_button("quiet", "ellipsis", true))
        .child(chrome::corner_button("corner", "x", true))
}

fn tracks() -> impl IntoElement {
    row()
        .child(
            chrome::track()
                .child(chrome::track_item("one", true).px(len(space::SPACE_3)).child("Unified"))
                .child(chrome::track_item("two", false).px(len(space::SPACE_3)).child("Split")),
        )
        .child(
            chrome::tab_row()
                .child(chrome::tab("files", true).px(len(space::SPACE_4)).child("Files"))
                .child(chrome::tab("changes", false).px(len(space::SPACE_4)).child("Changes")),
        )
}

fn choices() -> impl IntoElement {
    let few: Vec<SharedString> = ["System", "Light", "Dark"].map(SharedString::from).to_vec();
    row()
        .child(chrome::selector("appearance", &few, 1, |_, _, _| {}))
        .child(chrome::switch("on", true, |_, _, _| {}))
        .child(chrome::switch("off", false, |_, _, _| {}))
}

fn menu(cx: &gpui_kit::App) -> AnyElement {
    let items: Vec<crate::menu::Item> = vec![
        Some(("file-plus", "New file", Box::new(crate::files_view::NewFile))),
        Some(("folder-plus", "New folder", Box::new(crate::files_view::NewFolder))),
        None,
        Some(("git-branch", "History", Box::new(crate::workspace::ShowHistory))),
    ];
    div().w(px(260.)).child(crate::menu::lines("gallery-menu", items, |_, _, _| {}, |_, _| {}, cx)).into_any_element()
}

fn panels() -> impl IntoElement {
    let sample = |name: &'static str, glow: Option<saturn_design::Shadows>| {
        div().w(px(120.)).h(px(64.)).child(
            chrome::lit_panel(glow)
                .p(len(space::SPACE_3))
                .text_scale(text::META)
                .text_color(theme::color(color::INK_MUTED))
                .child(name),
        )
    };
    row()
        .child(sample("Panel", None))
        .child(sample("Focused", Some(shadow::GLOW_FOCUS)))
        .child(sample("Needs you", Some(shadow::GLOW_NEEDS_YOU)))
        .child(sample("Your turn", Some(shadow::GLOW_YOUR_TURN)))
        .child(
            chrome::floating(div())
                .p(len(space::SPACE_3))
                .rounded(theme::corner(radius::RADIUS_MD))
                .text_scale(text::META)
                .child("Floating"),
        )
}

impl Render for GalleryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = chrome::look();
        let column = || div().flex().flex_col().flex_1().min_w_0().gap(len(chrome::pane_gap()));
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(len(chrome::pane_gap()))
            .p(len(chrome::pane_gap()))
            .pt(len(space::SPACE_7))
            .bg(theme::color(color::GROUND))
            .font_family(look.sans)
            .text_color(theme::color(color::INK))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(len(space::SPACE_3))
                    .px(len(space::SPACE_2))
                    .child(div().text_scale(text::TITLE).font_weight(FontWeight::MEDIUM).child(look.name))
                    .child(
                        div().text_scale(text::META).text_color(theme::color(color::INK_MUTED)).child(look.description),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap(len(chrome::pane_gap()))
                    .child(
                        column()
                            .child(section("Type scale", type_scale()))
                            .child(section("Spacing", spacing()))
                            .child(section("Colours", colours())),
                    )
                    .child(
                        column()
                            .child(section("Buttons", buttons()))
                            .child(section("Tracks and tabs", tracks()))
                            .child(section("Choices and switches", choices()))
                            .child(section("Field", div().w(px(320.)).child(chrome::input(&self.input))))
                            .child(section("Panels", panels())),
                    )
                    .child(column().child(section("Menu", menu(cx)))),
            )
    }
}
