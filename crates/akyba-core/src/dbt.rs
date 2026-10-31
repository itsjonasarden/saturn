//! A dbt project's lineage, for the lineage graph: its models, the sources, seeds and snapshots
//! they read, and which builds on which. Read from the project's files (`ref()` and `source()` in
//! each model), so neither dbt nor a warehouse is needed. Then laid out left to right, sources
//! first, each model one column past the furthest thing it reads.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Source,
    Seed,
    Snapshot,
    Model,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Seed => "seed",
            Self::Snapshot => "snapshot",
            Self::Model => "model",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// As dbt names it: a model's file name, or `source.table` for a source.
    pub name: String,
    pub kind: Kind,
    /// The file that defines it, inside the dbt project; sources have none.
    pub path: Option<PathBuf>,
    /// The folder it's in under its kind's folder (`staging`, `marts/finance`), for its colour band.
    pub folder: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lineage {
    /// The folder holding `dbt_project.yml`.
    pub root: PathBuf,
    pub name: String,
    pub nodes: Vec<Node>,
    /// `(from, to)`: `to` reads `from`. Indices into `nodes`.
    pub edges: Vec<(usize, usize)>,
}

/// The dbt projects in a repository: folders with a `dbt_project.yml`, outside installed packages.
pub fn find_projects(repo: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = ignore::WalkBuilder::new(repo)
        .max_depth(Some(4))
        .filter_entry(|entry| !matches!(entry.file_name().to_str(), Some("dbt_packages" | "node_modules" | "target")))
        .build()
        .flatten()
        .filter(|entry| entry.file_name() == "dbt_project.yml")
        .filter_map(|entry| entry.path().parent().map(Path::to_owned))
        .collect();
    found.sort();
    found
}

/// A top-level list setting in `dbt_project.yml` (`model-paths: ["models"]`, or a YAML list under
/// it), else `default`.
fn paths_setting(project_yml: &str, key: &str, default: &str) -> Vec<String> {
    let unquote = |text: &str| text.trim().trim_matches(|c| c == '"' || c == '\'').to_owned();
    let mut lines = project_yml.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix(key).and_then(|rest| rest.trim_start().strip_prefix(':')) else { continue };
        let rest = rest.trim();
        if let Some(inline) = rest.strip_prefix('[').and_then(|rest| rest.split(']').next()) {
            return inline.split(',').map(unquote).filter(|path| !path.is_empty()).collect();
        }
        if !rest.is_empty() {
            return vec![unquote(rest)];
        }
        let items: Vec<String> = lines
            .clone()
            .take_while(|line| line.trim_start().starts_with('-') || line.trim().is_empty())
            .filter_map(|line| line.trim_start().strip_prefix('-').map(unquote))
            .collect();
        if !items.is_empty() {
            return items;
        }
    }
    vec![default.to_owned()]
}

/// The project's `name:`.
fn project_name(project_yml: &str) -> Option<String> {
    project_yml.lines().find_map(|line| {
        line.strip_prefix("name:").map(|name| name.trim().trim_matches(|c| c == '"' || c == '\'').to_owned())
    })
}

/// SQL and Jinja with comments taken out, so a `ref()` in a comment draws no line.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["{#", "/*", "--"]
            .iter()
            .filter_map(|open| rest.find(open).map(|at| (at, *open)))
            .min_by_key(|(at, _)| *at);
        let Some((at, open)) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at]);
        let close = match open {
            "{#" => "#}",
            "/*" => "*/",
            _ => "\n",
        };
        rest = rest[at + open.len()..].find(close).map_or("", |end| &rest[at + open.len() + end + close.len()..]);
        if close == "\n" {
            out.push('\n');
        }
    }
    out
}

/// The quoted arguments of each `call(` in `text`: `ref('a')` → `["a"]`, `source('s', 't')` →
/// `["s", "t"]`. Keyword arguments (`v=2`) are left out.
fn calls(text: &str, call: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(call) {
        let before = rest[..at].chars().next_back();
        rest = &rest[at + call.len()..];
        // `myref(` or `dbt_ref(` isn't a call to `ref(`; `dbt.ref(` in a Python model is.
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some(args) = rest.trim_start().strip_prefix('(').and_then(|args| args.split(')').next()) else { continue };
        let strings: Vec<String> = args
            .split(',')
            .map(str::trim)
            .filter(|arg| !arg.contains('='))
            .filter_map(|arg| {
                let quote = arg.chars().next().filter(|c| *c == '\'' || *c == '"')?;
                arg.strip_prefix(quote)?.strip_suffix(quote).map(str::to_owned)
            })
            .collect();
        if !strings.is_empty() {
            found.push(strings);
        }
    }
    found
}

/// What a model reads: the models it `ref`s and the sources it reads, as `source.table`.
pub fn reads(sql: &str) -> (Vec<String>, Vec<String>) {
    let sql = without_comments(sql);
    let refs = calls(&sql, "ref").into_iter().filter_map(|args| args.last().cloned()).collect();
    let sources = calls(&sql, "source")
        .into_iter()
        .filter(|args| args.len() == 2)
        .map(|args| format!("{}.{}", args[0], args[1]))
        .collect();
    (refs, sources)
}

/// The name in a snapshot block: `{% snapshot orders_snapshot %}`.
fn snapshot_names(sql: &str) -> Vec<String> {
    sql.split("{%")
        .filter_map(|block| block.trim_start().trim_start_matches('-').trim_start().strip_prefix("snapshot "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|name| name.trim_end_matches(['%', '-']).to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Adds `node`, or finds it if it's there: a source by its name, anything else by its name among
/// the things `ref()` can name.
fn add(lineage: &mut Lineage, index: &mut HashMap<(Kind, String), usize>, node: Node) -> usize {
    let key = (if node.kind == Kind::Source { Kind::Source } else { Kind::Model }, node.name.clone());
    *index.entry(key).or_insert_with(|| {
        lineage.nodes.push(node);
        lineage.nodes.len() - 1
    })
}

/// Reads the lineage of the dbt project in `root`.
pub fn load(root: &Path) -> Lineage {
    let project_yml = std::fs::read_to_string(root.join("dbt_project.yml")).unwrap_or_default();
    let mut lineage = Lineage {
        root: root.to_owned(),
        name: project_name(&project_yml)
            .unwrap_or_else(|| root.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())),
        ..Lineage::default()
    };
    let mut index: HashMap<(Kind, String), usize> = HashMap::new();
    let files = |setting: &str, default: &str, extensions: &[&str]| -> Vec<(PathBuf, String)> {
        let mut files = Vec::new();
        for folder in paths_setting(&project_yml, setting, default) {
            let base = root.join(&folder);
            for entry in ignore::WalkBuilder::new(&base).build().flatten() {
                let path = entry.path();
                let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
                if entry.file_type().is_some_and(|kind| kind.is_file()) && extensions.contains(&extension) {
                    let folder = path
                        .parent()
                        .and_then(|parent| parent.strip_prefix(&base).ok())
                        .map(|folder| folder.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    files.push((path.to_owned(), folder));
                }
            }
        }
        files.sort();
        files
    };
    let relative = |path: &Path| path.strip_prefix(root).unwrap_or(path).to_owned();
    // Seeds and snapshots first, so a model that refs them finds them.
    for (path, folder) in files("seed-paths", "seeds", &["csv"]) {
        let name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        add(&mut lineage, &mut index, Node { name, kind: Kind::Seed, path: Some(relative(&path)), folder });
    }
    let mut pending: Vec<(usize, String)> = Vec::new();
    for (path, folder) in files("snapshot-paths", "snapshots", &["sql"]) {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for name in snapshot_names(&text) {
            let node = add(
                &mut lineage,
                &mut index,
                Node { name, kind: Kind::Snapshot, path: Some(relative(&path)), folder: folder.clone() },
            );
            pending.push((node, text.clone()));
        }
    }
    for (path, folder) in files("model-paths", "models", &["sql", "py"]) {
        let name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let node = add(&mut lineage, &mut index, Node { name, kind: Kind::Model, path: Some(relative(&path)), folder });
        pending.push((node, text));
    }
    let mut edges = HashSet::new();
    for (node, text) in pending {
        let (refs, sources) = reads(&text);
        for name in refs {
            if let Some(&from) = index.get(&(Kind::Model, name)) {
                edges.insert((from, node));
            }
        }
        for name in sources {
            let folder = name.split('.').next().unwrap_or_default().to_owned();
            let from = add(&mut lineage, &mut index, Node { name, kind: Kind::Source, path: None, folder });
            edges.insert((from, node));
        }
    }
    lineage.edges = edges.into_iter().filter(|(from, to)| from != to).collect();
    lineage.edges.sort_unstable();
    lineage
}

/// Where a node's box goes, in diagram units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
}

pub const NODE_WIDTH: f32 = 220.;
pub const NODE_HEIGHT: f32 = 44.;
const COLUMN_GAP: f32 = 96.;
const ROW_GAP: f32 = 14.;

/// Lays the graph out left to right: each node one column past the furthest thing it reads, so
/// sources and seeds start on the left and the final models end on the right. Within a column,
/// nodes sit near the middle of what they read, so lines run as straight as they can.
#[expect(clippy::cast_precision_loss, reason = "rows and columns of a graph that fits on screen")]
pub fn layout(lineage: &Lineage) -> Vec<Placed> {
    let count = lineage.nodes.len();
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &(from, to) in &lineage.edges {
        parents[to].push(from);
        children[from].push(to);
    }
    // The column: the longest path from a node with no parents. A cycle (which dbt refuses)
    // stops at the column count rather than looping.
    let mut column = vec![0usize; count];
    for _ in 0..count {
        let mut changed = false;
        for &(from, to) in &lineage.edges {
            if column[to] < column[from] + 1 && column[from] + 1 < count {
                column[to] = column[from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // What reads nothing (a source, a seed) sits just left of its first reader, not at the far
    // left: a seed only a mart uses would otherwise draw a line across every column between.
    for node in 0..count {
        if parents[node].is_empty()
            && let Some(first) = children[node].iter().map(|&child| column[child]).min()
        {
            column[node] = first.saturating_sub(1);
        }
    }
    let columns = column.iter().copied().max().map_or(0, |max| max + 1);
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); columns];
    for node in 0..count {
        order[column[node]].push(node);
    }
    // First by folder then name, so each band stays together; then a few passes pulling each
    // node toward the middle of its parents (left to right) and children (right to left).
    for nodes in &mut order {
        nodes.sort_by(|&a, &b| {
            (&lineage.nodes[a].folder, &lineage.nodes[a].name).cmp(&(&lineage.nodes[b].folder, &lineage.nodes[b].name))
        });
    }
    let mut row = vec![0f32; count];
    let renumber = |order: &[Vec<usize>], row: &mut [f32]| {
        for nodes in order {
            for (at, &node) in nodes.iter().enumerate() {
                row[node] = at as f32;
            }
        }
    };
    renumber(&order, &mut row);
    for pass in 0..6 {
        let forward = pass % 2 == 0;
        let columns_in_turn: Vec<usize> =
            if forward { (1..columns).collect() } else { (0..columns.saturating_sub(1)).rev().collect() };
        for c in columns_in_turn {
            let near = |node: usize| if forward { &parents[node] } else { &children[node] };
            let mut keyed: Vec<(f32, usize)> = order[c]
                .iter()
                .map(|&node| {
                    let others = near(node);
                    let key = if others.is_empty() {
                        row[node]
                    } else {
                        others.iter().map(|&other| row[other]).sum::<f32>() / others.len() as f32
                    };
                    (key, node)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            order[c] = keyed.into_iter().map(|(_, node)| node).collect();
            renumber(&order, &mut row);
        }
    }
    // Columns are centred on the tallest, so a short column sits in the middle of the page.
    let tallest = order.iter().map(Vec::len).max().unwrap_or(0) as f32;
    let mut placed = vec![Placed { x: 0., y: 0. }; count];
    for (c, nodes) in order.iter().enumerate() {
        let offset = (tallest - nodes.len() as f32) * (NODE_HEIGHT + ROW_GAP) / 2.;
        for (at, &node) in nodes.iter().enumerate() {
            placed[node] =
                Placed { x: c as f32 * (NODE_WIDTH + COLUMN_GAP), y: offset + at as f32 * (NODE_HEIGHT + ROW_GAP) };
        }
    }
    placed
}

/// How many nodes a graph can have and still be drawn whole; past it, one model's neighbourhood.
pub const WHOLE_GRAPH_LIMIT: usize = 150;

/// The nodes within `depth` steps of `center`, upstream and downstream (every step, for `None`),
/// `center` first.
pub fn neighbourhood(lineage: &Lineage, center: usize, depth: Option<usize>) -> Vec<usize> {
    let mut found = vec![center];
    let mut seen = vec![false; lineage.nodes.len()];
    seen[center] = true;
    for upstream in [true, false] {
        let mut frontier = vec![center];
        let mut step = 0;
        while !frontier.is_empty() && depth.is_none_or(|depth| step < depth) {
            let mut next = Vec::new();
            for &at in &frontier {
                for &(from, to) in &lineage.edges {
                    let (here, other) = if upstream { (to, from) } else { (from, to) };
                    if here == at && !seen[other] {
                        seen[other] = true;
                        found.push(other);
                        next.push(other);
                    }
                }
            }
            frontier = next;
            step += 1;
        }
    }
    found
}

/// The part of the lineage made of `nodes`, with the edges between them; indices are `nodes`' order.
pub fn subgraph(lineage: &Lineage, nodes: &[usize]) -> Lineage {
    let mut position = vec![None; lineage.nodes.len()];
    for (at, &node) in nodes.iter().enumerate() {
        position[node] = Some(at);
    }
    Lineage {
        root: lineage.root.clone(),
        name: lineage.name.clone(),
        nodes: nodes.iter().map(|&node| lineage.nodes[node].clone()).collect(),
        edges: lineage.edges.iter().filter_map(|&(from, to)| Some((position[from]?, position[to]?))).collect(),
    }
}

/// Everything upstream of `node` (what it's built from) and downstream (what's built from it).
pub fn related(lineage: &Lineage, node: usize) -> Vec<bool> {
    let mut related = vec![false; lineage.nodes.len()];
    related[node] = true;
    for upstream in [true, false] {
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            for &(from, to) in &lineage.edges {
                let (here, next) = if upstream { (to, from) } else { (from, to) };
                if here == at && !related[next] {
                    related[next] = true;
                    stack.push(next);
                }
            }
        }
    }
    related
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_refs_and_sources_but_not_in_comments() {
        let sql = "-- from {{ ref('old_orders') }}\n{# {{ ref('draft') }} #}\nselect * from {{ ref('stg_orders') }}\n\
                   join {{ ref(\"pkg\", 'stg_items', v=2) }} join {{ source('shop', 'customers') }}\n/* ref('gone') */";
        let (refs, sources) = reads(sql);
        assert_eq!(refs, ["stg_orders", "stg_items"]);
        assert_eq!(sources, ["shop.customers"]);
        let (refs, _) = reads("orders = dbt.ref(\"fct_orders\")\nx = myref('nope')");
        assert_eq!(refs, ["fct_orders"]);
    }

    #[test]
    fn reads_the_project_settings_and_snapshot_names() {
        let yml = "name: 'acme'\nmodel-paths: [\"models\", \"more\"]\nseed-paths:\n  - data\n";
        assert_eq!(project_name(yml).as_deref(), Some("acme"));
        assert_eq!(paths_setting(yml, "model-paths", "models"), ["models", "more"]);
        assert_eq!(paths_setting(yml, "seed-paths", "seeds"), ["data"]);
        assert_eq!(paths_setting(yml, "snapshot-paths", "snapshots"), ["snapshots"]);
        assert_eq!(snapshot_names("{% snapshot orders_snapshot %} select 1 {% endsnapshot %}"), ["orders_snapshot"]);
    }

    #[test]
    fn loads_a_project_and_lays_it_out_left_to_right() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let root = dir.path();
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("folders");
            std::fs::write(path, text).expect("written");
        };
        write("dbt_project.yml", "name: shop\n");
        write("seeds/rates.csv", "a\n1\n");
        write("models/staging/stg_orders.sql", "select * from {{ source('raw', 'orders') }}");
        write("models/marts/fct_orders.sql", "select * from {{ ref('stg_orders') }} join {{ ref('rates') }}");
        write("models/marts/report.sql", "select * from {{ ref('fct_orders') }}");
        let lineage = load(root);
        assert_eq!(lineage.name, "shop");
        let at = |name: &str| lineage.nodes.iter().position(|node| node.name == name).expect(name);
        assert_eq!(lineage.nodes[at("raw.orders")].kind, Kind::Source);
        assert_eq!(lineage.nodes[at("rates")].kind, Kind::Seed);
        assert_eq!(lineage.nodes[at("stg_orders")].folder, "staging");
        assert_eq!(lineage.edges.len(), 4);
        let placed = layout(&lineage);
        assert!(placed[at("raw.orders")].x < placed[at("stg_orders")].x);
        assert!(placed[at("stg_orders")].x < placed[at("fct_orders")].x);
        assert!(placed[at("fct_orders")].x < placed[at("report")].x);
        let related = related(&lineage, at("stg_orders"));
        assert!(related[at("raw.orders")] && related[at("report")] && !related[at("rates")]);
        let near = neighbourhood(&lineage, at("stg_orders"), Some(1));
        assert_eq!(near.len(), 3, "stg_orders, what it reads, what reads it");
        assert_eq!(neighbourhood(&lineage, at("stg_orders"), None).len(), 4);
        let part = subgraph(&lineage, &near);
        assert_eq!((part.nodes[0].name.as_str(), part.edges.len()), ("stg_orders", 2));
        write("dbt_packages/utils/dbt_project.yml", "name: utils\n");
        assert_eq!(find_projects(root), [root.to_owned()]);
    }
}
