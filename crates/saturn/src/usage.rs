//! The plans' usage, read for the whole app and shown at the foot of every window's sidebar:
//! Claude's (`/usage`: the session, the week, a model's week) and Codex's (every window its plan
//! has), each asked of its agent. Read every few minutes and soon after an agent finishes a turn;
//! each hidden when its agent has no plan limits to report or can't be asked.
//!
//! Claude's come first from what Claude Code hands Saturn's status line, which costs nothing:
//! asking Claude Code is a request to Anthropic's usage endpoint, which refuses one asked too often
//! (and Claude Code then doesn't ask again for some twenty minutes).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::{
    App, AppContext as _, FontWeight, Global, Hsla, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, px,
};
use saturn_core::agent_state::AgentState;
use saturn_core::usage::{self, Limit};
use saturn_design::space;

use crate::theme::TypeScale as _;
use crate::theme::len;
use saturn_design::text;
use saturn_design::tone;

/// How often the usage is read while nothing happens.
const EVERY: Duration = Duration::from_mins(5);
/// The least time between two reads, however many turns end.
const AT_MOST_EVERY: Duration = Duration::from_secs(60);
/// The least time between two questions to Claude Code, unless asked by hand.
const ASK_CLAUDE_EVERY: Duration = Duration::from_mins(10);

#[derive(Default)]
pub struct Usage {
    /// Claude's plan.
    pub limits: Option<Vec<Limit>>,
    /// Codex's plan.
    pub codex: Option<Vec<Limit>>,
    read_at: Option<Instant>,
    /// When Claude Code was last asked, rather than its status line read.
    asked_claude_at: Option<Instant>,
    reading: bool,
    /// The highest warning given for each limit's current window (80 or 95), by label and reset.
    warned: std::collections::HashMap<(String, Option<i64>), u8>,
}

/// Where a limit gets a notification: once nearly used, once almost gone.
const WARN_AT: [u8; 2] = [80, 95];

/// The 5-hour window's start, from the session limit's reset: what agents' own use is measured
/// against.
pub fn window_start(cx: &App) -> i64 {
    let resets = cx
        .try_global::<Usage>()
        .and_then(|usage| usage.limits.as_ref())
        .and_then(|limits| limits.iter().find(|limit| limit.label == "Session"))
        .and_then(|limit| limit.resets_at);
    resets.map_or_else(|| unix_now() - 5 * 3600, |resets| resets - 5 * 3600)
}

/// The session limit's use, 0–100, when the plan has one.
pub fn session_percent(cx: &App) -> Option<f64> {
    cx.try_global::<Usage>()?.limits.as_ref()?.iter().find(|limit| limit.label == "Session").map(|limit| limit.percent)
}

/// A notification when a limit passes 80% or 95% of its window, once each per window.
fn warn(usage: &mut Usage) {
    let plans = [("", usage.limits.clone()), ("Codex ", usage.codex.clone())];
    for (whose, limits) in plans {
        warn_about(usage, whose, &limits.unwrap_or_default());
    }
}

/// Warns about each of a plan's limits nearly used (`whose`: "" for Claude's, "Codex " for Codex's).
fn warn_about(usage: &mut Usage, whose: &str, limits: &[Limit]) {
    for limit in limits {
        let level = WARN_AT.iter().rev().copied().find(|at| limit.percent >= f64::from(*at));
        let Some(level) = level else { continue };
        let key = (format!("{whose}{}", limit.label), limit.resets_at);
        if usage.warned.get(&key).is_some_and(|warned| *warned >= level) {
            continue;
        }
        usage.warned.insert(key, level);
        let which = match limit.label.as_str() {
            "Session" => "5-hour limit".to_owned(),
            "Week" => "weekly limit".to_owned(),
            model => format!("weekly {model} limit"),
        };
        let resets =
            limit.resets_at.map(|at| format!(" It resets {}.", resets_in(at - unix_now()))).unwrap_or_default();
        let title = format!("{level}% of your {whose}{which} used");
        let body = format!("Agents slow down or stop when it runs out.{resets}");
        crate::notify::post(&format!("limit-{whose}{}", limit.label), &title, &body, false);
    }
}

/// "in 2 h 15 min", "in 3 days".
fn resets_in(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    match minutes {
        0..=59 => format!("in {minutes} min"),
        60..=1439 => format!("in {} h {} min", minutes / 60, minutes % 60),
        _ => format!("in {} days", minutes / 1440),
    }
}

impl Global for Usage {}

/// Starts reading the usage, once for the app. Automated runs show made-up numbers when asked
/// (`SATURN_FAKE_USAGE`), and read the real ones only when asked to (`SATURN_USAGE_LIVE`).
pub fn start(cx: &mut App) {
    cx.set_global(Usage::default());
    let live = std::env::var_os("SATURN_USAGE_LIVE").is_some();
    if (crate::spike::is_automated() && !live) || std::env::var_os("SATURN_FAKE_USAGE").is_some() {
        if std::env::var_os("SATURN_FAKE_USAGE").is_some() {
            cx.global_mut::<Usage>().limits = Some(made_up());
            if std::env::var_os("SATURN_AGENTS").is_some_and(|agents| agents.to_string_lossy().contains("codex")) {
                let now = unix_now();
                cx.global_mut::<Usage>().codex =
                    Some(vec![Limit { label: "Week".to_owned(), percent: 23., resets_at: Some(now + 5 * 86_400) }]);
            }
        }
        return;
    }
    cx.spawn(async move |cx| {
        // The login shell's PATH (where `claude` lives) is read as Saturn starts.
        cx.background_executor().timer(Duration::from_secs(3)).await;
        loop {
            cx.update(|cx| refresh(false, cx));
            cx.background_executor().timer(EVERY).await;
        }
    })
    .detach();
}

/// Reads the usage again: `now` when asked by hand, else not within a minute of the last read.
pub fn refresh(now: bool, cx: &mut App) {
    let Some(usage) = cx.try_global::<Usage>() else { return };
    let recent = usage.read_at.is_some_and(|at| at.elapsed() < AT_MOST_EVERY);
    if usage.reading || (recent && !now) || std::env::var_os("SATURN_FAKE_USAGE").is_some() {
        return;
    }
    let may_ask_claude = now || usage.asked_claude_at.is_none_or(|at| at.elapsed() >= ASK_CLAUDE_EVERY);
    cx.global_mut::<Usage>().reading = true;
    let path = crate::lsp::login_path();
    cx.spawn(async move |cx| {
        let (claude, asked, codex) = cx
            .background_spawn(async move {
                // Only what Codex says now: a figure from an old session would be out of date.
                let codex = saturn_core::codex_usage::read(&path);
                let now = u64::try_from(unix_now()).unwrap_or(0);
                match crate::status_line::recent_plan_limits(now) {
                    Some(limits) => (Some(Some(limits)), false, codex),
                    None if may_ask_claude => (Some(usage::read(&path)), true, codex),
                    // Asked a few minutes ago: what it said stands until the next question.
                    None => (None, false, codex),
                }
            })
            .await;
        cx.update(|cx| {
            let usage = cx.global_mut::<Usage>();
            usage.reading = false;
            usage.read_at = Some(Instant::now());
            if asked {
                usage.asked_claude_at = Some(Instant::now());
            }
            if let Some(limits) = claude {
                usage.limits = limits;
            }
            usage.codex = codex;
            warn(usage);
            cx.refresh_windows();
        });
    })
    .detach();
}

/// The demo's numbers.
fn made_up() -> Vec<Limit> {
    let now = unix_now();
    vec![
        Limit { label: "Session".to_owned(), percent: 34., resets_at: Some(now + 2 * 3600 + 900) },
        Limit { label: "Week".to_owned(), percent: 61., resets_at: Some(now + 4 * 86_400) },
        Limit { label: "Fable".to_owned(), percent: 12., resets_at: Some(now + 4 * 86_400) },
    ]
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
}

/// White at `alpha`: the gradient card always carries white text. A slab card is the theme's
/// grey, so there it is the theme's ink.
fn white(alpha: f32) -> Hsla {
    if crate::chrome::look().agent_cards == saturn_design::AgentCards::Slab {
        return crate::theme::color(saturn_design::color::INK).opacity(alpha);
    }
    gpui_kit::hsla(0., 0., 1., alpha)
}

/// The dotted meter: a row of square dots, the used ones filled, orange from 80%.
fn dots(percent: f64) -> gpui_kit::Div {
    const DOTS: u8 = 25;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a share of 25 dots")]
    let lit = ((percent / 100.).clamp(0., 1.) * f64::from(DOTS)).round() as u8;
    let fill =
        crate::theme::color(if percent >= 80. { saturn_design::color::ORANGE } else { saturn_design::color::INK });
    div().flex().gap(len(space::SPACE_0)).children((0..DOTS).map(|index| {
        let dot = div().flex_1().h(px(4.));
        if index < lit { dot.bg(fill) } else { dot.bg(crate::theme::tone(tone::RULE)) }
    }))
}

/// A hairline meter on the gradient, white with a soft glow; orange from 80%.
fn meter(percent: f64) -> gpui_kit::AnyElement {
    if crate::chrome::look().meters == saturn_design::Meters::Dots {
        return dots(percent).into_any_element();
    }
    #[expect(clippy::cast_possible_truncation, reason = "a share between 0 and 1")]
    let width = (percent / 100.).clamp(0., 1.) as f32;
    let fill = if percent >= 80. { gpui_kit::Hsla::from(gpui_kit::rgb(0xff_b3_7a)) } else { white(1.) };
    div()
        .relative()
        .h(px(2.))
        .w_full()
        .rounded_full()
        .bg(white(0.18))
        .child(div().absolute().top_0().left_0().h_full().w(gpui_kit::relative(width)).rounded_full().bg(fill).shadow(
            vec![gpui_kit::BoxShadow {
                color: fill.opacity(0.6),
                offset: gpui_kit::point(px(0.), px(0.)),
                blur_radius: px(6.),
                spread_radius: px(0.),
                inset: false,
            }],
        ))
        .into_any_element()
}

/// The plan's limits as a card at the foot of the sidebar, on the same gradient as an agent's.
/// Clicking it reads them again.
pub fn render(cx: &App) -> Option<impl IntoElement> {
    let usage = cx.try_global::<Usage>()?;
    let plans: Vec<(&str, Vec<Limit>)> = [("Claude", usage.limits.clone()), ("Codex", usage.codex.clone())]
        .into_iter()
        .filter_map(|(name, limits)| Some((name, limits.filter(|limits| !limits.is_empty())?)))
        .collect();
    if plans.is_empty() {
        return None;
    }
    let card = crate::agents_view::surface("usage", AgentState::Working)
        .gap(len(space::SPACE_3))
        .cursor_pointer()
        .on_click(|_, _, cx| refresh(true, cx));
    // One plan is "Usage"; two are told apart by their agent.
    let titled = plans.len() > 1;
    let mut card = card;
    for (name, limits) in plans {
        let title =
            div().relative().text_scale(text::META).text_color(white(0.62)).child(if titled { name } else { "Usage" });
        card = card.child(title).child(limit_rows(&limits));
    }
    Some(card)
}

/// Each limit of a plan: its name, when it resets, how much is used, and a meter.
fn limit_rows(limits: &[Limit]) -> impl IntoElement {
    let now = unix_now();
    let resets = |limit: &Limit| limit.resets_at.map(|at| usage::resets_in(at, now)).unwrap_or_default();
    let mut rows = div().relative().flex().flex_col().gap(len(space::SPACE_3));
    for limit in limits {
        rows = rows.child(
            div()
                .flex()
                .flex_col()
                .gap(len(space::SPACE_2))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(len(space::SPACE_2))
                        .text_scale(text::LABEL)
                        .text_color(white(1.))
                        .child(limit.label.clone())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_scale(text::MICRO)
                                .text_color(white(0.5))
                                .child(format!("resets {}", resets(limit))),
                        )
                        .child(div().font_weight(FontWeight::MEDIUM).child(format!("{:.0}%", limit.percent))),
                )
                .child(meter(limit.percent)),
        );
    }
    rows
}
