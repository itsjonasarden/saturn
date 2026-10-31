//! Which files use which, for the impact view: every JavaScript and TypeScript file's imports,
//! resolved to files the way the TypeScript compiler would (relative paths, `index` files, `.js`
//! written for `.ts`, `tsconfig.json` path aliases, and a monorepo's own packages), and turned
//! around, so a change can be followed to everything that depends on it.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

mod python;
mod rust;

const SOURCE: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// The import graph of a repository, by paths relative to its top.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Graph {
    /// What each file imports (files of this repository only).
    pub imports: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// What imports each file.
    pub importers: BTreeMap<PathBuf, Vec<PathBuf>>,
}

impl Graph {
    /// Builds the graph from each file's imports (`file` → specifiers), resolving them with `resolve`.
    pub fn from_imports(
        files: &BTreeMap<PathBuf, Vec<String>>,
        resolve: impl Fn(&Path, &str) -> Option<PathBuf>,
    ) -> Self {
        let mut graph = Self::default();
        for (file, specifiers) in files {
            let mut targets: Vec<PathBuf> = specifiers
                .iter()
                .filter_map(|specifier| resolve(file, specifier))
                .filter(|target| target != file && files.contains_key(target))
                .collect();
            targets.sort();
            targets.dedup();
            for target in &targets {
                graph.importers.entry(target.clone()).or_default().push(file.clone());
            }
            graph.imports.insert(file.clone(), targets);
        }
        for importers in graph.importers.values_mut() {
            importers.sort();
        }
        graph
    }

    /// Everything that depends on `changed`, directly or through other files, nearest first: each
    /// with how many steps away it is and which changed file it leads back to.
    pub fn impact(&self, changed: &[PathBuf]) -> Vec<Affected> {
        let changed_set: HashSet<&PathBuf> = changed.iter().collect();
        let mut seen: HashSet<PathBuf> = changed.iter().cloned().collect();
        let mut queue: VecDeque<(PathBuf, usize, PathBuf)> =
            changed.iter().map(|file| (file.clone(), 0, file.clone())).collect();
        let mut affected = Vec::new();
        while let Some((file, depth, origin)) = queue.pop_front() {
            for importer in self.importers.get(&file).into_iter().flatten() {
                if changed_set.contains(importer) || !seen.insert(importer.clone()) {
                    continue;
                }
                affected.push(Affected { file: importer.clone(), depth: depth + 1, because_of: origin.clone() });
                queue.push_back((importer.clone(), depth + 1, origin.clone()));
            }
        }
        affected.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.file.cmp(&b.file)));
        affected
    }
}

/// A file a change reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Affected {
    pub file: PathBuf,
    /// 1: it imports a changed file; 2: it imports one of those; and so on.
    pub depth: usize,
    /// The changed file it leads back to.
    pub because_of: PathBuf,
}

/// The folder a file is grouped under in the impact view: its first two folders
/// (`apps/api/src/x.ts` → `apps/api`), or its first one.
pub fn area(file: &Path) -> String {
    let folders: Vec<String> = file
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    match folders.as_slice() {
        [] => ".".to_owned(),
        [one] => one.clone(),
        [first, second, ..] => format!("{first}/{second}"),
    }
}

/// The module specifiers a JavaScript or TypeScript source imports: `import … from "x"`,
/// `import "x"`, `export … from "x"`, `import("x")` and `require("x")`. Comments are skipped.
pub fn specifiers(source: &str) -> Vec<String> {
    let code = strip_comments(source);
    let mut found = Vec::new();
    let bytes = code.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &code[at..];
        let starts_word = at == 0
            || !(bytes[at - 1].is_ascii_alphanumeric()
                || bytes[at - 1] == b'_'
                || bytes[at - 1] == b'$'
                || bytes[at - 1] == b'.');
        let keyword = ["from", "import", "require"].into_iter().find(|word| starts_word && rest.starts_with(word));
        let Some(keyword) = keyword else {
            at += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let after = rest[keyword.len()..].trim_start();
        let after = if keyword == "from" { after } else { after.strip_prefix('(').map_or(after, str::trim_start) };
        if let Some(quote) = after.chars().next().filter(|c| matches!(c, '"' | '\'' | '`'))
            && let Some(end) = after[1..].find(quote)
        {
            let specifier = &after[1..=end];
            if !specifier.is_empty() && !specifier.contains('$') && !specifier.contains('\n') {
                found.push(specifier.to_owned());
            }
        }
        at += keyword.len();
    }
    found
}

fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut previous = ' ';
                for skipped in chars.by_ref() {
                    if previous == '*' && skipped == '/' {
                        break;
                    }
                    previous = skipped;
                }
                out.push(' ');
            }
            ('"' | '\'' | '`', _) => {
                quote = Some(c);
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// A tsconfig's path aliases: each pattern (`@/*`) with the paths it stands for.
type Aliases = Vec<(String, Vec<PathBuf>)>;

/// How a repository's imports resolve: its `tsconfig.json` path aliases and its own packages.
#[derive(Debug, Default)]
pub struct Resolver {
    root: PathBuf,
    /// Every source file, relative to the root.
    files: HashSet<PathBuf>,
    /// Folders with a `tsconfig.json`, deepest first, with its aliases: (pattern, targets), the
    /// targets relative to the root.
    aliases: Vec<(PathBuf, Aliases)>,
    /// The repository's own packages: name → folder, relative to the root.
    packages: HashMap<String, PathBuf>,
}

impl Resolver {
    pub fn new(root: &Path, files: HashSet<PathBuf>) -> Self {
        let mut resolver = Self { root: root.to_owned(), files, ..Self::default() };
        for entry in walk(root).max_depth(Some(6)).build().flatten() {
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else { continue };
            let folder = relative.parent().unwrap_or(Path::new("")).to_owned();
            match path.file_name().and_then(|name| name.to_str()) {
                Some("tsconfig.json") => {
                    let aliases = tsconfig_aliases(path, root);
                    if !aliases.is_empty() {
                        resolver.aliases.push((folder, aliases));
                    }
                }
                Some("package.json") => {
                    if let Some(name) = read_json(path).and_then(|json| json.get("name")?.as_str().map(str::to_owned)) {
                        resolver.packages.insert(name, folder);
                    }
                }
                _ => {}
            }
        }
        resolver.aliases.sort_by_key(|(folder, _)| std::cmp::Reverse(folder.components().count()));
        resolver
    }

    /// The repository file `specifier` means when imported from `from`, if it's one of ours.
    pub fn resolve(&self, from: &Path, specifier: &str) -> Option<PathBuf> {
        if specifier.starts_with('.') {
            let base = from.parent().unwrap_or(Path::new("")).join(specifier);
            return self.file_at(&normalize(&base));
        }
        // The nearest tsconfig's aliases, then the next one up.
        for (folder, aliases) in &self.aliases {
            if !from.starts_with(folder) {
                continue;
            }
            for (pattern, targets) in aliases {
                let matched = match pattern.strip_suffix('*') {
                    Some(prefix) => specifier.strip_prefix(prefix),
                    None => (specifier == pattern).then_some(""),
                };
                let Some(rest) = matched else { continue };
                for target in targets {
                    let candidate = target.to_string_lossy().replace('*', rest);
                    if let Some(found) = self.file_at(&normalize(Path::new(&candidate))) {
                        return Some(found);
                    }
                }
            }
        }
        // One of the repository's own packages: `@acme/shared`, `@acme/shared/dates`.
        let (name, sub) = split_package(specifier);
        let folder = self.packages.get(name)?;
        match sub {
            Some(sub) => self.file_at(&folder.join(sub)).or_else(|| self.file_at(&folder.join("src").join(sub))),
            None => self.package_entry(folder),
        }
    }

    /// A package's main file: what its package.json names, else `src/index`, else `index`.
    fn package_entry(&self, folder: &Path) -> Option<PathBuf> {
        let manifest = read_json(&self.root.join(folder).join("package.json"));
        let named = manifest.as_ref().into_iter().flat_map(|json| {
            ["types", "source", "module", "main"]
                .into_iter()
                .filter_map(|key| json.get(key)?.as_str().map(str::to_owned))
        });
        let candidates: Vec<PathBuf> =
            named.map(PathBuf::from).chain([PathBuf::from("src/index"), PathBuf::from("index")]).collect();
        candidates.iter().find_map(|candidate| self.file_at(&normalize(&folder.join(candidate))))
    }

    /// The source file at `path`: itself, with a source extension added, with `.js` swapped for
    /// `.ts`, or its folder's `index`.
    fn file_at(&self, path: &Path) -> Option<PathBuf> {
        if self.files.contains(path) {
            return Some(path.to_owned());
        }
        let stem = match path.extension().and_then(|ext| ext.to_str()) {
            Some("js" | "jsx" | "mjs" | "cjs") => path.with_extension(""),
            _ => path.to_owned(),
        };
        for ext in SOURCE {
            let with_ext = PathBuf::from(format!("{}.{ext}", stem.display()));
            if self.files.contains(&with_ext) {
                return Some(with_ext);
            }
        }
        SOURCE.iter().map(|ext| path.join(format!("index.{ext}"))).find(|index| self.files.contains(index))
    }
}

fn split_package(specifier: &str) -> (&str, Option<&str>) {
    let mut parts = specifier.splitn(3, '/');
    let first = parts.next().unwrap_or_default();
    if first.starts_with('@') {
        let second = parts.next();
        let name_len = first.len() + second.map_or(0, |s| s.len() + 1);
        (&specifier[..name_len], parts.next())
    } else {
        let rest = specifier.split_once('/').map(|(_, rest)| rest);
        (first, rest)
    }
}

/// `a/./b/../c` → `a/c`, without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&strip_json_comments(&text)).ok()
}

/// tsconfig files allow comments and trailing commas; JSON doesn't.
fn strip_json_comments(text: &str) -> String {
    let without = strip_comments(text);
    let mut out = String::with_capacity(without.len());
    let chars: Vec<char> = without.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        out.push(*c);
    }
    out
}

/// A tsconfig's `paths`, resolved against its `baseUrl` (or itself), following `extends` to
/// files of the repository.
fn tsconfig_aliases(path: &Path, root: &Path) -> Aliases {
    let mut current = path.to_owned();
    for _ in 0..4 {
        let Some(json) = read_json(&current) else { return Vec::new() };
        let options = json.get("compilerOptions");
        if let Some(paths) = options.and_then(|o| o.get("paths")).and_then(Value::as_object) {
            let folder = current.parent().unwrap_or(root);
            let base = options
                .and_then(|o| o.get("baseUrl"))
                .and_then(Value::as_str)
                .map_or(folder.to_owned(), |b| folder.join(b));
            return paths
                .iter()
                .map(|(pattern, targets)| {
                    let targets = targets
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|target| {
                            normalize(&base.join(target)).strip_prefix(root).map(Path::to_path_buf).unwrap_or_default()
                        })
                        .collect();
                    (pattern.clone(), targets)
                })
                .collect();
        }
        let Some(parent) = json.get("extends").and_then(Value::as_str).filter(|e| e.starts_with('.')) else {
            return Vec::new();
        };
        let next = normalize(&current.parent().unwrap_or(root).join(parent));
        current = if next.extension().is_some() { next } else { next.with_extension("json") };
    }
    Vec::new()
}

/// A walk of `root` that skips what git ignores (even before `git init`) and `node_modules`.
fn walk(root: &Path) -> ignore::WalkBuilder {
    let mut walk = ignore::WalkBuilder::new(root);
    walk.require_git(false).filter_entry(|entry| entry.file_name() != "node_modules");
    walk
}

/// The languages whose imports the graph follows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Script,
    Rust,
    Python,
}

fn language(path: &Path) -> Option<Language> {
    match path.extension()?.to_str()? {
        ext if SOURCE.contains(&ext) && !path.to_string_lossy().ends_with(".d.ts") => Some(Language::Script),
        "rs" => Some(Language::Rust),
        "py" => Some(Language::Python),
        _ => None,
    }
}

/// Whether the graph follows imports in `path`'s language: JavaScript, TypeScript, Rust, Python.
pub fn follows(path: &Path) -> bool {
    language(path).is_some()
}

/// Reads every JavaScript, TypeScript, Rust and Python file in `root` (gitignored folders such as
/// `node_modules` and `target` skipped) and builds its import graph.
pub fn build(root: &Path) -> Graph {
    let mut sources: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for entry in walk(root).build().flatten() {
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else { continue };
        let Some(language) = language(path) else { continue };
        if let Ok(text) = std::fs::read_to_string(path) {
            let found = match language {
                Language::Script => specifiers(&text),
                Language::Rust => rust::paths(&text),
                Language::Python => python::imports(&text),
            };
            sources.insert(relative.to_owned(), found);
        }
    }
    let of = |wanted: Language| -> Vec<PathBuf> {
        sources.keys().filter(|file| language(file) == Some(wanted)).cloned().collect()
    };
    let scripts = Resolver::new(root, of(Language::Script).into_iter().collect());
    let crates = rust::Crates::new(root, &of(Language::Rust));
    let modules = python::Modules::new(&of(Language::Python));
    Graph::from_imports(&sources, |from, specifier| match language(from)? {
        Language::Script => scripts.resolve(from, specifier),
        Language::Rust => crates.resolve(from, specifier),
        Language::Python => modules.resolve(from, specifier),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_kind_of_import() {
        let source = r#"
            import React, { useState } from "react";
            import type { Order } from './types';
            import './styles.css';
            export { total } from "../money";
            export * from '@acme/shared/dates';
            const lazy = await import("./lazy");
            const old = require('./legacy.js');
            // import { gone } from "./commented-out";
            /* import "./block-commented"; */
            const text = "import nothing from here";
            const route = { from: 'a-word-not-an-import' };
        "#;
        assert_eq!(
            specifiers(source),
            ["react", "./types", "./styles.css", "../money", "@acme/shared/dates", "./lazy", "./legacy.js"]
        );
    }

    fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        for (path, text) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().expect("a folder")).expect("folders");
            std::fs::write(full, text).expect("written");
        }
        dir
    }

    #[test]
    fn resolves_relative_index_js_for_ts_aliases_and_packages() {
        let dir = repo(&[
            (
                "tsconfig.json",
                r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["apps/web/src/*"], }, }, // aliases
            }"#,
            ),
            ("package.json", r#"{ "name": "root" }"#),
            ("packages/shared/package.json", r#"{ "name": "@acme/shared", "main": "dist/index.js" }"#),
            ("packages/shared/src/index.ts", "export * from './dates';"),
            ("packages/shared/src/dates.ts", "export const d = 1;"),
            ("apps/web/src/money/index.ts", "export const m = 1;"),
            (
                "apps/web/src/orders.ts",
                "import { m } from './money'; import { d } from '@acme/shared'; import '@/money';",
            ),
            ("apps/web/src/server.ts", "import './orders.js'; import { d } from '@acme/shared/dates'; import 'react';"),
            ("node_modules/react/index.js", "module.exports = 1;"),
            (".gitignore", "node_modules\n"),
        ]);
        let graph = build(dir.path());
        let imports = |file: &str| graph.imports.get(Path::new(file)).cloned().unwrap_or_default();
        assert_eq!(
            imports("apps/web/src/orders.ts"),
            [PathBuf::from("apps/web/src/money/index.ts"), PathBuf::from("packages/shared/src/index.ts")]
        );
        assert_eq!(
            imports("apps/web/src/server.ts"),
            [PathBuf::from("apps/web/src/orders.ts"), PathBuf::from("packages/shared/src/dates.ts")]
        );
        assert!(!graph.imports.contains_key(Path::new("node_modules/react/index.js")));
    }

    #[test]
    fn a_change_reaches_its_importers_nearest_first() {
        let files: BTreeMap<PathBuf, Vec<String>> = [
            ("a.ts", vec![]),
            ("b.ts", vec!["./a"]),
            ("c.ts", vec!["./b"]),
            ("d.ts", vec!["./c", "./a"]),
            ("e.ts", vec![]),
        ]
        .into_iter()
        .map(|(file, imports)| (PathBuf::from(file), imports.into_iter().map(str::to_owned).collect()))
        .collect();
        let graph = Graph::from_imports(&files, |from, specifier| {
            let target =
                from.parent().unwrap_or(Path::new("")).join(format!("{}.ts", specifier.trim_start_matches("./")));
            Some(normalize(&target))
        });
        let impact = graph.impact(&[PathBuf::from("a.ts")]);
        let summary: Vec<(&str, usize)> =
            impact.iter().map(|a| (a.file.to_str().unwrap_or_default(), a.depth)).collect();
        assert_eq!(summary, [("b.ts", 1), ("d.ts", 1), ("c.ts", 2)]);
        assert!(impact.iter().all(|a| a.because_of == Path::new("a.ts")));
    }

    #[test]
    fn groups_files_by_their_first_two_folders() {
        assert_eq!(area(Path::new("apps/api/src/orders.ts")), "apps/api");
        assert_eq!(area(Path::new("scripts/build.ts")), "scripts");
        assert_eq!(area(Path::new("index.ts")), ".");
    }
}
