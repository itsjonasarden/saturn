//! The References view, in the editor panel: everywhere a symbol is used, as the language server
//! found it, grouped by file with each line's text. Clicking one opens the file there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use lsp_types::{Location, Position};
use saturn_core::lsp;
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::editor_view::EditorView;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use saturn_design::text;

/// A place the symbol is used: its line (0-based) and column, and the line's text.
struct Use {
    position: Position,
    text: String,
}

pub struct ReferencesView {
    editor: WeakEntity<EditorView>,
    /// The file the search started in, to show paths relative to its repository.
    root: PathBuf,
    files: BTreeMap<PathBuf, Vec<Use>>,
}

impl ReferencesView {
    pub fn new(from: &Path, locations: Vec<Location>, editor: WeakEntity<EditorView>, cx: &mut Context<Self>) -> Self {
        let mut view = Self { editor, root: PathBuf::new(), files: BTreeMap::new() };
        view.set(from, locations, cx);
        view
    }

    /// Shows `locations`, found from a symbol in `from`.
    pub fn set(&mut self, from: &Path, locations: Vec<Location>, cx: &mut Context<Self>) {
        from.ancestors()
            .skip(1)
            .find(|folder| folder.join(".git").exists())
            .or_else(|| from.parent())
            .unwrap_or(Path::new("/"))
            .clone_into(&mut self.root);
        self.files.clear();
        let mut by_file: BTreeMap<PathBuf, Vec<Position>> = BTreeMap::new();
        for location in locations {
            if let Some(path) = lsp::path(location.uri.as_str()) {
                by_file.entry(path).or_default().push(location.range.start);
            }
        }
        for (path, mut positions) in by_file {
            positions.sort_by_key(|position| (position.line, position.character));
            positions.dedup();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            let uses = positions
                .into_iter()
                .map(|position| {
                    let line = usize::try_from(position.line).ok().and_then(|line| lines.get(line)).copied();
                    Use { position, text: line.unwrap_or_default().trim().to_owned() }
                })
                .collect();
            self.files.insert(path, uses);
        }
        cx.notify();
    }

    fn render_use(path: &Path, index: usize, item: &Use, cx: &mut Context<Self>) -> AnyElement {
        let (target, position) = (path.to_owned(), item.position);
        div()
            .id(gpui_kit::SharedString::from(format!("use-{}-{index}", path.display())))
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .h(px(26.))
            .mx(len(space::SPACE_2))
            .px(len(space::SPACE_2))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .hover(|s| s.bg(theme::fill_hover()))
            .child(
                div()
                    .flex_none()
                    .w(px(40.))
                    .text_scale(text::META)
                    .font_family(saturn_design::font::MONO)
                    .text_color(theme::color(color::INK_FAINT))
                    .child((position.line + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(saturn_design::font::MONO)
                    .text_scale(text::META)
                    .text_color(theme::color(color::INK_BODY))
                    .child(item.text.clone()),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                let _ = this.editor.update(cx, |editor, cx| editor.open_at(&target, position, window, cx));
            }))
            .into_any_element()
    }
}

impl Render for ReferencesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count: usize = self.files.values().map(Vec::len).sum();
        let headline = match (count, self.files.len()) {
            (0, _) => "Not used anywhere else.".to_owned(),
            (1, _) => "Used once.".to_owned(),
            (uses, 1) => format!("Used {uses} times, in one file."),
            (uses, files) => format!("Used {uses} times, in {files} files."),
        };
        let mut list = div().flex().flex_col().pb(len(space::SPACE_4));
        for (path, uses) in &self.files {
            let shown = path.strip_prefix(&self.root).unwrap_or(path);
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
                    .text_scale(text::LABEL)
                    .child(chrome::icon("file-code", 13., theme::color(color::INK_FAINT)))
                    .child(div().font_weight(FontWeight::MEDIUM).text_color(theme::color(color::INK)).child(name))
                    .child(div().min_w_0().truncate().text_color(theme::color(color::INK_FAINT)).child(folder))
                    .child(
                        div()
                            .text_scale(text::META)
                            .text_color(theme::color(color::INK_FAINT))
                            .child(uses.len().to_string()),
                    ),
            );
            for (index, item) in uses.iter().enumerate() {
                list = list.child(Self::render_use(path, index, item, cx));
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
                    .text_scale(text::BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(color::INK))
                    .child(headline),
            )
            .child(div().id("references-list").flex_1().min_h_0().overflow_y_scroll().child(list))
    }
}
