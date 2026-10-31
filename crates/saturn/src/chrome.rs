//! The window's chrome in the design system, layered as macOS lays out its windows: the chrome (title
//! bar, sidebar, the gaps) is clear over the window's sidebar material, and content sits on opaque
//! cards. A card (radius 10) holds a strip of controls and, under a hairline, its content; controls
//! that belong together sit in a track. Strokes and fills are relative (`theme::overlay`).

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Action, AnyElement, AnyView, App, Bounds, ColorSpace, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, PathBuilder, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    canvas, div, fill, linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px, size, svg,
};
use saturn_design::{color, radius, space};

use crate::theme::{self, len};

/// A tooltip saying what a control does.
pub fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| Tooltip::new(text).build(window, cx)
}

/// A tooltip saying what a control does, with the shortcut that does the same (as the keymap
/// binds it, so it's never out of date).
pub fn tip_for<A: Action + Clone>(
    text: &'static str,
    action: A,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| Tooltip::new(text).action(&action, None).build(window, cx)
}

/// Height of every control: tabs, buttons, track items, segments. Dense, as development tools are.
pub const CONTROL_HEIGHT: f32 = 28.0;

/// A Lucide icon from gpui-kit's bundled set, e.g. `panel-left`. SVGs don't inherit the text
/// colour, so it is given here.
pub fn icon(name: &str, size: f32, color: gpui_kit::Hsla) -> impl IntoElement {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

/// Whether the Cyberdeck theme is on: square panels with cut corners instead of glass and cards.
pub fn cyberdeck() -> bool {
    saturn_design::theme() == saturn_design::Theme::Cyberdeck
}

/// How far along each edge Cyberdeck cuts a panel's corner, and a selection's.
const PANEL_CUT: f32 = 12.;
const SELECTION_CUT: f32 = 8.;

/// The corner Cyberdeck cuts: panels lose the top right, selections and buttons the bottom right.
/// A row (a setting) has its bottom left cut and a raised notch along its top edge.
#[derive(Clone, Copy)]
pub enum Cut {
    TopRight,
    BottomRight,
    Row,
}

/// `bounds` as a path with one corner cut `cut` pixels along each edge.
/// `stroke` draws its outline that many pixels wide instead of filling it.
fn cut_path(bounds: Bounds<Pixels>, cut: f32, corner: Cut, stroke: Option<f32>) -> Option<gpui_kit::Path<Pixels>> {
    let (left, top) = (bounds.origin.x, bounds.origin.y);
    let (right, bottom) = (left + bounds.size.width, top + bounds.size.height);
    let cut = px(cut);
    let points = match corner {
        Cut::TopRight => vec![
            point(left, top),
            point(right - cut, top),
            point(right, top + cut),
            point(right, bottom),
            point(left, bottom),
        ],
        Cut::BottomRight => vec![
            point(left, top),
            point(right, top),
            point(right, bottom - cut),
            point(right - cut, bottom),
            point(left, bottom),
        ],
        Cut::Row => {
            // The top edge sits 3px down, but for a raised stretch from 13px in to two thirds across.
            let step = px(3.);
            let notch_end = left + bounds.size.width * 0.68;
            vec![
                point(left, top + step),
                point(left + px(10.), top + step),
                point(left + px(13.), top),
                point(notch_end, top),
                point(notch_end + step, top + step),
                point(right, top + step),
                point(right, bottom),
                point(left + cut, bottom),
                point(left, bottom - cut),
            ]
        }
    };
    let mut path = stroke.map_or_else(PathBuilder::fill, |width| PathBuilder::stroke(px(width)));
    path.move_to(points[0]);
    for to in &points[1..] {
        path.line_to(*to);
    }
    path.close();
    path.build().ok()
}

/// A Cyberdeck surface, painted under an element's children: `fill` with one corner cut, inside a
/// `width` edge of `edge`. Drawn, not styled, since GPUI's boxes only round their corners.
pub fn cut_surface(edge: gpui_kit::Hsla, fill: gpui_kit::Hsla, width: f32, cut: f32, corner: Cut) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            if let Some(path) = cut_path(bounds, cut, corner, None) {
                window.paint_path(path, fill);
            }
            // The edge is stroked along a path inset by half its width, so it stays inside the
            // bounds, and the fill under it can be translucent.
            let half = width / 2.;
            let inner = Bounds {
                origin: point(bounds.origin.x + px(half), bounds.origin.y + px(half)),
                size: size(bounds.size.width - px(width), bounds.size.height - px(width)),
            };
            if width > 0.
                && let Some(path) = cut_path(inner, (cut - half * 0.41).max(0.), corner, Some(width))
            {
                window.paint_path(path, edge);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The fill of the one primary action: vivid blue sinking toward deep blue, blended in `OKLab`.
fn accent_fill() -> gpui_kit::Background {
    linear_gradient(
        135.,
        linear_color_stop(theme::color(color::BLUE_VIVID), 0.),
        linear_color_stop(theme::color(color::BLUE_VIVID).blend(theme::color(color::BLUE_DEEP).opacity(0.45)), 1.),
    )
    .color_space(ColorSpace::Oklab)
}

/// A card: an opaque content surface (a terminal pane, the editor) over the glass. Its edge is
/// relative: in light mode a faint dark ring and a soft shadow lift it off the glass; in dark mode
/// a faint light ring and a highlight along its top edge.
pub fn panel() -> gpui_kit::Div {
    lit_panel(None)
}

/// A panel with a status ring (`glow`): focused, your turn, or needs you. In Saturn's theme it
/// glows around the card; in Cyberdeck the panel's own edge takes the ring's colour, cut corner and
/// all.
pub fn lit_panel(glow: Option<saturn_design::Shadows>) -> gpui_kit::Div {
    let dark = saturn_design::appearance() == saturn_design::Appearance::Dark;
    if cyberdeck() {
        let ring = glow.and_then(|glow| glow.get().first()).map(|layer| theme::layers(&[*layer])[0].color);
        let edge = ring.unwrap_or_else(|| theme::overlay(if dark { 0.16 } else { 0.18 }));
        return div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .px(len(space::SPACE_1))
            .pt(len(space::SPACE_1))
            .child(cut_surface(edge, theme::color(color::SURFACE), 1., PANEL_CUT, Cut::TopRight))
            .overflow_hidden();
    }
    let edge = if dark {
        vec![shadow(0., 1., 0., 0., gpui_kit::hsla(0., 0., 1., 0.05), true)]
    } else {
        vec![shadow(0., 1., 3., 0., gpui_kit::hsla(0., 0., 0., 0.1), false)]
    };
    let panel = div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .min_h_0()
        .px(len(space::SPACE_1))
        .pt(len(space::SPACE_1))
        .rounded(theme::corner(radius::RADIUS_MD))
        // Mostly opaque: the glass shows through, never enough to fight the text.
        .bg(theme::color(color::SURFACE).opacity(if dark { 0.8 } else { 0.83 }))
        .border_1()
        .border_color(theme::overlay(if dark { 0.07 } else { 0.08 }))
        .shadow(edge)
        .overflow_hidden();
    match glow {
        Some(glow) => panel.border_color(gpui_kit::transparent_black()).shadow(theme::shadows(glow)),
        None => panel,
    }
}

/// A card with no strip: just content, edge to edge (a list, an empty state).
pub fn card() -> gpui_kit::Div {
    panel().p_0()
}

/// A card's content, under its strip: edge to edge, below one hairline.
pub fn well() -> gpui_kit::Div {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .mt(len(space::SPACE_1))
        .mx(len(-space::SPACE_1))
        .border_t_1()
        .border_color(theme::separator())
        .overflow_hidden()
}

fn shadow(x: f32, y: f32, blur: f32, spread: f32, color: gpui_kit::Hsla, inset: bool) -> gpui_kit::BoxShadow {
    gpui_kit::BoxShadow { color, offset: point(px(x), px(y)), blur_radius: px(blur), spread_radius: px(spread), inset }
}

/// A track: controls that belong together, on a faint relative fill, no border.
pub fn track() -> gpui_kit::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .p(len(space::SPACE_1))
        .rounded(theme::corner(radius::RADIUS_MD))
        .bg(theme::overlay(if saturn_design::appearance() == saturn_design::Appearance::Dark { 0.07 } else { 0.05 }))
}

/// What sits right on the window's glass (the title bar's tracks and buttons): on the panes' own
/// surface, so it reads over any desktop. The edge is a shadow, not a border, so it adds no size.
pub fn on_glass(content: gpui_kit::Div) -> gpui_kit::Div {
    let dark = saturn_design::appearance() == saturn_design::Appearance::Dark;
    let ring = shadow(0., 0., 0., 1., theme::overlay(if dark { 0.07 } else { 0.08 }), false);
    let lift = if dark {
        shadow(0., 1., 0., 0., gpui_kit::hsla(0., 0., 1., 0.05), true)
    } else {
        shadow(0., 1., 3., 0., gpui_kit::hsla(0., 0., 0., 0.1), false)
    };
    div()
        .flex()
        .flex_none()
        .rounded(theme::corner(radius::RADIUS_MD))
        // The panes' own surface, solid, so the title bar reads as the same material as the cards
        // and not lighter: the track inside adds no tint of its own on top.
        .bg(theme::color(color::SURFACE))
        .shadow(vec![ring, lift])
        .child(content.bg(gpui_kit::transparent_black()))
}

/// The selected look of every tab and track item: raised, in light mode a white pill with a
/// faint edge and a small shadow, in dark mode a lighter fill. Selection is tone, not colour.
/// Every item keeps its 1px edge (clear unless raised), so selecting one never changes its size
/// and nothing beside it moves.
pub fn raised<E: gpui_kit::Styled>(item: E, pressed: bool) -> E {
    let dark = saturn_design::appearance() == saturn_design::Appearance::Dark;
    let item = item.border_1();
    if cyberdeck() {
        // Cyberdeck's selection is drawn by `selection`, under the item's children.
        item.border_color(gpui_kit::transparent_black())
    } else if pressed && dark {
        item.bg(theme::overlay(0.14)).border_color(gpui_kit::transparent_black())
    } else if pressed {
        item.bg(gpui_kit::white()).border_color(theme::overlay(0.06)).shadow(vec![shadow(
            0.,
            1.,
            2.,
            0.,
            gpui_kit::hsla(0., 0., 0., 0.14),
            false,
        )])
    } else {
        item.border_color(gpui_kit::transparent_black())
    }
}

/// A control floating over content (a diagram's search and zoom): a solid card surface with a
/// faint edge and a soft shadow, so it reads over whatever is under it.
pub fn floating(element: gpui_kit::Div) -> gpui_kit::Div {
    let dark = saturn_design::appearance() == saturn_design::Appearance::Dark;
    if cyberdeck() {
        // Square, its bottom right corner cut, with a stronger edge than a panel's since it floats.
        let edge = theme::overlay(if dark { 0.3 } else { 0.32 });
        return element.relative().child(cut_surface(edge, theme::color(color::SURFACE), 1., 10., Cut::BottomRight));
    }
    element
        .bg(theme::color(color::SURFACE))
        .border_1()
        .border_color(theme::overlay(if dark { 0.1 } else { 0.08 }))
        .shadow(vec![shadow(0., 2., 8., 0., gpui_kit::hsla(0., 0., 0., if dark { 0.4 } else { 0.1 }), false)])
}

/// An item in a track: raised when pressed (see `raised`), else quiet until hovered.
pub fn track_item(id: impl Into<ElementId>, pressed: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(len(space::SPACE_2))
        .h(len(CONTROL_HEIGHT))
        .min_w(len(CONTROL_HEIGHT))
        .rounded(theme::corner(radius::RADIUS_SM))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .map(|item| raised(item, pressed))
        .when(pressed && cyberdeck(), |item| item.relative().child(selection()))
        .map(|item| {
            if cyberdeck() && !pressed {
                item.text_color(theme::color(color::INK_MUTED)).hover(|s| s.text_color(theme::color(color::INK)))
            } else if pressed {
                item.text_color(theme::color(color::INK))
            } else {
                item.text_color(theme::color(color::INK_MUTED)).hover(|s| s.bg(theme::fill_hover()))
            }
        })
}

/// Cyberdeck's selection, under a selected item's children: an outline in ink round a faint fill,
/// its bottom right corner cut, as a list marks the line you're on.
pub fn selection() -> impl IntoElement {
    cut_surface(theme::color(color::INK), theme::overlay(0.07), 1., SELECTION_CUT, Cut::BottomRight)
}

/// Cyberdeck's hover, under a clickable line's children: a quieter `selection`, shown only while
/// the pointer is over the hover group `group`.
pub fn hover_outline(group: &'static str) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .invisible()
        .group_hover(group, gpui_kit::Styled::visible)
        .child(cut_surface(theme::overlay(0.4), theme::overlay(0.05), 1., SELECTION_CUT, Cut::BottomRight))
}

/// A tab: one of a few views of the same place (Files and Changes, open files, repos). In Saturn's
/// theme a track item; in Cyberdeck words over a bar, the bar lit under the tab you're on.
pub fn tab(id: impl Into<ElementId>, selected: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    if !cyberdeck() {
        return track_item(id, selected);
    }
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(len(space::SPACE_2))
        .h(len(CONTROL_HEIGHT))
        .min_w(len(CONTROL_HEIGHT))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(theme::color(if selected { color::INK } else { color::INK_MUTED }))
        .when(!selected, |tab| tab.hover(|s| s.text_color(theme::color(color::INK))))
        .child(div().absolute().bottom_0().left_0().right_0().h(px(2.)).bg(if selected {
            theme::color(color::INK)
        } else {
            theme::overlay(0.16)
        }))
}

/// A row of tabs: a track in Saturn's theme, bare in Cyberdeck, where each tab carries its own bar.
pub fn tab_row() -> gpui_kit::Div {
    if cyberdeck() { div().flex().flex_none().items_center().gap(px(2.)) } else { track() }
}

/// A text field as tall as every control, so it lines up with the button beside it.
pub fn input(state: &gpui_kit::Entity<gpui_kit::component::input::InputState>) -> gpui_kit::component::input::Input {
    gpui_kit::component::input::Input::new(state).h(len(CONTROL_HEIGHT))
}

/// A choice of a few values. In Saturn's theme a track with the chosen one raised; in Cyberdeck a
/// selector: the value between two arrows that step through the others, over a dash for
/// each one, the chosen dash lit. Clicking the value steps on, round to the first.
pub fn selector(
    id: &'static str,
    labels: &[SharedString],
    current: usize,
    pick: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let pick = std::rc::Rc::new(pick);
    if !cyberdeck() {
        return track()
            .children(labels.iter().enumerate().map(|(index, label)| {
                let pick = pick.clone();
                track_item((id, index), index == current)
                    .px(len(space::SPACE_3))
                    .child(label.clone())
                    .on_click(move |_, window, cx| pick(index, window, cx))
            }))
            .into_any_element();
    }
    let last = labels.len().saturating_sub(1);
    let step = |slot: usize, left: bool, to: Option<usize>| {
        let pick = pick.clone();
        div()
            .id((id, slot))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(len(CONTROL_HEIGHT))
            .h_full()
            .when(to.is_some(), gpui_kit::Styled::cursor_pointer)
            .child(arrow(left, to.is_some()))
            .when_some(to, |arrow, to| arrow.on_click(move |_, window, cx| pick(to, window, cx)))
    };
    // A dash for each value, while there are few enough to count at a glance.
    let dashes = (labels.len() <= 8).then(|| {
        div().absolute().bottom(px(3.)).flex().gap(px(3.)).children((0..labels.len()).map(|index| {
            div().w(px(10.)).h(px(2.)).bg(if index == current { theme::color(color::INK) } else { theme::overlay(0.2) })
        }))
    });
    let next = pick.clone();
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .min_w(px(168.))
        .h(len(CONTROL_HEIGHT))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK))
        .child(cut_surface(theme::overlay(0.3), theme::overlay(0.03), 1., SELECTION_CUT, Cut::BottomRight))
        .child(step(labels.len(), true, current.checked_sub(1)))
        .child(
            div()
                .id((id, labels.len() + 2))
                .relative()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(labels.get(current).cloned().unwrap_or_default())
                .children(dashes)
                .on_click(move |_, window, cx| next(if current >= last { 0 } else { current + 1 }, window, cx)),
        )
        .child(step(labels.len() + 1, false, (current < last).then_some(current + 1)))
        .into_any_element()
}

/// The selector's arrow: an outlined triangle pointing left or right, faint where it can't step.
fn arrow(left: bool, enabled: bool) -> impl IntoElement {
    let ink = if enabled { theme::color(color::INK) } else { theme::overlay(0.2) };
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            let (x, y) = (bounds.origin.x, bounds.origin.y);
            let (width, height) = (bounds.size.width, bounds.size.height);
            let (tip, back) = if left { (x, x + width) } else { (x + width, x) };
            let mut path = PathBuilder::stroke(px(1.2));
            path.move_to(point(back, y));
            path.line_to(point(tip, y + height / 2.));
            path.line_to(point(back, y + height));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, ink);
            }
        },
    )
    .w(px(9.))
    .h(px(11.))
}

/// An on/off switch. In Cyberdeck it is the word, then a cut box with a block that
/// sits right and lit when on, left and dim when off.
pub fn switch(id: &'static str, on: bool, toggle: impl Fn(&bool, &mut Window, &mut App) + 'static) -> AnyElement {
    if !cyberdeck() {
        return gpui_kit::component::switch::Switch::new(id).checked(on).on_click(toggle).into_any_element();
    }
    let block = div()
        .absolute()
        .top(px(3.))
        .bottom(px(3.))
        .w(px(22.))
        .map(|block| if on { block.right(px(3.)) } else { block.left(px(3.)) })
        .child(cut_surface(
            gpui_kit::transparent_black(),
            if on { theme::color(color::INK) } else { theme::overlay(0.22) },
            0.,
            5.,
            Cut::BottomRight,
        ));
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(len(space::SPACE_3))
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(if on { color::INK } else { color::INK_MUTED }))
        .child(if on { "On" } else { "Off" })
        .child(
            div()
                .relative()
                .w(px(52.))
                .h(px(22.))
                .child(cut_surface(theme::overlay(0.3), theme::color(color::SURFACE), 1., 6., Cut::BottomRight))
                .child(block),
        )
        .on_click(move |_, window, cx| toggle(&!on, window, cx))
        .into_any_element()
}

/// A thin divider between groups in a track.
pub fn track_separator() -> impl IntoElement {
    div().flex_none().w(px(1.)).h(px(16.)).mx(len(space::SPACE_1)).bg(theme::overlay(0.14))
}

/// A square icon button, as tall as every control. Quiet ones have no fill until hovered.
pub fn icon_button(id: impl Into<ElementId>, name: &str, quiet: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(len(CONTROL_HEIGHT))
        .rounded(theme::corner(radius::RADIUS_SM))
        .cursor_pointer()
        .map(|button| {
            if quiet {
                button.hover(|s| s.bg(theme::fill_hover()))
            } else {
                button.bg(theme::fill_control()).hover(|s| s.bg(theme::fill_pressed()))
            }
        })
        .child(icon(name, 14., theme::color(if quiet { color::INK_MUTED } else { color::INK })))
}

/// A text button with an optional shortcut. `primary` is the one action a view is for: the
/// accent fill with a soft blue bloom beneath it.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    shortcut: Option<&'static str>,
    primary: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let label = label.into();
    let hint = if primary { primary_ink().opacity(0.7) } else { theme::color(color::INK_FAINT) };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(len(space::SPACE_3))
        .h(len(CONTROL_HEIGHT))
        .px(len(space::SPACE_4))
        .rounded(theme::corner(radius::RADIUS_SM))
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .map(|button| {
            if primary {
                primary_surface(button)
            } else if cyberdeck() {
                // Cyberdeck's button: an outline with its corner cut, lit under the pointer.
                button
                    .relative()
                    .group("cyberdeck-button")
                    .text_color(theme::color(color::INK))
                    .child(cut_surface(theme::overlay(0.3), theme::overlay(0.03), 1., SELECTION_CUT, Cut::BottomRight))
                    .child(hover_outline("cyberdeck-button"))
            } else {
                button
                    .text_color(theme::color(color::INK))
                    .bg(theme::fill_control())
                    .hover(|s| s.bg(theme::fill_pressed()))
            }
        })
        .child(label)
        .children(shortcut.map(|keys| div().font_weight(FontWeight::NORMAL).text_color(hint).child(keys)))
}

/// The surface of the one primary action: the accent fill with a soft blue bloom beneath it. A
/// split button lays its parts on one surface, so the gradient and the bloom run across both.
pub fn primary_surface<E: gpui_kit::Styled + gpui_kit::ParentElement>(element: E) -> E {
    if cyberdeck() {
        // Cyberdeck's button, in blue: a blue outline round a faint blue fill, its corner cut.
        return element.relative().text_color(theme::color(color::INK)).child(cut_surface(
            theme::color(color::BLUE),
            theme::color(color::BLUE_VIVID).opacity(0.16),
            1.,
            SELECTION_CUT,
            Cut::BottomRight,
        ));
    }
    element.text_color(theme::color(color::INK_ON_ACCENT)).bg(accent_fill()).shadow(vec![gpui_kit::BoxShadow {
        color: theme::color(color::BLUE_VIVID).opacity(0.35),
        offset: point(px(0.), px(6.)),
        blur_radius: px(20.),
        spread_radius: px(0.),
        inset: false,
    }])
}

/// The ink on the primary action: white on Saturn's blue fill, plain ink on Cyberdeck's outline.
pub fn primary_ink() -> gpui_kit::Hsla {
    theme::color(if cyberdeck() { color::INK } else { color::INK_ON_ACCENT })
}

/// A headline in two weights, the point in semibold: "No terminals **yet**".
pub fn headline(light: &str, bold: &str, size: f32) -> impl IntoElement {
    div()
        .flex()
        .gap(px(size * 0.25))
        .text_size(px(size))
        .line_height(px(size * 1.1))
        .text_color(theme::color(color::INK))
        .child(div().font_weight(FontWeight::LIGHT).child(light.to_owned()))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(bold.to_owned()))
}

/// A dialog's own surface: solid, since the theme's background is clear for the glass.
pub fn dialog_surface() -> gpui_kit::Hsla {
    theme::color(color::SURFACE_RAISED)
}

/// The dot grid: faint dots on an 18px pitch, only inside empty areas.
pub fn dot_grid() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            const PITCH: f32 = 18.;
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let mut y = PITCH / 2.;
            while y < height {
                let mut x = PITCH / 2.;
                while x < width {
                    // Strongest in the middle, fading out toward the edges, so the field reads as
                    // light falling on the card rather than a pattern stamped on it.
                    let across = ((x / width - 0.5) * 2.).hypot((y / height - 0.5) * 2.);
                    let strength = (1. - across / std::f32::consts::SQRT_2).clamp(0., 1.).powf(1.5);
                    let dot = theme::overlay(0.22 * strength);
                    let origin = point(bounds.origin.x + px(x - 1.), bounds.origin.y + px(y - 1.));
                    window.paint_quad(fill(Bounds::new(origin, size(px(2.), px(2.))), dot).corner_radii(px(1.)));
                    x += PITCH;
                }
                y += PITCH;
            }
        },
    )
    .size_full()
    .absolute()
    .top_0()
    .left_0()
}

/// The sidebar: its header (the folder it shows), the Files/Changes switch, the tab's content, and
/// an optional footer (the focused agent's card), all on a card like the panes': on glass, a
/// busy desktop would show through behind the file names.
pub fn sidebar(
    header: impl IntoElement,
    switch: impl IntoElement,
    content: impl IntoElement,
    footer: Option<AnyElement>,
) -> AnyElement {
    // A card like the panes, so the file names sit on a solid surface whatever the desktop is.
    panel()
        .gap(len(space::SPACE_1))
        .pb(len(space::SPACE_1))
        .child(header)
        .child(switch)
        .child(div().relative().flex_1().min_h_0().overflow_hidden().child(content))
        .children(footer)
        .into_any_element()
}

/// The sidebar's header: the folder it shows, its branch and path below, and an action.
pub fn sidebar_header(name: &str, detail: &str, action: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(len(space::SPACE_3))
        .pl(len(space::SPACE_1))
        // The folder's mark, not a button: nothing happens on a click, so nothing on hover either.
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(len(CONTROL_HEIGHT))
                .rounded(theme::corner(radius::RADIUS_SM))
                .bg(theme::fill_control())
                .child(icon("folder", 14., theme::color(color::INK))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(15.))
                        .line_height(px(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::color(color::INK))
                        .truncate()
                        .child(name.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .line_height(px(14.))
                        .text_color(theme::color(color::INK_FAINT))
                        .truncate()
                        .child(detail.to_owned()),
                ),
        )
        .child(action)
}

/// What the grid shows with no terminal open: the dot grid, a headline in two weights, one
/// sentence and the action.
pub fn empty_grid(new_terminal: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static) -> AnyElement {
    card()
        .child(
            div().relative().flex_1().min_h_0().child(dot_grid()).child(
                div().absolute().size_full().flex().items_center().justify_center().child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(len(space::SPACE_6))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(len(space::SPACE_3))
                                .child(headline("No terminals", "yet", 44.))
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .text_color(theme::color(color::INK_MUTED))
                                        .child("Open a shell in the project folder, then start your agent in it."),
                                ),
                        )
                        .child(button("empty-new", "New terminal", Some("⌘T"), true).on_click(new_terminal)),
                ),
            ),
        )
        .into_any_element()
}

/// How the draggable gaps between panels look: nothing at rest (the gap of ground is the
/// divider), a short blue pill while the pointer is on it or dragging.
pub fn quiet_resize_handle() -> gpui_kit::base::ResizeHandleRenderer {
    std::rc::Rc::new(|handle, _, _| {
        use gpui_kit::base::ResizeHandleState;
        // An empty element, not `None`: `None` brings back the library's own hairline.
        if handle.state() == ResizeHandleState::Idle {
            return Some(div().into_any_element());
        }
        let pill =
            theme::color(color::BLUE).opacity(if handle.state() == ResizeHandleState::Hovered { 0.5 } else { 0.9 });
        // The handle's own area is a one-pixel line; the pill is centred along it and overhangs
        // across it.
        Some(match handle.axis() {
            gpui_kit::Axis::Horizontal => div()
                .flex_none()
                .flex()
                .items_center()
                .w(px(1.))
                .h_full()
                .child(div().flex_none().ml(px(-1.)).w(px(3.)).h(px(36.)).rounded_full().bg(pill))
                .into_any_element(),
            gpui_kit::Axis::Vertical => div()
                .flex_none()
                .flex()
                .justify_center()
                .h(px(1.))
                .w_full()
                .child(div().flex_none().mt(px(-1.)).h(px(3.)).w(px(36.)).rounded_full().bg(pill))
                .into_any_element(),
        })
    })
}
