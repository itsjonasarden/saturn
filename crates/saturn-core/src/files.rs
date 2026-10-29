//! The file tree as data: a root folder, the folders that are open, and their listings, read
//! lazily one folder at a time. Everything is listed but `.git` and `.DS_Store`; what git ignores
//! (`node_modules`, `.env.local`, `dist`) is marked, so the tree can show it faint, as editors do.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
    /// Whether git ignores it, or a folder it is in.
    pub ignored: bool,
}

#[derive(Debug, Clone)]
struct Child {
    path: PathBuf,
    name: String,
    is_dir: bool,
    ignored: bool,
}

#[derive(Debug)]
pub struct FileTree {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<Child>>,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        Self { root, expanded: HashSet::new(), listings: HashMap::new() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_expanded(&self, dir: &Path) -> bool {
        self.expanded.contains(dir)
    }

    /// Opens a closed folder or closes an open one.
    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_owned());
        }
    }

    /// Forgets the listings of `dirs` (or of every folder), so the next `rows` reads them again.
    /// Called when files change on disk.
    pub fn invalidate(&mut self, dirs: Option<&[PathBuf]>) {
        match dirs {
            Some(dirs) => {
                for dir in dirs {
                    self.listings.remove(dir);
                }
            }
            None => self.listings.clear(),
        }
    }

    /// The visible lines: the root's entries, and those of every open folder beneath them.
    pub fn rows(&mut self) -> Vec<Row> {
        let mut rows = Vec::new();
        let root = self.root.clone();
        self.push_rows(&root, 0, false, &mut rows);
        rows
    }

    fn push_rows(&mut self, dir: &Path, depth: usize, in_ignored: bool, rows: &mut Vec<Row>) {
        let children = self.listing(dir).to_vec();
        for child in children {
            let expanded = child.is_dir && self.expanded.contains(&child.path);
            let ignored = in_ignored || child.ignored;
            rows.push(Row {
                path: child.path.clone(),
                name: child.name,
                is_dir: child.is_dir,
                depth,
                expanded,
                ignored,
            });
            if expanded {
                self.push_rows(&child.path, depth + 1, ignored, rows);
            }
        }
    }

    fn listing(&mut self, dir: &Path) -> &[Child] {
        let root = self.root.clone();
        self.listings.entry(dir.to_owned()).or_insert_with(|| read_dir(&root, dir))
    }
}

/// The root's `.gitignore`, to tell which changed paths the tree and git do not care about.
pub struct Ignored(ignore::gitignore::Gitignore);

impl Ignored {
    pub fn new(root: &Path) -> Self {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
        builder.add(root.join(".gitignore"));
        Self(builder.build().unwrap_or_else(|_| ignore::gitignore::Gitignore::empty()))
    }

    /// Whether `path`, or a folder it is in, is ignored. Paths outside the root never are.
    pub fn is_ignored(&self, path: &Path) -> bool {
        path.starts_with(self.0.path()) && self.0.matched_path_or_any_parents(path, path.is_dir()).is_ignore()
    }
}

/// Never listed: git's own folder, and the folder settings macOS leaves everywhere.
const NEVER: [&str; 2] = [".git", ".DS_Store"];

/// A folder's entries: folders first, then files, each by name ignoring case, marked when the
/// repository's `.gitignore` files (read from `root` down) ignore them.
fn read_dir(root: &Path, dir: &Path) -> Vec<Child> {
    let kept: HashSet<PathBuf> = kept_by_git(root, dir);
    let mut children: Vec<Child> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| !NEVER.contains(&entry.file_name().to_string_lossy().as_ref()))
        .map(|entry| {
            let path = entry.path();
            Child {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: path.is_dir(),
                ignored: !kept.contains(&path),
                path,
            }
        })
        .collect();
    children.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    children
}

/// The entries of `dir` that git doesn't ignore.
fn kept_by_git(root: &Path, dir: &Path) -> HashSet<PathBuf> {
    ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .parents(dir != root)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.depth() == 1)
        .map(ignore::DirEntry::into_path)
        .collect()
}

/// Where a new file or folder named `name` goes in `folder`: a name, or a short path under it
/// (`src/new.ts`), never outside it and never onto something already there.
pub fn new_entry(folder: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    let relative = Path::new(name);
    let plain = relative.components().all(|part| matches!(part, std::path::Component::Normal(_)));
    if name.is_empty() || !plain {
        return Err("Type a name, like notes.md or src/notes.md".to_owned());
    }
    let path = folder.join(relative);
    if path.exists() {
        return Err(format!("{name} is already there"));
    }
    Ok(path)
}

/// `from` renamed to `name`, in the same folder.
pub fn renamed(from: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err("Type a name, without a /".to_owned());
    }
    let folder = from.parent().ok_or_else(|| "It has no folder to be renamed in".to_owned())?;
    let path = folder.join(name);
    if path == from {
        return Err("That's its name already".to_owned());
    }
    if path.exists() {
        return Err(format!("{name} is already there"));
    }
    Ok(path)
}

/// Where `from` lands when it's moved into `folder`; an error when it can't go there: into itself,
/// where it already is, or onto something of the same name.
pub fn moved_into(from: &Path, folder: &Path) -> Result<PathBuf, String> {
    let name = from.file_name().ok_or_else(|| "It has no name to move".to_owned())?;
    if folder.starts_with(from) {
        return Err("A folder can't go inside itself".to_owned());
    }
    if from.parent() == Some(folder) {
        return Err("It's there already".to_owned());
    }
    let path = folder.join(name);
    if path.exists() {
        return Err(format!("{} is already there", name.to_string_lossy()));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("apps/api")).expect("create a folder");
        fs::create_dir_all(root.join("node_modules/left-pad")).expect("create a folder");
        fs::create_dir_all(root.join(".git")).expect("create a folder");
        fs::write(root.join(".gitignore"), "node_modules/\n*.log\n").expect("write a file");
        fs::write(root.join(".env"), "PORT=4000\n").expect("write a file");
        fs::write(root.join("README.md"), "").expect("write a file");
        fs::write(root.join("debug.log"), "").expect("write a file");
        fs::write(root.join("apps/api/orders.ts"), "").expect("write a file");
        fs::write(root.join("apps/api/server.log"), "").expect("write a file");
        dir
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|row| format!("{}{}", "  ".repeat(row.depth), row.name)).collect()
    }

    fn ignored(rows: &[Row]) -> Vec<String> {
        rows.iter().filter(|row| row.ignored).map(|row| row.name.clone()).collect()
    }

    #[test]
    fn folders_come_first_and_ignored_files_are_marked() {
        let dir = project();
        let mut tree = FileTree::new(dir.path().to_owned());
        // node_modules/ and *.log are ignored but listed; .git is never shown.
        let rows = tree.rows();
        assert_eq!(names(&rows), ["apps", "node_modules", ".env", ".gitignore", "debug.log", "README.md"]);
        assert_eq!(ignored(&rows), ["node_modules", "debug.log"]);
    }

    #[test]
    fn whats_inside_an_ignored_folder_is_ignored_too() {
        let dir = project();
        let mut tree = FileTree::new(dir.path().to_owned());
        tree.toggle(&dir.path().join("node_modules"));
        assert_eq!(ignored(&tree.rows()), ["node_modules", "left-pad", "debug.log"]);
    }

    #[test]
    fn opening_a_folder_shows_its_entries_indented_and_applies_the_root_gitignore() {
        let dir = project();
        let mut tree = FileTree::new(dir.path().to_owned());
        tree.toggle(&dir.path().join("apps"));
        tree.toggle(&dir.path().join("apps/api"));
        let rows = tree.rows();
        assert_eq!(
            names(&rows),
            [
                "apps",
                "  api",
                "    orders.ts",
                "    server.log",
                "node_modules",
                ".env",
                ".gitignore",
                "debug.log",
                "README.md"
            ]
        );
        assert!(rows.iter().any(|row| row.name == "server.log" && row.ignored));
        tree.toggle(&dir.path().join("apps"));
        assert_eq!(names(&tree.rows()), ["apps", "node_modules", ".env", ".gitignore", "debug.log", "README.md"]);
    }

    #[test]
    fn changes_in_ignored_folders_are_recognised() {
        let dir = project();
        let ignored = Ignored::new(dir.path());
        assert!(ignored.is_ignored(&dir.path().join("node_modules/left-pad/index.js")));
        assert!(ignored.is_ignored(&dir.path().join("apps/api/server.log")));
        assert!(!ignored.is_ignored(&dir.path().join("apps/api/orders.ts")));
        assert!(!ignored.is_ignored(Path::new("/elsewhere/file.ts")));
    }

    #[test]
    fn listings_are_cached_until_invalidated() {
        let dir = project();
        let mut tree = FileTree::new(dir.path().to_owned());
        assert_eq!(tree.rows().len(), 6);
        fs::write(dir.path().join("new.ts"), "").expect("write a file");
        assert_eq!(tree.rows().len(), 6);
        tree.invalidate(Some(&[dir.path().to_owned()]));
        assert_eq!(tree.rows().len(), 7);
    }

    #[test]
    fn a_new_entry_stays_inside_its_folder_and_off_what_is_there() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(dir.path().join("taken.md"), "").expect("a file");
        assert_eq!(new_entry(dir.path(), " notes.md "), Ok(dir.path().join("notes.md")));
        assert_eq!(new_entry(dir.path(), "src/notes.md"), Ok(dir.path().join("src/notes.md")));
        assert!(new_entry(dir.path(), "taken.md").is_err());
        for bad in ["", "  ", "../out.md", "/etc/hosts", "./here.md"] {
            assert!(new_entry(dir.path(), bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn renames_in_place_only() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let from = dir.path().join("old.md");
        std::fs::write(&from, "").expect("a file");
        std::fs::write(dir.path().join("taken.md"), "").expect("a file");
        assert_eq!(renamed(&from, "new.md"), Ok(dir.path().join("new.md")));
        assert!(renamed(&from, "old.md").is_err());
        assert!(renamed(&from, "taken.md").is_err());
        assert!(renamed(&from, "sub/new.md").is_err());
    }

    #[test]
    fn moves_into_another_folder_but_never_into_itself() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let folder = dir.path().join("docs");
        let inner = folder.join("guides");
        std::fs::create_dir_all(&inner).expect("folders");
        let file = dir.path().join("readme.md");
        std::fs::write(&file, "").expect("a file");
        assert_eq!(moved_into(&file, &folder), Ok(folder.join("readme.md")));
        assert!(moved_into(&file, dir.path()).is_err());
        assert!(moved_into(&folder, &inner).is_err());
        assert!(moved_into(&folder, &folder).is_err());
    }
}
