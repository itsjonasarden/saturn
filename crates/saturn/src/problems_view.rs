//! The Problems view, in the editor panel: every error and warning the language servers report,
//! by file, errors first. Clicking one opens the file there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use lsp_types::{Diagnostic, DiagnosticSeverity};
use saturn_design::{Token, color, radius, space};

use crate::chrome;
use crate::editor_view::EditorView;
use crate::theme::{self, len};

/// How many errors and warnings `problems` hold (hints and notes don't count).
pub fn counts(problems: &BTreeMap<PathBuf, Vec<Diagnostic>>) -> (usize, usize) {
    let all = problems.values().flatten();
    let errors = all.clone().filter(|problem| severity(problem) == DiagnosticSeverity::ERROR).count();
    let warnings = all.filter(|problem| severity(problem) == DiagnosticSeverity::WARNING).count();
    (errors, warnings)
}

/// A problem's severity; a server that leaves it out means an error.
fn severity(problem: &Diagnostic) -> DiagnosticSeverity {
    problem.severity.unwrap_or(DiagnosticSeverity::ERROR)
}

/// Whether a problem is worth listing: errors and warnings, not hints such as an unused name.
fn listed(problem: &Diagnostic) -> bool {
    matches!(severity(problem), DiagnosticSeverity::ERROR | DiagnosticSeverity::WARNING)
}

/// A severity's icon and colour.
pub fn mark(severity: DiagnosticSeverity) -> (&'static str, Token) {
    if severity == DiagnosticSeverity::ERROR { ("circle-x", color::RED) } else { ("triangle-alert", color::AMBER_TEXT) }
}

pub struct ProblemsView {
    editor: WeakEntity<EditorView>,
    problems: BTreeMap<PathBuf, Vec<Diagnostic>>,
}

impl ProblemsView {
    pub fn new(problems: BTreeMap<PathBuf, Vec<Diagnostic>>, editor: WeakEntity<EditorView>) -> Self {
        let mut view = Self { editor, problems: BTreeMap::new() };
        view.problems = Self::listable(problems);
        view
    }

    pub fn set(&mut self, problems: BTreeMap<PathBuf, Vec<Diagnostic>>, cx: &mut Context<Self>) {
        self.problems = Self::listable(problems);
        cx.notify();
    }

    /// Errors and warnings only, errors first, then by line.
    fn listable(problems: BTreeMap<PathBuf, Vec<Diagnostic>>) -> BTreeMap<PathBuf, Vec<Diagnostic>> {
        problems
            .into_iter()
            .filter_map(|(path, list)| {
                let mut list: Vec<Diagnostic> = list.into_iter().filter(listed).collect();
                list.sort_by_key(|problem| {
                    (severity(problem), problem.range.start.line, problem.range.start.character)
                });
                (!list.is_empty()).then_some((path, list))
            })
            .collect()
    }

    fn render_problem(path: &Path, index: usize, problem: &Diagnostic, cx: &mut Context<Self>) -> AnyElement {
        let (target, position) = (path.to_owned(), problem.range.start);
        let (icon, colour) = mark(severity(problem));
        // The first line: servers put details on the ones after.
        let message = problem.message.lines().next().unwrap_or_default().to_owned();
        div()
            .id(gpui_kit::SharedString::from(format!("problem-{}-{index}", path.display())))
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(px(26.))
            .mx(len(space::SPACE_2))
            .px(len(space::SPACE_2))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .text_size(px(13.))
            .hover(|s| s.bg(theme::fill_hover()))
            .child(chrome::icon(icon, 13., theme::color(colour)))
            .child(div().flex_1().min_w_0().truncate().text_color(theme::color(color::INK_BODY)).child(message))
            .child(
                div()
                    .flex_none()
                    .font_family(saturn_design::font::MONO)
                    .text_size(px(11.))
                    .text_color(theme::color(color::INK_FAINT))
                    .child(format!("{}:{}", position.line + 1, position.character + 1)),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                let _ = this.editor.update(cx, |editor, cx| editor.open_at(&target, position, window, cx));
            }))
            .into_any_element()
    }
}

impl Render for ProblemsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (errors, warnings) = counts(&self.problems);
        let plural = |count: usize, one: &str, many: &str| {
            if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
        };
        let headline = match (errors, warnings) {
            (0, 0) => "No problems.".to_owned(),
            (errors, 0) => format!("{}.", plural(errors, "error", "errors")),
            (0, warnings) => format!("{}.", plural(warnings, "warning", "warnings")),
            (errors, warnings) => {
                format!("{} and {}.", plural(errors, "error", "errors"), plural(warnings, "warning", "warnings"))
            }
        };
        let mut list = div().flex().flex_col().pb(len(space::SPACE_4));
        for (path, problems) in &self.problems {
            let root = path.ancestors().skip(1).find(|folder| folder.join(".git").exists()).unwrap_or(Path::new("/"));
            let shown = path.strip_prefix(root).unwrap_or(path);
            let name = shown.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = shown.parent().map(|folder| folder.display().to_string()).unwrap_or_default();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .px(len(space::SPACE_4))
                    .pt(len(space::SPACE_3))
                    .pb(len(space::SPACE_1))
                    .text_size(px(13.))
                    .child(chrome::icon("file-code", 13., theme::color(color::INK_FAINT)))
                    .child(div().font_weight(FontWeight::MEDIUM).text_color(theme::color(color::INK)).child(name))
                    .child(div().min_w_0().truncate().text_color(theme::color(color::INK_FAINT)).child(folder))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::color(color::INK_FAINT))
                            .child(problems.len().to_string()),
                    ),
            );
            for (index, problem) in problems.iter().enumerate() {
                list = list.child(Self::render_problem(path, index, problem, cx));
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(len(space::SPACE_4))
                    .py(len(space::SPACE_3))
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(color::INK))
                    .child(headline),
            )
            .child(div().id("problems-list").flex_1().min_h_0().overflow_y_scroll().child(list))
    }
}
