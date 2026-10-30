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
#[derive(Clone, Copy)]
enum Cut {
    TopRight,
    BottomRight,
}

/// `bounds` as a path with one corner cut `cut` pixels along each edge.
fn cut_path(bounds: Bounds<Pixels>, cut: f32, corner: Cut) -> Option<gpui_kit::Path<Pixels>> {
    let (left, top) = (bounds.origin.x, bounds.origin.y);
    let (right, bottom) = (left + bounds.size.width, top + bounds.size.height);
    let cut = px(cut);
    let points = match corner {
        Cut::TopRight => [
            point(left, top),
            point(right - cut, top),
            point(right, top + cut),
            point(right, bottom),
            point(left, bottom),
        ],
        Cut::BottomRight => [
            point(left, top),
            point(right, top),
            point(right, bottom - cut),
            point(right - cut, bottom),
            point(left, bottom),
        ],
    };
    let mut path = PathBuilder::fill();
    path.move_to(points[0]);
    for to in &points[1..] {
        path.line_to(*to);
    }
    path.close();
    path.build().ok()
}

/// A Cyberdeck surface, painted under an element's children: `fill` with one corner cut, inside a
/// `width` edge of `edge`. Drawn, not styled, since GPUI's boxes only round their corners.
fn cut_surface(edge: gpui_kit::Hsla, fill: gpui_kit::Hsla, width: f32, cut: f32, corner: Cut) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            if width > 0.
                && let Some(path) = cut_path(bounds, cut, corner)
            {
                window.paint_path(path, edge);
            }
            let inner = Bounds {
                origin: point(bounds.origin.x + px(width), bounds.origin.y + px(width)),
                size: size(bounds.size.width - px(width * 2.), bounds.size.height - px(width * 2.)),
            };
            // The diagonal moves in by less than the straight edges, so the edge is as thick there.
            if let Some(path) = cut_path(inner, (cut - width * 0.41).max(0.), corner) {
                window.paint_path(path, fill);
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
            if pressed && cyberdeck() {
                item.text_color(theme::color(color::INK_INVERSE))
            } else if pressed {
                item.text_color(theme::color(color::INK))
            } else {
                item.text_color(theme::color(color::INK_MUTED)).hover(|s| s.bg(theme::fill_hover()))
            }
        })
}

/// Cyberdeck's selection, under a selected item's children: a solid block of ink with its bottom
/// right corner cut. The item's text turns `ink-inverse` on it.
pub fn selection() -> impl IntoElement {
    let ink = theme::color(color::INK);
    cut_surface(ink, ink, 0., SELECTION_CUT, Cut::BottomRight)
}

/// The ink of a track item's icon or text: on a Cyberdeck selection, which is a block of ink, the
/// inverse.
pub fn item_ink(pressed: bool) -> saturn_design::Token {
    if pressed && cyberdeck() { color::INK_INVERSE } else { color::INK }
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
    let hint = if primary { theme::color(color::INK_ON_ACCENT).opacity(0.7) } else { theme::color(color::INK_FAINT) };
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
pub fn primary_surface<E: gpui_kit::Styled>(element: E) -> E {
    if cyberdeck() {
        // Flat and square, with a lighter bar down its left edge.
        return element
            .text_color(theme::color(color::INK_ON_ACCENT))
            .bg(theme::color(color::BLUE_VIVID))
            .border_l(px(3.))
            .border_color(theme::color(color::BLUE));
    }
    element.text_color(theme::color(color::INK_ON_ACCENT)).bg(accent_fill()).shadow(vec![gpui_kit::BoxShadow {
        color: theme::color(color::BLUE_VIVID).opacity(0.35),
        offset: point(px(0.), px(6.)),
        blur_radius: px(20.),
        spread_radius: px(0.),
        inset: false,
    }])
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
