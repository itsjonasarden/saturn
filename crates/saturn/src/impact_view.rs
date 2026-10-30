//! The Impact view, in the editor panel: what the changes reach. Every JavaScript, TypeScript,
//! Rust and Python file that imports a changed file, directly or through others, grouped by the folder it lives
//! in, nearest first. Clicking a file focuses it: what it uses on the left, what uses it on the
//! right, and clicking either side walks the graph.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::diff::DiffFile;
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use saturn_core::dbt;
use saturn_core::dbt_impact::{self, Blast};
use saturn_core::git::{self, Scope};
use saturn_core::imports::{self, Affected, Graph};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::editor_view::EditorView;
use crate::theme::{self, len};

const ROW_HEIGHT: f32 = 28.;

pub struct ImpactView {
    repo: PathBuf,
    editor: WeakEntity<EditorView>,
    graph: Arc<Graph>,
    scope: Scope,
    /// The changed files, relative to the repository's top.
    changed: Vec<PathBuf>,
    affected: Vec<Affected>,
    /// For each dbt project a change touches (its folder, relative to the repository's top): the
    /// models built from what changed, and the exposures reading them.
    blasts: Vec<(PathBuf, Blast)>,
    /// The file in focus, and the ones focused before it, for Back.
    focus: Option<PathBuf>,
    trail: Vec<PathBuf>,
    loaded: bool,
}

impl ImpactView {
    pub fn new(repo: &Path, editor: WeakEntity<EditorView>, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            repo: repo.to_owned(),
            editor,
            graph: Arc::default(),
            scope: Scope::Uncommitted,
            changed: Vec::new(),
            affected: Vec::new(),
            blasts: Vec::new(),
            focus: None,
            trail: Vec::new(),
            loaded: false,
        };
        view.reload(cx);
        view
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    /// Reads the imports and the changes again, off the main thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (repo, scope) = (self.repo.clone(), self.scope.clone());
        let load = move || {
            let graph = imports::build(&repo);
            let changed = changed_files(&repo, scope);
            let blasts = dbt_blasts(&repo, &changed);
            (graph, changed, blasts)
        };
        if std::env::var_os("SATURN_IMPACT").is_some() {
            // Screenshots: the test window stops drawing before a background read would land.
            let (graph, changed, blasts) = load();
            self.loaded(graph, changed, blasts);
            self.focus = std::env::var_os("SATURN_IMPACT_FOCUS").map(PathBuf::from);
            return;
        }
        cx.spawn(async move |this, cx| {
            let (graph, changed, blasts) = cx.background_spawn(async move { load() }).await;
            let _ = this.update(cx, |this, cx| {
                this.loaded(graph, changed, blasts);
                cx.notify();
            });
        })
        .detach();
    }

    fn loaded(&mut self, graph: Graph, changed: Vec<PathBuf>, blasts: Vec<(PathBuf, Blast)>) {
        self.affected = graph.impact(&changed);
        self.blasts = blasts;
        self.graph = Arc::new(graph);
        self.changed = changed;
        self.loaded = true;
    }

    /// Focuses `file`: what it uses and what uses it.
    pub fn focus(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        if let Some(previous) = self.focus.replace(file) {
            self.trail.push(previous);
        }
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.focus = self.trail.pop();
        cx.notify();
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.loaded = false;
            self.reload(cx);
            cx.notify();
        }
    }

    /// "used by 3", or "used by nothing".
    fn used_by(&self, file: &Path) -> String {
        match self.graph.importers.get(file).map_or(0, Vec::len) {
            0 => "used by nothing".to_owned(),
            count => format!("used by {count}"),
        }
    }

    fn open(&self, file: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.repo.join(file);
        let _ = self.editor.update(cx, |editor, cx| editor.open(&path, window, cx));
    }

    fn render_impact(&self, cx: &mut Context<Self>) -> AnyElement {
        let followed: Vec<&PathBuf> =
            self.changed.iter().filter(|file| self.graph.imports.contains_key(*file)).collect();
        let headline = match (self.changed.len(), followed.len(), self.affected.len()) {
            (0, _, _) => "Nothing has changed here yet.".to_owned(),
            (_, 0, _) if !self.blasts.is_empty() => dbt_headline(&self.blasts),
            (_, 0, _) => "None of the changed files are JavaScript, TypeScript, Rust or Python.".to_owned(),
            (_, n, 0) => format!("{} used by nothing else.", files(n, "changed file is", "changed files are")),
            (_, n, m) => {
                format!("{} used by {}.", files(n, "changed file is", "changed files are"), files(m, "other", "others"))
            }
        };
        let mut areas: BTreeMap<String, Vec<&Affected>> = BTreeMap::new();
        for affected in &self.affected {
            areas.entry(imports::area(&affected.file)).or_default().push(affected);
        }
        let mut areas: Vec<(String, Vec<&Affected>)> = areas.into_iter().collect();
        // Nearest first: folders with direct importers, then the bigger ones.
        let nearest = |files: &[&Affected]| files.iter().map(|a| a.depth).min().unwrap_or(usize::MAX);
        areas.sort_by(|a, b| {
            nearest(&a.1).cmp(&nearest(&b.1)).then(b.1.len().cmp(&a.1.len())).then_with(|| a.0.cmp(&b.0))
        });
        // Which changed file a file leads back to only says something when several changed.
        let several = followed.len() > 1;

        let mut list = div().flex().flex_col().pb(len(space::SPACE_4));
        if !followed.is_empty() {
            list = list.child(section_header("Changed", followed.len()));
            for file in followed {
                list = list.child(self.file_row(file, self.used_by(file), None, cx));
            }
        }
        for (area, affected) in areas {
            list = list.child(section_header(&area, affected.len()));
            for item in affected {
                let note = match item.depth {
                    1 => "imports it".to_owned(),
                    depth => format!("{depth} steps away"),
                };
                let via = several.then_some(item.because_of.as_path());
                list = list.child(self.file_row(&item.file, note, via, cx));
            }
        }
        for (project, blast) in &self.blasts {
            list = Self::render_blast(list, project, blast, cx);
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(len(space::SPACE_3))
                    .px(len(space::SPACE_4))
                    .py(len(space::SPACE_3))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(color::INK))
                            .child(if self.loaded { headline } else { "Reading the imports…".to_owned() }),
                    )
                    .child(
                        chrome::track()
                            .child(
                                chrome::track_item("impact-uncommitted", self.scope == Scope::Uncommitted)
                                    .px(len(space::SPACE_3))
                                    .child("Uncommitted")
                                    .on_click(cx.listener(|this, _, _, cx| this.set_scope(Scope::Uncommitted, cx))),
                            )
                            .child(
                                chrome::track_item("impact-branch", self.scope == Scope::Branch)
                                    .px(len(space::SPACE_3))
                                    .child("Branch")
                                    .on_click(cx.listener(|this, _, _, cx| this.set_scope(Scope::Branch, cx))),
                            ),
                    ),
            )
            .child(div().id("impact-list").flex_1().min_h_0().overflow_y_scroll().child(list))
            .into_any_element()
    }

    /// A dbt project's part of the impact: the changed models, what's built from them, and the
    /// exposures reading any of it. Each opens its file.
    fn render_blast(mut list: gpui_kit::Div, project: &Path, blast: &Blast, cx: &mut Context<Self>) -> gpui_kit::Div {
        let file = |path: Option<&PathBuf>| path.map(|path| project.join(path));
        list = list.child(section_header(&format!("dbt {}: changed", blast.lineage.name), blast.changed.len()));
        for &index in &blast.changed {
            let node = &blast.lineage.nodes[index];
            list = list.child(Self::dbt_row(
                ("dbt-changed", index),
                "workflow",
                &node.name,
                node.kind.name().to_owned(),
                file(node.path.as_ref()),
                cx,
            ));
        }
        if !blast.downstream.is_empty() {
            list = list.child(section_header("Built from them", blast.downstream.len()));
            for reached in &blast.downstream {
                let node = &blast.lineage.nodes[reached.node];
                let distance =
                    if reached.steps == 1 { "reads it".to_owned() } else { format!("{} steps away", reached.steps) };
                list = list.child(Self::dbt_row(
                    ("dbt-downstream", reached.node),
                    "workflow",
                    &node.name,
                    distance,
                    file(node.path.as_ref()),
                    cx,
                ));
            }
        }
        if !blast.exposures.is_empty() {
            list = list.child(section_header("Exposures that read them", blast.exposures.len()));
            for (index, exposure) in blast.exposures.iter().enumerate() {
                list = list.child(Self::dbt_row(
                    ("dbt-exposure", index),
                    "globe",
                    &exposure.name,
                    exposure.kind.clone(),
                    file(Some(&exposure.path)),
                    cx,
                ));
            }
        }
        list
    }

    /// A dbt model or exposure in the impact: its name, a note, and its file to open.
    fn dbt_row(
        id: (&'static str, usize),
        icon: &'static str,
        name: &str,
        note: String,
        file: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(px(ROW_HEIGHT))
            .mx(len(space::SPACE_2))
            .px(len(space::SPACE_2))
            .rounded(theme::corner(radius::RADIUS_SM))
            .text_size(px(13.))
            .when(file.is_some(), |row| row.cursor_pointer().hover(|s| s.bg(theme::fill_hover())))
            .child(chrome::icon(icon, 13., theme::color(color::INK_FAINT)))
            .child(div().flex_1().min_w_0().truncate().text_color(theme::color(color::INK_BODY)).child(name.to_owned()))
            .child(div().flex_none().text_size(px(12.)).text_color(theme::color(color::INK_MUTED)).child(note))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(file) = &file {
                    this.open(file, window, cx);
                }
            }))
            .into_any_element()
    }

    /// A file in a list: its name and folder, a note on the right, and the changed file it leads
    /// back to. Clicking focuses it.
    fn file_row(&self, file: &Path, note: String, via: Option<&Path>, cx: &mut Context<Self>) -> AnyElement {
        let focused = file.to_owned();
        let selected = self.focus.as_deref() == Some(file);
        div()
            .id(gpui_kit::SharedString::from(format!("file-{}", file.display())))
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(px(ROW_HEIGHT))
            .mx(len(space::SPACE_2))
            .px(len(space::SPACE_2))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .text_size(px(13.))
            .when(selected, |row| row.bg(theme::fill_pressed()))
            .when(!selected, |row| row.hover(|s| s.bg(theme::fill_hover())))
            .child(chrome::icon("file-code", 13., theme::color(color::INK_FAINT)))
            .child(div().flex_none().text_color(theme::color(color::INK_BODY)).child(name(file)))
            .child(div().flex_1().min_w_0().truncate().text_color(theme::color(color::INK_FAINT)).child(folder(file)))
            .children(via.map(|via| {
                div()
                    .flex_none()
                    .max_w(px(200.))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(theme::color(color::INK_FAINT))
                    .child(format!("via {}", name(via)))
            }))
            .child(div().flex_none().text_size(px(12.)).text_color(theme::color(color::INK_MUTED)).child(note))
            .on_click(cx.listener(move |this, _, _, cx| this.focus(focused.clone(), cx)))
            .into_any_element()
    }

    fn render_focus(&self, file: &Path, cx: &mut Context<Self>) -> AnyElement {
        let uses = self.graph.imports.get(file).cloned().unwrap_or_default();
        let used_by = self.graph.importers.get(file).cloned().unwrap_or_default();
        let followed = self.graph.imports.contains_key(file);
        let opened = file.to_owned();
        let column = |title: &'static str, files: &[PathBuf], empty: &'static str, cx: &mut Context<Self>| {
            div()
                .id(title)
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .child(section_header(title, files.len()))
                .when(files.is_empty(), |column| {
                    column.child(
                        div()
                            .px(len(space::SPACE_4))
                            .text_size(px(13.))
                            .text_color(theme::color(color::INK_FAINT))
                            .child(empty),
                    )
                })
                .children(files.iter().map(|other| self.file_row(other, self.used_by(other), None, cx)))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .px(len(space::SPACE_2))
                    .py(len(space::SPACE_2))
                    .border_b_1()
                    .border_color(theme::separator())
                    .child(
                        chrome::icon_button("impact-back", "arrow-left", true)
                            .tooltip(chrome::tip("Back"))
                            .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(color::INK))
                            .child(name(file)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(theme::color(color::INK_FAINT))
                            .child(folder(file)),
                    )
                    .child(
                        chrome::button("impact-open", "Open file", None, false)
                            .on_click(cx.listener(move |this, _, window, cx| this.open(&opened, window, cx))),
                    ),
            )
            .child(if followed {
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(column("Uses", &uses, "Imports nothing from this repository.", cx))
                    .child(div().flex_none().w(px(1.)).h_full().bg(theme::separator()))
                    .child(column("Used by", &used_by, "Nothing imports it.", cx))
                    .into_any_element()
            } else {
                div()
                    .p(len(space::SPACE_4))
                    .text_size(px(13.))
                    .text_color(theme::color(color::INK_MUTED))
                    .child("Saturn follows imports in JavaScript, TypeScript, Rust and Python.")
                    .into_any_element()
            })
            .into_any_element()
    }
}

/// The files `scope` changed, relative to the repository's top (a renamed file by its new name).
fn changed_files(repo: &Path, scope: Scope) -> Vec<PathBuf> {
    let files = DiffFile::parse(&git::patch(repo, scope)).unwrap_or_default();
    let mut changed: Vec<PathBuf> = files.iter().map(|file| PathBuf::from(file.path().as_ref())).collect();
    changed.sort();
    changed.dedup();
    changed
}

/// What the `changed` files (relative to `repo`'s top) reach in each dbt project they touch.
fn dbt_blasts(repo: &Path, changed: &[PathBuf]) -> Vec<(PathBuf, Blast)> {
    let mut blasts = Vec::new();
    for project in dbt::find_projects(repo) {
        let Ok(folder) = project.strip_prefix(repo) else { continue };
        let files: Vec<PathBuf> =
            changed.iter().filter_map(|file| file.strip_prefix(folder).ok()).map(Path::to_owned).collect();
        if files.is_empty() {
            continue;
        }
        let exposures = dbt_impact::exposures(&project);
        if let Some(blast) = dbt_impact::blast(dbt::load(&project), &exposures, &files) {
            blasts.push((folder.to_owned(), blast));
        }
    }
    blasts
}

/// "2 changed dbt models reach 5 others and 1 exposure."
fn dbt_headline(blasts: &[(PathBuf, Blast)]) -> String {
    let changed: usize = blasts.iter().map(|(_, blast)| blast.changed.len()).sum();
    let downstream: usize = blasts.iter().map(|(_, blast)| blast.downstream.len()).sum();
    let exposures: usize = blasts.iter().map(|(_, blast)| blast.exposures.len()).sum();
    let what = files(changed, "changed dbt model", "changed dbt models");
    let reach = if changed == 1 { "reaches" } else { "reach" };
    match (downstream, exposures) {
        (0, 0) => format!("{what}, and nothing is built from them."),
        (_, 0) => format!("{what} {reach} {}.", files(downstream, "other", "others")),
        _ => format!(
            "{what} {reach} {} and {}.",
            files(downstream, "other", "others"),
            files(exposures, "exposure", "exposures")
        ),
    }
}

/// "1 changed file is" / "3 changed files are".
fn files(count: usize, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}

fn name(file: &Path) -> String {
    file.file_name().map_or_else(|| file.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn folder(file: &Path) -> String {
    file.parent().map(|parent| parent.display().to_string()).unwrap_or_default()
}

/// A list's section: its name and how many files are in it.
fn section_header(title: &str, count: usize) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(len(space::SPACE_2))
        .px(len(space::SPACE_4))
        .pt(len(space::SPACE_3))
        .pb(len(space::SPACE_1))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(div().text_color(theme::color(color::INK_MUTED)).child(title.to_owned()))
        .child(div().text_color(theme::color(color::INK_FAINT)).child(count.to_string()))
}

impl Render for ImpactView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.focus.clone() {
            Some(file) => self.render_focus(&file, cx),
            None => self.render_impact(cx),
        }
    }
}
