//! Which formatter a project uses for a file, so saving formats it the way the project's own
//! tooling would. Only a project that says so (a formatter config) is formatted, and only with the
//! copy it installed; nothing is fetched or run globally.

use std::path::{Path, PathBuf};

/// A command that formats one file in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatter {
    /// Short name for messages: `oxfmt`, `prettier`, `biome`, `rustfmt`.
    pub name: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Where to run it, so it finds its config.
    pub dir: PathBuf,
}

/// File types the JavaScript-world formatters handle.
const WEB: &[&str] = &[
    "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "json", "jsonc", "css", "scss", "less", "md", "mdx", "yaml",
    "yml", "html", "vue", "svelte", "graphql", "toml",
];

/// The formatter for `file` in the project rooted at `root`, if the project has one.
pub fn formatter_for(file: &Path, root: &Path) -> Option<Formatter> {
    let extension = file.extension()?.to_str()?.to_ascii_lowercase();
    let file_arg = file.to_string_lossy().into_owned();
    if extension == "rs" {
        let dir = ancestors(file, root).find(|dir| has_any(dir, &["rustfmt.toml", ".rustfmt.toml"]))?;
        let edition = rust_edition(file, root).unwrap_or_else(|| "2021".to_owned());
        return Some(Formatter {
            name: "rustfmt",
            program: PathBuf::from("rustfmt"),
            args: vec!["--edition".to_owned(), edition, file_arg],
            dir,
        });
    }
    if !WEB.contains(&extension.as_str()) {
        return None;
    }
    // The nearest config wins: a package inside a monorepo may use something else than its root.
    for dir in ancestors(file, root) {
        let tool = if has_any(&dir, &[".oxfmtrc.json", ".oxfmtrc.jsonc"]) {
            Some(("oxfmt", vec![file_arg.clone()]))
        } else if has_any(&dir, &["biome.json", "biome.jsonc"]) {
            Some(("biome", vec!["format".to_owned(), "--write".to_owned(), file_arg.clone()]))
        } else if has_prettier_config(&dir) {
            Some(("prettier", vec!["--write".to_owned(), file_arg.clone()]))
        } else {
            None
        };
        if let Some((name, args)) = tool {
            let program = ancestors(&dir.join("x"), root)
                .map(|up| up.join("node_modules/.bin").join(name))
                .find(|bin| bin.is_file())?;
            return Some(Formatter { name, program, args, dir });
        }
    }
    None
}

/// `file`'s folder and its parents, up to and including `root`.
fn ancestors<'a>(file: &'a Path, root: &'a Path) -> impl Iterator<Item = PathBuf> + 'a {
    file.ancestors().skip(1).take_while(move |dir| dir.starts_with(root)).map(Path::to_owned)
}

fn has_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| dir.join(name).is_file())
}

fn has_prettier_config(dir: &Path) -> bool {
    let named = [
        ".prettierrc",
        ".prettierrc.json",
        ".prettierrc.yaml",
        ".prettierrc.yml",
        ".prettierrc.js",
        ".prettierrc.cjs",
        ".prettierrc.mjs",
        ".prettierrc.toml",
        "prettier.config.js",
        "prettier.config.cjs",
        "prettier.config.mjs",
        "prettier.config.ts",
    ];
    has_any(dir, &named)
        || std::fs::read_to_string(dir.join("package.json")).is_ok_and(|package| package.contains("\"prettier\":"))
}

/// The Rust edition of the crate `file` belongs to, following `edition.workspace = true` up to the
/// workspace's `Cargo.toml`.
fn rust_edition(file: &Path, root: &Path) -> Option<String> {
    let mut inherited = false;
    for dir in ancestors(file, root) {
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else { continue };
        for line in manifest.lines().map(str::trim) {
            if line.starts_with("edition.workspace") || line.starts_with("edition = { workspace") {
                inherited = true;
            } else if let Some(value) =
                line.strip_prefix("edition").map(str::trim).and_then(|rest| rest.strip_prefix('='))
            {
                let value = value.trim().trim_matches('"');
                if !value.is_empty() && (!inherited || manifest.contains("[workspace.package]")) {
                    return Some(value.to_owned());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn touch(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("a parent")).expect("create a folder");
        fs::write(path, text).expect("write a file");
    }

    #[test]
    fn a_project_without_a_formatter_config_is_left_alone() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("src/orders.ts");
        touch(&file, "");
        touch(&dir.path().join("node_modules/.bin/prettier"), "");
        assert_eq!(formatter_for(&file, dir.path()), None);
    }

    #[test]
    fn oxfmt_runs_from_the_projects_own_node_modules() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("apps/api/orders.ts");
        touch(&file, "");
        touch(&dir.path().join(".oxfmtrc.json"), "{}");
        touch(&dir.path().join("node_modules/.bin/oxfmt"), "");
        let formatter = formatter_for(&file, dir.path()).expect("oxfmt");
        assert_eq!(formatter.name, "oxfmt");
        assert_eq!(formatter.program, dir.path().join("node_modules/.bin/oxfmt"));
        assert_eq!(formatter.dir, dir.path());
    }

    #[test]
    fn a_config_without_the_installed_tool_formats_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("orders.ts");
        touch(&file, "");
        touch(&dir.path().join(".prettierrc"), "{}");
        assert_eq!(formatter_for(&file, dir.path()), None);
    }

    #[test]
    fn the_nearest_config_wins_and_prettier_can_live_in_package_json() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("packages/ui/button.tsx");
        touch(&file, "");
        touch(&dir.path().join(".oxfmtrc.json"), "{}");
        touch(&dir.path().join("packages/ui/package.json"), r#"{ "prettier": {} }"#);
        touch(&dir.path().join("node_modules/.bin/prettier"), "");
        touch(&dir.path().join("node_modules/.bin/oxfmt"), "");
        assert_eq!(formatter_for(&file, dir.path()).map(|f| f.name), Some("prettier"));
    }

    #[test]
    fn rustfmt_uses_the_workspace_edition() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("crates/app/src/main.rs");
        touch(&file, "");
        touch(&dir.path().join("rustfmt.toml"), "max_width = 120\n");
        touch(&dir.path().join("Cargo.toml"), "[workspace.package]\nedition = \"2024\"\n");
        touch(&dir.path().join("crates/app/Cargo.toml"), "[package]\nedition.workspace = true\n");
        let formatter = formatter_for(&file, dir.path()).expect("rustfmt");
        assert_eq!(formatter.args[..2], ["--edition".to_owned(), "2024".to_owned()]);
    }

    #[test]
    fn files_no_formatter_handles_are_left_alone() {
        let dir = tempfile::tempdir().expect("a temp dir");
        touch(&dir.path().join(".oxfmtrc.json"), "{}");
        touch(&dir.path().join("node_modules/.bin/oxfmt"), "");
        let file = dir.path().join(".env");
        touch(&file, "PORT=4000\n");
        assert_eq!(formatter_for(&file, dir.path()), None);
    }
}
