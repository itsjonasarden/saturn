//! The gradient card: the one coloured surface on a screen, deep to near black with a state's
//! colour pooling in. The Claude plan's usage card at the foot of the sidebar stands on it.

use akyba_core::agent_state::AgentState;
use akyba_design::{radius, space};
use gpui_kit::{
    ColorSpace, ElementId, Hsla, InteractiveElement as _, ParentElement as _, Styled as _, div, linear_color_stop,
    linear_gradient, rgb,
};

use crate::theme::{self, len};
use akyba_design::tone;

/// The gradient's bright and deep ends for a state. The card always carries white text, so these
/// are the same in light and dark mode.
fn tint(state: AgentState) -> (u32, u32, u32) {
    match state {
        AgentState::NeedsYou | AgentState::Error => (0xff_8a_3d, 0x4a_1d_05, 0x0d_06_03),
        AgentState::YourTurn => (0x2f_9e_57, 0x0b_3a_1c, 0x03_0b_06),
        _ => (0x1c_69_e3, 0x07_28_69, 0x04_08_14),
    }
}

/// The gradient surface, tinted by `state`: deep to near black, the state's bright colour pooling
/// in from the lower left. It sits 4px inside the sidebar's card (radius 10), so its corners are 6.
pub fn surface(id: impl Into<ElementId>, state: AgentState) -> gpui_kit::Stateful<gpui_kit::Div> {
    if crate::chrome::look().agent_cards == akyba_design::AgentCards::Slab {
        // No gradient: a raised slab with its top right corner cut.
        let edge = theme::tone(tone::EDGE);
        return div().id(id).relative().flex().flex_col().gap(len(space::SPACE_1)).p(len(space::SPACE_3)).child(
            crate::chrome::cut_surface(
                edge,
                theme::color(akyba_design::color::SURFACE_RAISED),
                1.,
                10.,
                crate::chrome::Cut::TopRight,
            ),
        );
    }
    let (bright, deep, end) = tint(state);
    div()
        .id(id)
        .relative()
        .flex()
        .flex_col()
        .gap(len(space::SPACE_1))
        .p(len(space::SPACE_3))
        .rounded(theme::corner(radius::RADIUS_SM))
        .overflow_hidden()
        .bg(linear_gradient(155., linear_color_stop(rgb(deep), 0.), linear_color_stop(rgb(end), 1.))
            .color_space(ColorSpace::Oklab))
        // The light pooling in from the lower left, as in the design system's gradient card. Rounded
        // like the card: GPUI clips children to a rectangle, not to rounded corners.
        .child(
            div().absolute().top_0().left_0().size_full().rounded(theme::corner(radius::RADIUS_SM)).bg(
                linear_gradient(
                    30.,
                    linear_color_stop(Hsla::from(rgb(bright)).opacity(0.7), 0.),
                    linear_color_stop(Hsla::from(rgb(bright)).opacity(0.), 0.85),
                )
                .color_space(ColorSpace::Oklab),
            ),
        )
}
