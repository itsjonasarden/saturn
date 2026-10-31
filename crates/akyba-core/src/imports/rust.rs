//! Rust's imports: every file's module path (`src/a/b.rs` is `crate::a::b` of its package), and
//! the paths a file names (`use` trees expanded, and paths written inline) resolved to the module
//! file they reach. The repository's own crates resolve by name: `akyba_core::lsp::path` is
//! `crates/akyba-core/src/lsp.rs`.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// The paths a Rust source names, as `a::b::c`: its `use` trees expanded, then every path written
/// inline that starts with a lowercase name (a crate or module; `Vec::new` names neither).
pub fn paths(source: &str) -> Vec<String> {
    let code = strip(source);
    let mut found = Vec::new();
    let mut rest = code.as_str();
    while let Some(at) = find_word(rest, "use") {
        let after = &rest[at + 3..];
        let Some(end) = after.find(';') else { break };
        expand(after[..end].trim().trim_start_matches("::"), "", &mut found);
        rest = &after[end..];
    }
    found.extend(inline_paths(&code));
    // `use super::*` is a test module reaching into its own file, not the parent module.
    found.retain(|path| path != "super");
    found.sort();
    found.dedup();
    found
}

/// Where `word` next starts as a whole word.
fn find_word(text: &str, word: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let is_ident = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut from = 0;
    while let Some(at) = text[from..].find(word).map(|at| at + from) {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + word.len()).copied();
        if !before.is_some_and(is_ident) && !after.is_some_and(is_ident) {
            return Some(at);
        }
        from = at + word.len();
    }
    None
}

/// A `use` tree's paths: `a::{b, c::{self, d as e}, f::*}` gives `a::b`, `a::c`, `a::c::d`, `a::f`.
fn expand(tree: &str, prefix: &str, out: &mut Vec<String>) {
    let tree = tree.trim();
    let join = |head: &str| match (prefix.is_empty(), head.is_empty()) {
        (true, _) => head.to_owned(),
        (false, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}::{head}"),
    };
    if let (Some(open), Some(close)) = (tree.find('{'), tree.rfind('}'))
        && open < close
    {
        let base = join(tree[..open].trim().trim_end_matches("::").trim());
        for part in split_top_level(&tree[open + 1..close]) {
            expand(part, &base, out);
        }
        return;
    }
    let path = tree.split(" as ").next().unwrap_or_default().trim();
    let path = path.trim_end_matches('*').trim_end_matches("::");
    let full = if path == "self" { prefix.to_owned() } else { join(path) };
    if !full.is_empty() {
        out.push(full.split_whitespace().collect());
    }
}

/// The parts of a brace group, split at commas outside nested braces.
fn split_top_level(inner: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0, 0);
    for (at, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts.into_iter().filter(|part| !part.trim().is_empty()).collect()
}

/// Paths written in code, `a::b(...)`: two names or more, the first lowercase.
fn inline_paths(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    let chars: Vec<char> = code.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        let starts = (chars[at].is_ascii_lowercase() || chars[at] == '_')
            && (at == 0 || !(chars[at - 1].is_alphanumeric() || chars[at - 1] == '_' || chars[at - 1] == ':'));
        if !starts {
            at += 1;
            continue;
        }
        let mut segments = Vec::new();
        loop {
            let begin = at;
            while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            segments.push(chars[begin..at].iter().collect::<String>());
            if chars.get(at) == Some(&':')
                && chars.get(at + 1) == Some(&':')
                && chars.get(at + 2).is_some_and(|c| c.is_alphabetic() || *c == '_')
            {
                at += 2;
            } else {
                break;
            }
        }
        if segments.len() > 1 {
            found.push(segments.join("::"));
        }
    }
    found
}

/// The source without comments, strings or character literals, lifetimes kept as they are.
fn strip(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        match (c, next) {
            ('/', Some('/')) => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            ('/', Some('*')) => {
                let mut depth = 0;
                while at < chars.len() {
                    if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
                        depth += 1;
                        at += 2;
                    } else if chars[at] == '*' && chars.get(at + 1) == Some(&'/') {
                        depth -= 1;
                        at += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        at += 1;
                    }
                }
                out.push(' ');
            }
            ('r', Some('"' | '#')) if at == 0 || !(chars[at - 1].is_alphanumeric() || chars[at - 1] == '_') => {
                // A raw string: r"…", r#"…"#.
                let mut hashes = 0;
                let mut i = at + 1;
                while chars.get(i) == Some(&'#') {
                    hashes += 1;
                    i += 1;
                }
                if chars.get(i) != Some(&'"') {
                    out.push(c);
                    at += 1;
                    continue;
                }
                i += 1;
                while i < chars.len() {
                    if chars[i] == '"' && (0..hashes).all(|h| chars.get(i + 1 + h) == Some(&'#')) {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                out.push_str("\"\"");
                at = i;
            }
            ('"', _) => {
                at += 1;
                while at < chars.len() && chars[at] != '"' {
                    at += if chars[at] == '\\' { 2 } else { 1 };
                }
                at += 1;
                out.push_str("\"\"");
            }
            ('\'', _) => {
                // A character literal ('x', '\n', '\u{1F600}'), or a lifetime ('a), kept.
                let end = if next == Some('\\') {
                    (at + 2..chars.len().min(at + 12)).find(|&i| chars[i] == '\'')
                } else {
                    (chars.get(at + 2) == Some(&'\'')).then_some(at + 2)
                };
                if let Some(end) = end {
                    out.push_str("' '");
                    at = end + 1;
                } else {
                    out.push(c);
                    at += 1;
                }
            }
            _ => {
                out.push(c);
                at += 1;
            }
        }
    }
    out
}

/// The repository's crates: each Rust file's crate and module path, and the file of each module.
#[derive(Debug, Default)]
pub struct Crates {
    /// The file of each (crate, module path).
    modules: HashMap<(String, Vec<String>), PathBuf>,
    /// Each file's crate and module path.
    of_file: HashMap<PathBuf, (String, Vec<String>)>,
    /// The repository's own crates, as code names them (`akyba_core`).
    names: HashSet<String>,
}

impl Crates {
    /// Reads where each of `files` (relative to `root`) sits in its package.
    pub fn new(root: &Path, files: &[PathBuf]) -> Self {
        let mut crates = Self::default();
        let mut packages: HashMap<PathBuf, Option<String>> = HashMap::new();
        for file in files {
            let package = file.ancestors().skip(1).find(|folder| root.join(folder).join("Cargo.toml").is_file());
            let Some(package) = package else { continue };
            let name = packages
                .entry(package.to_owned())
                .or_insert_with(|| package_name(&root.join(package).join("Cargo.toml")))
                .clone();
            let Some(name) = name else { continue };
            let (krate, module) = match file.strip_prefix(package.join("src")) {
                Ok(inside) if !inside.starts_with("bin") => (name.clone(), module_path(inside)),
                // Tests, examples, benches, build scripts and binaries are crates of their own.
                _ => (file.to_string_lossy().into_owned(), Vec::new()),
            };
            crates.names.insert(name);
            // The library is the crate others name; a binary beside it shares the name.
            let is_lib = file.ends_with("src/lib.rs");
            let key = (krate.clone(), module.clone());
            if is_lib || !crates.modules.contains_key(&key) {
                crates.modules.insert(key, file.clone());
            }
            crates.of_file.insert(file.clone(), (krate, module));
        }
        crates
    }

    /// The file a path written in `from` reaches: the deepest module along it that is a file.
    pub fn resolve(&self, from: &Path, path: &str) -> Option<PathBuf> {
        let (krate, module) = self.of_file.get(from)?;
        let segments: Vec<&str> = path.split("::").collect();
        let (first, rest) = segments.split_first()?;
        let child = |name: &str| {
            let mut path = module.clone();
            path.push(name.to_owned());
            self.modules.contains_key(&(krate.clone(), path))
        };
        let (base_crate, mut at, rest): (&str, Vec<String>, &[&str]) = match *first {
            "crate" => (krate, Vec::new(), rest),
            "self" => (krate, module.clone(), rest),
            "super" => {
                let ups = 1 + rest.iter().take_while(|segment| **segment == "super").count();
                let kept = module.len().checked_sub(ups)?;
                (krate, module[..kept].to_vec(), &rest[ups - 1..])
            }
            name if child(name) => (krate, module.clone(), &segments[..]),
            name if self.names.contains(name) => (name, Vec::new(), rest),
            _ => return None,
        };
        let mut best = self.modules.get(&(base_crate.to_owned(), at.clone()));
        for segment in rest {
            at.push((*segment).to_owned());
            match self.modules.get(&(base_crate.to_owned(), at.clone())) {
                Some(file) => best = Some(file),
                None => break,
            }
        }
        best.cloned()
    }
}

/// A file's module path inside `src`: `a/b.rs` and `a/b/mod.rs` are `a::b`; `lib.rs` and
/// `main.rs` are the crate itself.
fn module_path(inside: &Path) -> Vec<String> {
    let mut segments: Vec<String> = inside
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if let Some(last) = segments.pop() {
        let stem = last.trim_end_matches(".rs");
        let is_root = segments.is_empty() && (stem == "lib" || stem == "main");
        if stem != "mod" && !is_root {
            segments.push(stem.to_owned());
        }
    }
    segments
}

/// The crate's name as code writes it (`akyba-core` is `akyba_core`), from `[package]`.
fn package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let mut in_package = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some(value) = line.strip_prefix("name").map(str::trim_start).and_then(|rest| rest.strip_prefix('='))
        {
            return Some(value.trim().trim_matches('"').replace('-', "_"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_use_trees_and_finds_inline_paths() {
        let source = r#"
            use std::path::Path;
            pub(crate) use crate::{chrome, theme::{self, len}, a::b as c, glob::*};
            use super::super::up;
            // use crate::commented::out;
            fn f<'a>(x: &'a str) -> char {
                let _ = akyba_core::lsp::path("crate::in_a_string");
                Vec::new();
                '"'
            }
        "#;
        let found = paths(source);
        for expected in [
            "std::path::Path",
            "crate::chrome",
            "crate::theme",
            "crate::theme::len",
            "crate::a::b",
            "crate::glob",
            "super::super::up",
            "akyba_core::lsp::path",
        ] {
            assert!(found.contains(&expected.to_owned()), "{expected} in {found:?}");
        }
        assert!(!found.iter().any(|path| path.contains("commented") || path.contains("in_a_string")), "{found:?}");
        assert!(!found.iter().any(|path| path.starts_with("Vec")), "{found:?}");
    }

    #[test]
    fn resolves_modules_super_and_workspace_crates() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        let write = |path: &str, text: &str| {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().expect("a folder")).expect("folders");
            std::fs::write(full, text).expect("written");
        };
        write("crates/shop-core/Cargo.toml", "[package]\nname = \"shop-core\"\n");
        write("crates/shop/Cargo.toml", "[package]\nname = \"shop\"\n[dependencies]\nname = \"not this\"\n");
        let files: Vec<PathBuf> = [
            "crates/shop-core/src/lib.rs",
            "crates/shop-core/src/money/mod.rs",
            "crates/shop-core/src/money/tax.rs",
            "crates/shop/src/main.rs",
            "crates/shop/src/cart.rs",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let crates = Crates::new(root, &files);
        let resolve = |from: &str, path: &str| crates.resolve(Path::new(from), path);
        let file = |path: &str| Some(PathBuf::from(path));
        assert_eq!(
            resolve("crates/shop/src/cart.rs", "shop_core::money::tax::rate"),
            file("crates/shop-core/src/money/tax.rs")
        );
        assert_eq!(resolve("crates/shop/src/cart.rs", "shop_core::Thing"), file("crates/shop-core/src/lib.rs"));
        assert_eq!(
            resolve("crates/shop-core/src/money/tax.rs", "super::round"),
            file("crates/shop-core/src/money/mod.rs")
        );
        assert_eq!(
            resolve("crates/shop-core/src/money/mod.rs", "tax::rate"),
            file("crates/shop-core/src/money/tax.rs")
        );
        assert_eq!(resolve("crates/shop/src/main.rs", "crate::cart::Cart"), file("crates/shop/src/cart.rs"));
        assert_eq!(resolve("crates/shop/src/cart.rs", "std::path::Path"), None);
    }
}
