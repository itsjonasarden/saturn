//! What a dbt change reaches: the models built from a changed model, seed or snapshot, however many
//! steps away, and the exposures (dashboards, reports, apps) that read any of them. Exposures are
//! read from the project's YAML (`exposures:` blocks, their `depends_on` refs), like the lineage,
//! without dbt or a warehouse.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::dbt::{self, Kind, Lineage};

/// Something outside dbt that reads the project: a dashboard, a report, an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exposure {
    pub name: String,
    /// Its `type:`, as dbt has it: `dashboard`, `notebook`, `analysis`, `ml`, `application`.
    pub kind: String,
    /// The file that declares it, inside the dbt project.
    pub path: PathBuf,
    /// The models (by name) and sources (`source.table`) it depends on.
    pub models: Vec<String>,
    pub sources: Vec<String>,
}

/// The exposures declared anywhere in the project in `root`, outside installed packages.
#[must_use]
pub fn exposures(root: &Path) -> Vec<Exposure> {
    let mut found = Vec::new();
    let walk = ignore::WalkBuilder::new(root)
        .filter_entry(|entry| !matches!(entry.file_name().to_str(), Some("dbt_packages" | "target" | "node_modules")))
        .build();
    for entry in walk.flatten() {
        let path = entry.path();
        let yaml = path.extension().is_some_and(|extension| extension == "yml" || extension == "yaml");
        if !yaml {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        if text.contains("exposures:") {
            found.extend(parse(&text, path.strip_prefix(root).unwrap_or(path)));
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The exposures in one YAML file: each item of its top-level `exposures:` list.
fn parse(yaml: &str, path: &Path) -> Vec<Exposure> {
    let mut lines = yaml.lines().skip_while(|line| line.trim_end() != "exposures:").skip(1);
    // The block ends at the next top-level key.
    let block: Vec<&str> =
        lines.by_ref().take_while(|line| line.trim().is_empty() || line.starts_with([' ', '-', '#'])).collect();
    let Some(item_indent) = block.iter().find(|line| line.trim_start().starts_with('-')).map(|line| indent(line))
    else {
        return Vec::new();
    };
    let mut items: Vec<Vec<&str>> = Vec::new();
    for line in block {
        if indent(line) == item_indent && line.trim_start().starts_with('-') {
            items.push(Vec::new());
        }
        if let Some(item) = items.last_mut() {
            item.push(line);
        }
    }
    items
        .into_iter()
        .filter_map(|item| {
            // Keys of the item itself, not of a nested map (an owner's `name:`).
            let own = |key: &str| {
                item.iter().find_map(|line| {
                    let at = indent(line);
                    let text = line.trim_start().trim_start_matches('-').trim_start();
                    let level = at == item_indent || at == item_indent + 2;
                    let value = text.strip_prefix(key)?.strip_prefix(':')?;
                    level.then(|| value.trim().trim_matches(['"', '\'']).to_owned()).filter(|value| !value.is_empty())
                })
            };
            let name = own("name")?;
            let (models, sources) = dbt::reads(&item.join("\n"));
            Some(Exposure {
                name,
                kind: own("type").unwrap_or_else(|| "exposure".to_owned()),
                path: path.to_owned(),
                models,
                sources,
            })
        })
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// A node the change reaches, and in how many steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reached {
    pub node: usize,
    pub steps: usize,
}

/// What a change to a dbt project reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blast {
    pub lineage: Lineage,
    /// The nodes whose files changed.
    pub changed: Vec<usize>,
    /// Everything built from them, nearest first.
    pub downstream: Vec<Reached>,
    /// The exposures reading any of those.
    pub exposures: Vec<Exposure>,
}

/// What changing `files` (relative to the project's folder) reaches; `None` when none of them
/// defines a model, seed or snapshot.
#[must_use]
pub fn blast(lineage: Lineage, exposures: &[Exposure], files: &[PathBuf]) -> Option<Blast> {
    let changed: Vec<usize> = lineage
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.path.as_ref().is_some_and(|path| files.contains(path)))
        .map(|(index, _)| index)
        .collect();
    if changed.is_empty() {
        return None;
    }
    let mut steps: Vec<Option<usize>> = vec![None; lineage.nodes.len()];
    let mut queue: VecDeque<usize> = changed.iter().copied().collect();
    for &node in &changed {
        steps[node] = Some(0);
    }
    let mut downstream = Vec::new();
    while let Some(at) = queue.pop_front() {
        let here = steps[at].unwrap_or(0);
        for &(from, to) in &lineage.edges {
            if from == at && steps[to].is_none() {
                steps[to] = Some(here + 1);
                downstream.push(Reached { node: to, steps: here + 1 });
                queue.push_back(to);
            }
        }
    }
    let reached = |name: &str, kind: Kind| {
        lineage.nodes.iter().enumerate().any(|(index, node)| {
            steps[index].is_some() && node.name == name && (node.kind == Kind::Source) == (kind == Kind::Source)
        })
    };
    let exposures = exposures
        .iter()
        .filter(|exposure| {
            exposure.models.iter().any(|model| reached(model, Kind::Model))
                || exposure.sources.iter().any(|source| reached(source, Kind::Source))
        })
        .cloned()
        .collect();
    Some(Blast { lineage, changed, downstream, exposures })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPOSURES: &str = "version: 2

exposures:
  - name: weekly_revenue
    type: dashboard
    owner:
      name: Finance team
      email: finance@example.com
    depends_on:
      - ref('fct_orders')
  - name: churn_model
    type: ml
    depends_on:
      - ref(\"dim_customers\")
      - source('stripe', 'payments')

models:
  - name: fct_orders
";

    #[test]
    fn reads_exposures_and_what_they_depend_on() {
        let found = parse(EXPOSURES, Path::new("models/exposures.yml"));
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].name.as_str(), found[0].kind.as_str()), ("weekly_revenue", "dashboard"));
        assert_eq!(found[0].models, ["fct_orders"]);
        assert_eq!(found[1].models, ["dim_customers"]);
        assert_eq!(found[1].sources, ["stripe.payments"]);
        assert_eq!(parse("models:\n  - name: a\n", Path::new("x.yml")), Vec::new());
    }

    #[test]
    fn follows_a_change_downstream_to_the_exposures() {
        let root = tempfile::tempdir().expect("temp dir");
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("folders");
            std::fs::write(path, text).expect("write");
        };
        write("dbt_project.yml", "name: acme\n");
        write("models/staging/stg_orders.sql", "select * from {{ source('shop', 'orders') }}");
        write("models/marts/fct_orders.sql", "select * from {{ ref('stg_orders') }}");
        write("models/marts/fct_revenue.sql", "select * from {{ ref('fct_orders') }}");
        write("models/marts/dim_customers.sql", "select 1");
        write("models/exposures.yml", EXPOSURES);
        let lineage = dbt::load(root.path());
        let exposures = exposures(root.path());
        assert_eq!(exposures.len(), 2);

        let blast = blast(lineage, &exposures, &[PathBuf::from("models/staging/stg_orders.sql")]).expect("a blast");
        let name = |index: usize| blast.lineage.nodes[index].name.as_str();
        assert_eq!(blast.changed.iter().map(|&index| name(index)).collect::<Vec<_>>(), ["stg_orders"]);
        let reached: Vec<(&str, usize)> =
            blast.downstream.iter().map(|reached| (name(reached.node), reached.steps)).collect();
        assert_eq!(reached, [("fct_orders", 1), ("fct_revenue", 2)]);
        assert_eq!(
            blast.exposures.iter().map(|exposure| exposure.name.as_str()).collect::<Vec<_>>(),
            ["weekly_revenue"]
        );
        assert_eq!(super::blast(dbt::load(root.path()), &exposures, &[PathBuf::from("README.md")]), None);
    }
}
