//! Which icon a file gets in the file tree, from its name: a Lucide icon for kinds of files (data,
//! docs, images, lock files), or a small badge with the language's letters for source code, the
//! way editors mark `TS` and `JS`. Folders are the tree's own business.

/// How a file is marked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIcon {
    /// A Lucide icon, by name.
    Icon(&'static str),
    /// A language's letters in a filled square (`TS`), for source code.
    Language(&'static str),
    /// Letters in an outlined square (`T` for TOML), for configuration formats.
    Format(&'static str),
}

/// The icon for a file called `name` (just the name, not its path).
pub fn file_icon(name: &str) -> FileIcon {
    let lower = name.to_lowercase();
    // Whole names first: they say more than an extension (`package.json` is more than JSON).
    let by_name = match lower.as_str() {
        "dockerfile"
        | "containerfile"
        | ".dockerignore"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yml"
        | "compose.yaml" => Some(FileIcon::Icon("container")),
        "license" | "licence" | "copying" | "license.md" | "license.txt" | "licence.md" => {
            Some(FileIcon::Icon("scale"))
        }
        "package.json" => Some(FileIcon::Icon("package")),
        "makefile" | "justfile" | "procfile" | "brewfile" | "gemfile" | "rakefile" => {
            Some(FileIcon::Icon("square-terminal"))
        }
        "cargo.lock" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "bun.lockb" | "bun.lock"
        | "poetry.lock" | "uv.lock" | "gemfile.lock" | "composer.lock" | "flake.lock" | "go.sum" => {
            Some(FileIcon::Icon("lock"))
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".git-blame-ignore-revs" | ".mailmap" => {
            Some(FileIcon::Icon("git-branch"))
        }
        _ => None,
    };
    if let Some(icon) = by_name {
        return icon;
    }
    // `lower` is already lowercase, so comparing its ending is case-insensitive.
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the name is lowercased above")]
    let env = lower.ends_with(".env");
    if lower == ".env" || lower.starts_with(".env.") || env {
        return FileIcon::Icon("key-round");
    }
    if lower.starts_with("readme") || lower.starts_with("changelog") || lower.starts_with("contributing") {
        return FileIcon::Icon("book-open");
    }
    match lower.rsplit_once('.') {
        Some((_, extension)) => by_extension(extension),
        // A dotfile without an extension (`.npmrc`, `.editorconfig`) is configuration.
        None if lower.starts_with('.') => FileIcon::Icon("settings-2"),
        // No extension at all (`report`, `CODEOWNERS`): most likely text, or a script.
        None => FileIcon::Icon("text-align-start"),
    }
}

fn by_extension(extension: &str) -> FileIcon {
    use FileIcon::{Format, Icon, Language};
    match extension {
        "ts" | "tsx" | "mts" | "cts" => Language("TS"),
        "js" | "jsx" | "mjs" | "cjs" => Language("JS"),
        "rs" => Language("RS"),
        "py" | "pyi" => Language("PY"),
        "go" => Language("GO"),
        "rb" => Language("RB"),
        "java" => Language("JV"),
        "kt" | "kts" => Language("KT"),
        "swift" => Language("SW"),
        "c" | "h" => Language("C"),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => Language("C+"),
        "cs" => Language("C#"),
        "php" => Language("PH"),
        "ex" | "exs" => Language("EX"),
        "lua" => Language("LU"),
        "dart" => Language("DA"),
        "zig" => Language("ZG"),
        "vue" => Language("VU"),
        "svelte" => Language("SV"),
        "astro" => Language("AS"),
        "toml" => Format("T"),
        "yaml" | "yml" => Format("Y"),
        "graphql" | "gql" => Format("GQ"),
        "xml" | "plist" => Format("<>"),
        "json" | "jsonc" | "json5" => Icon("braces"),
        "md" | "mdx" | "markdown" | "rst" | "adoc" | "txt" => Icon("book-open"),
        "log" => Icon("text-align-start"),
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "command" => Icon("square-terminal"),
        "sql" | "prisma" | "db" | "sqlite" => Icon("database"),
        "css" | "scss" | "sass" | "less" => Icon("palette"),
        "html" | "htm" => Icon("globe"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "avif" | "bmp" => Icon("image"),
        "csv" | "tsv" | "xlsx" | "parquet" => Icon("table-2"),
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" => Icon("file-archive"),
        "lock" => Icon("lock"),
        "pem" | "key" | "crt" | "p12" => Icon("shield"),
        "env" => Icon("key-round"),
        "ini" | "cfg" | "conf" | "properties" | "editorconfig" | "npmrc" | "nvmrc" => Icon("settings-2"),
        _ => Icon("file"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FileIcon::{Format, Icon, Language};

    #[test]
    fn names_say_more_than_extensions() {
        assert_eq!(file_icon("package.json"), Icon("package"));
        assert_eq!(file_icon("tsconfig.json"), Icon("braces"));
        assert_eq!(file_icon("pnpm-lock.yaml"), Icon("lock"));
        assert_eq!(file_icon("Cargo.lock"), Icon("lock"));
        assert_eq!(file_icon("Dockerfile"), Icon("container"));
        assert_eq!(file_icon("docker-compose.yml"), Icon("container"));
        assert_eq!(file_icon("README.md"), Icon("book-open"));
        assert_eq!(file_icon("LICENSE"), Icon("scale"));
        assert_eq!(file_icon(".gitignore"), Icon("git-branch"));
        assert_eq!(file_icon(".env.local"), Icon("key-round"));
    }

    #[test]
    fn code_gets_its_language_and_config_its_format() {
        assert_eq!(file_icon("checkout.test.tsx"), Language("TS"));
        assert_eq!(file_icon("commitlint.config.mjs"), Language("JS"));
        assert_eq!(file_icon("main.rs"), Language("RS"));
        assert_eq!(file_icon("Cargo.toml"), Format("T"));
        assert_eq!(file_icon("pnpm-workspace.yaml"), Format("Y"));
        assert_eq!(file_icon(".npmrc"), Icon("settings-2"));
        assert_eq!(file_icon(".editorconfig"), Icon("settings-2"));
        assert_eq!(file_icon("report"), Icon("text-align-start"));
        assert_eq!(file_icon("release.sh"), Icon("square-terminal"));
        assert_eq!(file_icon("how-to-open.txt"), Icon("book-open"));
        assert_eq!(file_icon("archive.weird"), Icon("file"));
        assert_eq!(file_icon("photo.PNG"), Icon("image"));
    }
}
