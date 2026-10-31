//! What the agent is told: for the focused pane's folder, everything its agent is given, found as
//! that agent finds it (`saturn_core::instructions`): Claude Code's instruction files, rules and the
//! files they're for, skills, commands, subagents, hooks and MCP servers; Codex's `AGENTS.md` files
//! and skills. Long instruction files
//! get a word, and anything said in more than one file is listed first. Each opens in the editor.

use std::path::PathBuf;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, px,
};
use saturn_core::agents::Agent;
use saturn_core::instructions::{self, Kind, Repeat, Source};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::Workspace;
use saturn_design::text;

/// Opens the list for an agent working in `folder`, inside `checkout`.
pub fn open(
    workspace: WeakEntity<Workspace>,
    checkout: PathBuf,
    folder: &std::path::Path,
    agent: Agent,
    window: &mut Window,
    cx: &mut App,
) {
    let sources = match agent {
        Agent::Claude => instructions::gather(&checkout, folder, saturn_core::prompts::claude_home().as_deref()),
        Agent::Codex => {
            let user_home = std::env::var_os("HOME").map(PathBuf::from);
            instructions::gather_codex(&checkout, folder, Agent::Codex.home().as_deref(), user_home.as_deref())
        }
    };
    let repeats = instructions::repeats(&sources);
    window.open_dialog(cx, move |dialog, _, _| {
        let mut list =
            div().id("context").flex().flex_col().gap(len(space::SPACE_1)).max_h(px(600.)).overflow_y_scroll();
        if !repeats.is_empty() {
            list = list.child(heading("Said in more than one place"));
            for (index, repeat) in repeats.iter().enumerate() {
                list = list.child(repeat_row(&workspace, index, repeat, &checkout));
            }
        }
        let kinds =
            [Kind::Instructions, Kind::Rule, Kind::Skill, Kind::Command, Kind::Subagent, Kind::Hook, Kind::McpServer];
        for kind in kinds {
            let of_kind: Vec<&Source> = sources.iter().filter(|source| source.kind == kind).collect();
            if of_kind.is_empty() {
                continue;
            }
            list = list.child(heading(kind.heading()));
            for (index, source) in of_kind.into_iter().enumerate() {
                list = list.child(source_row(&workspace, (kind, index), source));
            }
        }
        if sources.is_empty() {
            list = list.child(
                div()
                    .text_scale(text::BODY)
                    .text_color(theme::color(color::INK_MUTED))
                    .child("Nothing here tells the agent anything: no CLAUDE.md, AGENTS.md, rules or skills."),
            );
        }
        dialog
            .title("What the agent is told")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(saturn_design::size::DIALOG_LARGE))
            .child(list)
    });
}

fn heading(text: &str) -> impl IntoElement {
    div()
        .pt(len(space::SPACE_3))
        .pb(len(space::SPACE_1))
        .px(len(space::SPACE_2))
        .text_scale(text::MICRO)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(color::INK_FAINT))
        .child(text.to_uppercase())
}

/// Opens a file in the editor, at a line, and closes the list.
fn open_at(
    workspace: &WeakEntity<Workspace>,
    path: PathBuf,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let workspace = workspace.clone();
    move |_, window, cx| {
        let _ = workspace.update(cx, |workspace, cx| workspace.open_file(&path, window, cx));
        window.close_dialog(cx);
    }
}

fn source_row(workspace: &WeakEntity<Workspace>, id: (Kind, usize), source: &Source) -> impl IntoElement {
    let long = source.kind == Kind::Instructions && source.lines > instructions::LONG;
    let size = (source.lines > 0).then(|| format!("{} lines", source.lines));
    div()
        .id(gpui_kit::SharedString::from(format!("source-{:?}-{}", id.0, id.1)))
        .flex()
        .items_center()
        .gap(len(space::SPACE_3))
        .px(len(space::SPACE_2))
        .py(len(space::SPACE_1))
        .rounded(theme::corner(radius::RADIUS_SM))
        .cursor_pointer()
        .hover(|s| s.bg(theme::fill_hover()))
        .child(
            div()
                .flex_none()
                .text_scale(text::LABEL)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::color(color::INK))
                .child(source.name.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_scale(text::META)
                .text_color(theme::color(color::INK_FAINT))
                .children(source.detail.clone()),
        )
        .children(size.map(|size| {
            div()
                .flex_none()
                .text_scale(text::META)
                .text_color(theme::color(if long { color::AMBER_TEXT } else { color::INK_FAINT }))
                .child(if long { format!("{size}: long files are followed less") } else { size })
        }))
        .on_click(open_at(workspace, source.path.clone()))
}

fn repeat_row(
    workspace: &WeakEntity<Workspace>,
    index: usize,
    repeat: &Repeat,
    checkout: &std::path::Path,
) -> impl IntoElement {
    let mut places = div().flex().flex_wrap().gap(len(space::SPACE_2));
    for (place, (path, line)) in repeat.places.iter().enumerate() {
        let name = path
            .strip_prefix(checkout)
            .map_or_else(|_| path.display().to_string(), |relative| relative.display().to_string());
        places = places.child(
            div()
                .id(("repeat-place", index * 100 + place))
                .text_scale(text::META)
                .text_color(theme::color(color::INK_MUTED))
                .cursor_pointer()
                .hover(|s| s.text_color(theme::color(color::INK)))
                .child(format!("{name}:{line}"))
                .on_click(open_at(workspace, path.clone())),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(len(space::SPACE_0))
        .px(len(space::SPACE_2))
        .py(len(space::SPACE_1))
        .child(div().text_scale(text::LABEL).text_color(theme::color(color::INK)).truncate().child(repeat.text.clone()))
        .child(places)
}
