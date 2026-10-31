//! Python's imports: `import a.b` and `from .x import y`, resolved to the repository's own modules
//! (`a/b.py` or `a/b/__init__.py`) from its top, its `src` folder, or the importing package's
//! own top.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What a Python source imports: `a.b` for `import a.b`; `module|name` for each name of
/// `from module import name` (the module may start with dots).
pub fn imports(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut in_docstring = false;
    let mut statement = String::new();
    for line in source.lines() {
        // Triple-quoted strings can mention imports; skip what's inside them.
        let quotes = line.matches("\"\"\"").count() + line.matches("'''").count();
        if in_docstring {
            in_docstring = quotes % 2 == 0;
            continue;
        }
        if quotes % 2 == 1 {
            in_docstring = true;
            continue;
        }
        let code = line.split('#').next().unwrap_or_default();
        statement.push_str(code.trim_end_matches('\\'));
        statement.push(' ');
        let open = statement.matches('(').count() > statement.matches(')').count();
        if open || code.trim_end().ends_with('\\') {
            continue;
        }
        parse(statement.trim(), &mut found);
        statement.clear();
    }
    found
}

fn parse(statement: &str, found: &mut Vec<String>) {
    if let Some(names) = statement.strip_prefix("import ") {
        for name in names.split(',') {
            if let Some(module) = name.split_whitespace().next() {
                found.push(module.to_owned());
            }
        }
    } else if let Some(rest) = statement.strip_prefix("from ")
        && let Some((module, names)) = rest.split_once(" import ")
    {
        let names = names.replace(['(', ')'], " ");
        for name in names.split(',') {
            if let Some(name) = name.split_whitespace().next() {
                found.push(format!("{}|{name}", module.trim()));
            }
        }
    }
}

/// Where a repository's Python modules are.
#[derive(Debug, Default)]
pub struct Modules {
    files: HashSet<PathBuf>,
}

impl Modules {
    pub fn new(files: &[PathBuf]) -> Self {
        Self { files: files.iter().cloned().collect() }
    }

    /// The file an import in `from` reaches, if it's one of the repository's.
    pub fn resolve(&self, from: &Path, import: &str) -> Option<PathBuf> {
        let (module, name) = match import.split_once('|') {
            Some((module, name)) => (module, Some(name).filter(|name| *name != "*")),
            None => (import, None),
        };
        let dots = module.len() - module.trim_start_matches('.').len();
        let module = &module[dots..];
        let bases: Vec<PathBuf> = if dots > 0 {
            // `.` is the importing file's package, each further dot one up.
            let mut base = from.parent().unwrap_or(Path::new("")).to_owned();
            for _ in 1..dots {
                if !base.pop() {
                    return None;
                }
            }
            vec![base]
        } else {
            self.roots(from)
        };
        let segments: Vec<&str> = module.split('.').filter(|segment| !segment.is_empty()).collect();
        for base in bases {
            let mut path = base.clone();
            path.extend(&segments);
            // `from pkg import sub` names a module when there is one.
            if let Some(name) = name
                && let Some(found) = self.module(&path.join(name))
            {
                return Some(found);
            }
            // `import a.b.c` reaches the deepest of a, a.b, a.b.c that's ours.
            for depth in (1..=segments.len()).rev() {
                let mut prefix = base.clone();
                prefix.extend(&segments[..depth]);
                if let Some(found) = self.module(&prefix) {
                    return Some(found);
                }
            }
            if segments.is_empty()
                && let Some(found) = self.module(&base)
            {
                return Some(found);
            }
        }
        None
    }

    /// The module at `path`: `path.py`, or the package's `path/__init__.py`.
    fn module(&self, path: &Path) -> Option<PathBuf> {
        let file = path.with_extension("py");
        let package = path.join("__init__.py");
        [file, package].into_iter().find(|candidate| self.files.contains(candidate))
    }

    /// Where absolute imports start: the repository's top, its `src`, and the folder holding the
    /// importing file's top package.
    fn roots(&self, from: &Path) -> Vec<PathBuf> {
        let mut top = from.parent().unwrap_or(Path::new(""));
        while self.files.contains(&top.join("__init__.py")) {
            match top.parent() {
                Some(parent) => top = parent,
                None => break,
            }
        }
        let mut roots = vec![PathBuf::new(), PathBuf::from("src"), top.to_owned()];
        roots.dedup();
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_import() {
        let source = "import os, shop.money as m\nfrom .tax import (\n    rate,\n    round_up,  # comment\n)\n\"\"\"\nimport not_this\n\"\"\"\nfrom .. import cart\n";
        assert_eq!(imports(source), ["os", "shop.money", ".tax|rate", ".tax|round_up", "..|cart"]);
    }

    #[test]
    fn resolves_absolute_relative_and_package_imports() {
        let files: Vec<PathBuf> = [
            "src/shop/__init__.py",
            "src/shop/cart.py",
            "src/shop/money/__init__.py",
            "src/shop/money/tax.py",
            "scripts/seed.py",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let modules = Modules::new(&files);
        let resolve = |from: &str, import: &str| modules.resolve(Path::new(from), import);
        let file = |path: &str| Some(PathBuf::from(path));
        assert_eq!(resolve("scripts/seed.py", "shop.money.tax"), file("src/shop/money/tax.py"));
        assert_eq!(resolve("scripts/seed.py", "shop.money|tax"), file("src/shop/money/tax.py"));
        assert_eq!(resolve("scripts/seed.py", "shop.money|Money"), file("src/shop/money/__init__.py"));
        assert_eq!(resolve("src/shop/money/tax.py", "..|cart"), file("src/shop/cart.py"));
        assert_eq!(resolve("src/shop/money/tax.py", ".|rate"), file("src/shop/money/__init__.py"));
        assert_eq!(resolve("src/shop/cart.py", "os"), None);
    }
}
