//! A repository's worktree recipe, `.akyba/worktree.json`: how to make a fresh worktree ready
//! (`setup`: install, build, generate), how to start the app (`run`: named commands, one terminal
//! each), and how to tidy up before a worktree goes (`cleanup`). A fresh worktree has the code but
//! nothing git ignores, so it can't run until someone remembers the steps; the recipe remembers.
//!
//! ```json
//! {
//!   "setup": ["pnpm install", "pnpm --filter @acme/shared build"],
//!   "run": { "api": "pnpm --filter api dev", "web": "pnpm --filter web dev" },
//!   "cleanup": "docker compose down"
//! }
//! ```
//!
//! `setup` and `cleanup` are a command or a list run one after another, stopping at the first that
//! fails. Every field is optional.
//!
//! A repository without the file still gets a recipe, guessed from what it already has: its lockfile
//! says how to install (`pnpm-lock.yaml`: `pnpm install`, `uv.lock`: `uv sync`…) and its `dev`
//! scripts say how to start it. The guess can be saved as the file, to adjust and commit.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where the recipe lives in a checkout.
pub const FILE: &str = ".akyba/worktree.json";
/// Where the recipe was before Akyba: still read, so a repository's committed recipe keeps working.
pub const FORMER_FILE: &str = ".saturn/worktree.json";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recipe {
    pub setup: Vec<String>,
    /// Name and command, in the file's order.
    pub run: Vec<(String, String)>,
    pub cleanup: Vec<String>,
    /// Named checks (`"test": "pnpm test"`), run on demand with their logs kept.
    pub checks: Vec<(String, String)>,
}

/// One command or several.
#[derive(Deserialize)]
#[serde(untagged)]
enum Steps {
    One(String),
    Many(Vec<String>),
}

impl Steps {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(step) => vec![step],
            Self::Many(steps) => steps,
        }
        .into_iter()
        .map(|step| step.trim().to_owned())
        .filter(|step| !step.is_empty())
        .collect()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    setup: Option<Steps>,
    run: Option<serde_json::Map<String, serde_json::Value>>,
    cleanup: Option<Steps>,
    checks: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Named commands from a map in the file, saying which one isn't a command.
fn named(map: Option<serde_json::Map<String, serde_json::Value>>, key: &str) -> Result<Vec<(String, String)>, String> {
    map.unwrap_or_default()
        .into_iter()
        .map(|(name, command)| match command {
            serde_json::Value::String(command) if !command.trim().is_empty() => Ok((name, command.trim().to_owned())),
            _ => Err(format!("\"{key}\": \"{name}\" needs a command, as text")),
        })
        .collect()
}

impl Recipe {
    /// Reads a recipe, saying what is wrong with it in words.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|error| error.to_string())?;
        Ok(Self {
            setup: file.setup.map(Steps::into_vec).unwrap_or_default(),
            run: named(file.run, "run")?,
            cleanup: file.cleanup.map(Steps::into_vec).unwrap_or_default(),
            checks: named(file.checks, "checks")?,
        })
    }

    /// The setup steps as one shell command, each running only if the one before succeeded.
    pub fn setup_command(&self) -> Option<String> {
        and_then(&self.setup)
    }

    /// The cleanup steps as one shell command.
    pub fn cleanup_command(&self) -> Option<String> {
        and_then(&self.cleanup)
    }
}

fn and_then(steps: &[String]) -> Option<String> {
    (!steps.is_empty()).then(|| {
        if steps.len() == 1 {
            steps[0].clone()
        } else {
            steps.iter().map(|step| format!("({step})")).collect::<Vec<_>>().join(" && ")
        }
    })
}

/// Where a checkout's recipe came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `.akyba/worktree.json`, its own or the main checkout's.
    File,
    /// Guessed from its lockfiles and manifests.
    Guessed,
}

/// The recipe for `checkout`: its file (see `load`), else a guess, which may be empty. An error
/// names the file that is wrong.
pub fn for_checkout(checkout: &Path) -> Result<(Recipe, Source), String> {
    match load(checkout) {
        Some(Ok(recipe)) => Ok((recipe, Source::File)),
        Some(Err(problem)) => Err(problem),
        None => Ok((guess(checkout), Source::Guessed)),
    }
}

/// The recipe for `checkout`: its own file, else the main checkout's (a branch made before the
/// recipe existed still gets set up). `None` when neither has one; an error names the file.
pub fn load(checkout: &Path) -> Option<Result<Recipe, String>> {
    let main = crate::git::main_worktree(checkout);
    let path: PathBuf = [Some(checkout.to_path_buf()), main]
        .into_iter()
        .flatten()
        .flat_map(|root| [root.join(FILE), root.join(FORMER_FILE)])
        .find(|path| path.is_file())?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => return Some(Err(format!("{}: {error}", path.display()))),
    };
    Some(Recipe::parse(&text).map_err(|error| format!("{}: {error}", path.display())))
}

/// How a JavaScript repository installs and runs a package's script.
#[derive(Clone, Copy)]
enum PackageManager {
    Pnpm,
    Yarn,
    Bun,
    Npm,
}

impl PackageManager {
    fn of(checkout: &Path) -> Option<Self> {
        let has = |name: &str| checkout.join(name).is_file();
        if has("pnpm-lock.yaml") {
            Some(Self::Pnpm)
        } else if has("yarn.lock") {
            Some(Self::Yarn)
        } else if has("bun.lock") || has("bun.lockb") {
            Some(Self::Bun)
        } else if has("package-lock.json") || has("package.json") {
            Some(Self::Npm)
        } else {
            None
        }
    }

    fn install(self) -> &'static str {
        match self {
            Self::Pnpm => "pnpm install",
            Self::Yarn => "yarn install",
            Self::Bun => "bun install",
            Self::Npm => "npm install",
        }
    }

    /// Runs `script` at the top (`package` `None`) or in one workspace package.
    fn run(self, script: &str, package: Option<(&str, &str)>) -> String {
        match (self, package) {
            (Self::Pnpm, None) => format!("pnpm {script}"),
            (Self::Yarn, None) => format!("yarn {script}"),
            (Self::Bun, None) => format!("bun run {script}"),
            (Self::Npm, None) => format!("npm run {script}"),
            (Self::Pnpm, Some((name, _))) => format!("pnpm --filter {name} {script}"),
            (Self::Yarn, Some((name, _))) => format!("yarn workspace {name} {script}"),
            (Self::Bun, Some((_, folder))) => format!("bun run --cwd {folder} {script}"),
            (Self::Npm, Some((_, folder))) => format!("npm run {script} -w {folder}"),
        }
    }
}

/// A `package.json`'s name and whether it has a `dev` script.
fn package(folder: &Path) -> Option<(Option<String>, bool)> {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("package.json")).ok()?).ok()?;
    let name = json.get("name").and_then(serde_json::Value::as_str).map(str::to_owned);
    let dev = json.get("scripts").and_then(|scripts| scripts.get("dev")).is_some();
    Some((name, dev))
}

/// The folders where monorepos keep their apps, in the order they're shown.
const APP_FOLDERS: [&str; 4] = ["apps", "services", "packages", "examples"];

/// A recipe from what `checkout` already has. Empty when nothing is recognised.
pub fn guess(checkout: &Path) -> Recipe {
    let has = |name: &str| checkout.join(name).exists();
    let mut recipe = Recipe::default();
    if let Some(manager) = PackageManager::of(checkout) {
        recipe.setup.push(manager.install().to_owned());
        if package(checkout).is_some_and(|(_, dev)| dev) {
            recipe.run.push(("dev".to_owned(), manager.run("dev", None)));
        } else {
            // A monorepo: each app with a dev script runs on its own, named after its folder.
            for parent in APP_FOLDERS {
                let Ok(entries) = std::fs::read_dir(checkout.join(parent)) else { continue };
                let mut folders: Vec<PathBuf> =
                    entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect();
                folders.sort();
                for folder in folders {
                    let Some((name, true)) = package(&folder) else { continue };
                    let Some(short) = folder.file_name().map(|name| name.to_string_lossy().into_owned()) else {
                        continue;
                    };
                    let relative = format!("{parent}/{short}");
                    let name = name.unwrap_or_else(|| relative.clone());
                    recipe.run.push((short, manager.run("dev", Some((&name, &relative)))));
                }
            }
        }
    }
    let installs = [
        ("uv.lock", "uv sync"),
        ("poetry.lock", "poetry install"),
        ("Pipfile.lock", "pipenv install"),
        ("Gemfile.lock", "bundle install"),
        ("composer.lock", "composer install"),
        ("mix.lock", "mix deps.get"),
        ("go.sum", "go mod download"),
    ];
    for (file, install) in installs {
        if has(file) {
            recipe.setup.push(install.to_owned());
        }
    }
    if checkout.join("bin/dev").is_file() {
        recipe.run.push(("dev".to_owned(), "bin/dev".to_owned()));
    }
    recipe.checks = guess_checks(checkout);
    recipe
}

/// Checks from what a checkout already has: its `package.json` scripts that check (lint, typecheck,
/// test), or Cargo's.
fn guess_checks(checkout: &Path) -> Vec<(String, String)> {
    let mut checks = Vec::new();
    if let Some(manager) = PackageManager::of(checkout) {
        let scripts: Vec<String> = std::fs::read_to_string(checkout.join("package.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|json| json.get("scripts")?.as_object().map(|scripts| scripts.keys().cloned().collect()))
            .unwrap_or_default();
        for name in ["lint", "typecheck", "test"] {
            if scripts.iter().any(|script| script == name) {
                checks.push((name.to_owned(), manager.run(name, None)));
            }
        }
    }
    if checkout.join("Cargo.toml").is_file() {
        checks.push(("clippy".to_owned(), "cargo clippy --all-targets".to_owned()));
        checks.push(("test".to_owned(), "cargo test".to_owned()));
    }
    checks
}

impl Recipe {
    /// The recipe as `.akyba/worktree.json` holds it.
    pub fn to_json(&self) -> String {
        let mut file = serde_json::Map::new();
        if !self.setup.is_empty() {
            file.insert("setup".into(), self.setup.clone().into());
        }
        if !self.run.is_empty() {
            let run: serde_json::Map<String, serde_json::Value> =
                self.run.iter().map(|(name, command)| (name.clone(), command.clone().into())).collect();
            file.insert("run".into(), run.into());
        }
        if !self.cleanup.is_empty() {
            file.insert("cleanup".into(), self.cleanup.clone().into());
        }
        if !self.checks.is_empty() {
            let checks: serde_json::Map<String, serde_json::Value> =
                self.checks.iter().map(|(name, command)| (name.clone(), command.clone().into())).collect();
            file.insert("checks".into(), checks.into());
        }
        let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(file)).unwrap_or_default();
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_steps_as_one_command_or_a_list() {
        let recipe = Recipe::parse(
            r#"{ "setup": ["pnpm install", " ", "pnpm build"], "run": { "api": "pnpm --filter api dev", "web": "vite" }, "cleanup": "docker compose down" }"#,
        )
        .expect("a valid recipe");
        assert_eq!(recipe.setup_command().as_deref(), Some("(pnpm install) && (pnpm build)"));
        assert_eq!(recipe.cleanup_command().as_deref(), Some("docker compose down"));
        assert_eq!(
            recipe.run,
            [("api".to_owned(), "pnpm --filter api dev".to_owned()), ("web".to_owned(), "vite".to_owned())]
        );
    }

    fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary folder");
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("make folders");
            std::fs::write(path, text).expect("write");
        }
        dir
    }

    #[test]
    fn guesses_a_single_app() {
        let dir = checkout(&[("package.json", r#"{ "scripts": { "dev": "vite" } }"#), ("yarn.lock", "")]);
        let recipe = guess(dir.path());
        assert_eq!(recipe.setup, ["yarn install"]);
        assert_eq!(recipe.run, [("dev".to_owned(), "yarn dev".to_owned())]);
    }

    #[test]
    fn guesses_each_app_of_a_monorepo() {
        let dir = checkout(&[
            ("package.json", r#"{ "scripts": { "build": "turbo build" } }"#),
            ("pnpm-lock.yaml", ""),
            ("apps/web/package.json", r#"{ "name": "@acme/web", "scripts": { "dev": "vite" } }"#),
            ("apps/api/package.json", r#"{ "name": "@acme/api", "scripts": { "dev": "nest start --watch" } }"#),
            ("packages/shared/package.json", r#"{ "name": "@acme/shared", "scripts": { "build": "tsc" } }"#),
        ]);
        let recipe = guess(dir.path());
        assert_eq!(recipe.setup, ["pnpm install"]);
        assert_eq!(
            recipe.run,
            [
                ("api".to_owned(), "pnpm --filter @acme/api dev".to_owned()),
                ("web".to_owned(), "pnpm --filter @acme/web dev".to_owned()),
            ]
        );
    }

    #[test]
    fn guesses_other_languages() {
        let dir = checkout(&[("uv.lock", ""), ("Gemfile.lock", ""), ("bin/dev", "#!/bin/sh")]);
        let recipe = guess(dir.path());
        assert_eq!(recipe.setup, ["uv sync", "bundle install"]);
        assert_eq!(recipe.run, [("dev".to_owned(), "bin/dev".to_owned())]);
        assert_eq!(guess(checkout(&[("README.md", "")]).path()), Recipe::default());
    }

    #[test]
    fn a_guess_saves_as_a_file_that_reads_back_the_same() {
        let dir = checkout(&[("package.json", r#"{ "scripts": { "dev": "next dev" } }"#), ("package-lock.json", "")]);
        let recipe = guess(dir.path());
        let text = recipe.to_json();
        assert!(text.starts_with("{\n  \"setup\": [\n    \"npm install\"\n  ],"), "{text}");
        assert_eq!(Recipe::parse(&text).expect("valid"), recipe);
    }

    #[test]
    fn reads_and_guesses_checks() {
        let recipe =
            Recipe::parse(r#"{ "checks": { "fast": "pnpm typecheck", "full": "pnpm health" } }"#).expect("valid");
        assert_eq!(
            recipe.checks,
            [("fast".to_owned(), "pnpm typecheck".to_owned()), ("full".to_owned(), "pnpm health".to_owned())]
        );
        assert!(Recipe::parse(r#"{ "checks": { "fast": 1 } }"#).expect_err("not a command").contains("checks"));
        let dir = checkout(&[
            ("package.json", r#"{ "scripts": { "test": "vitest", "lint": "eslint .", "dev": "vite" } }"#),
            ("pnpm-lock.yaml", ""),
        ]);
        assert_eq!(
            guess(dir.path()).checks,
            [("lint".to_owned(), "pnpm lint".to_owned()), ("test".to_owned(), "pnpm test".to_owned())]
        );
        let saved = guess(dir.path());
        assert_eq!(Recipe::parse(&saved.to_json()).expect("valid"), saved);
    }

    #[test]
    fn every_field_is_optional() {
        let recipe = Recipe::parse("{}").expect("an empty recipe is valid");
        assert_eq!(recipe, Recipe::default());
        assert_eq!(recipe.setup_command(), None);
    }

    #[test]
    fn says_what_is_wrong() {
        assert!(Recipe::parse(r#"{ "setpu": "pnpm install" }"#).expect_err("a typo").contains("setpu"));
        assert!(Recipe::parse(r#"{ "run": { "web": 5173 } }"#).expect_err("not a command").contains("web"));
        assert!(Recipe::parse("{ setup: ").is_err());
    }

    #[test]
    fn a_recipe_from_before_akyba_still_reads() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        std::fs::create_dir_all(dir.path().join(".saturn")).expect("make folders");
        std::fs::write(dir.path().join(FORMER_FILE), r#"{"setup": "pnpm install"}"#).expect("write");
        let recipe = load(dir.path()).expect("a recipe").expect("valid");
        assert_eq!(recipe.setup.len(), 1);
        // A recipe under Akyba's own name wins over the old one.
        std::fs::create_dir_all(dir.path().join(".akyba")).expect("make folders");
        std::fs::write(dir.path().join(FILE), r#"{"setup": ["pnpm install", "pnpm build"]}"#).expect("write");
        assert_eq!(load(dir.path()).expect("a recipe").expect("valid").setup.len(), 2);
    }

    #[test]
    fn a_worktree_falls_back_to_the_main_checkout_recipe() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let main = dir.path().join("acme-web");
        std::fs::create_dir_all(main.join(".akyba")).expect("make folders");
        let git = |args: &[&str]| {
            let status = crate::git::command(&main).args(args).output().expect("git runs").status;
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "start",
        ]);
        let worktree = dir.path().join("acme-web-abc-123");
        git(&["worktree", "add", "-q", "-b", "abc-123", &worktree.to_string_lossy()]);

        assert!(load(&worktree).is_none(), "no recipe anywhere");
        std::fs::write(main.join(FILE), r#"{ "setup": "pnpm install" }"#).expect("write");
        let recipe = load(&worktree).expect("main's recipe").expect("valid");
        assert_eq!(recipe.setup_command().as_deref(), Some("pnpm install"));

        std::fs::create_dir_all(worktree.join(".akyba")).expect("make folders");
        std::fs::write(worktree.join(FILE), "{ broken").expect("write");
        assert!(load(&worktree).expect("its own file").expect_err("broken").contains("worktree.json"));
    }
}
