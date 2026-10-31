//! The editor's languages beyond what gpui-component bundles, and full highlight queries where its
//! bundled ones fall short: TSX gets only TypeScript's own additions, which leave keywords and
//! strings to JavaScript's query, and Swift gets no query at all. Every grammar here comes from
//! its own crate with the queries its authors ship (MIT or Apache-2.0).

use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};

/// A language Akyba adds: its name in the registry (what `language_for` returns), the grammar,
/// and its highlight, injection and locals queries.
struct Extra {
    name: &'static str,
    language: tree_sitter::Language,
    highlights: &'static str,
    injections: &'static str,
    locals: &'static str,
}

impl Extra {
    fn new(name: &'static str, language: impl Into<tree_sitter::Language>, highlights: &'static str) -> Self {
        Self { name, language: language.into(), highlights, injections: "", locals: "" }
    }

    fn injections(mut self, injections: &'static str) -> Self {
        self.injections = injections;
        self
    }

    fn locals(mut self, locals: &'static str) -> Self {
        self.locals = locals;
        self
    }
}

fn extras() -> Vec<Extra> {
    vec![
        Extra::new("dart", tree_sitter_dart::LANGUAGE, tree_sitter_dart::HIGHLIGHTS_QUERY),
        Extra::new("elm", tree_sitter_elm::LANGUAGE, tree_sitter_elm::HIGHLIGHTS_QUERY),
        Extra::new("erlang", tree_sitter_erlang::LANGUAGE, tree_sitter_erlang::HIGHLIGHTS_QUERY),
        Extra::new("fish", tree_sitter_fish::language(), tree_sitter_fish::HIGHLIGHTS_QUERY),
        Extra::new("gleam", tree_sitter_gleam::LANGUAGE, tree_sitter_gleam::HIGHLIGHT_QUERY),
        Extra::new("glsl", tree_sitter_glsl::LANGUAGE_GLSL, tree_sitter_glsl::HIGHLIGHTS_QUERY),
        Extra::new("haskell", tree_sitter_haskell::LANGUAGE, tree_sitter_haskell::HIGHLIGHTS_QUERY),
        Extra::new("ini", tree_sitter_ini::LANGUAGE, tree_sitter_ini::HIGHLIGHTS_QUERY),
        Extra::new("nix", tree_sitter_nix::LANGUAGE, tree_sitter_nix::HIGHLIGHTS_QUERY),
        Extra::new("objc", tree_sitter_objc::LANGUAGE, tree_sitter_objc::HIGHLIGHTS_QUERY),
        Extra::new("powershell", tree_sitter_powershell::LANGUAGE, tree_sitter_powershell::HIGHLIGHTS_QUERY),
        Extra::new("prisma", tree_sitter_prisma::LANGUAGE, tree_sitter_prisma::HIGHLIGHTS_QUERY),
        Extra::new("r", tree_sitter_r::LANGUAGE, tree_sitter_r::HIGHLIGHTS_QUERY),
        Extra::new("scss", tree_sitter_scss::language(), tree_sitter_scss::HIGHLIGHTS_QUERY),
        Extra::new("vim", tree_sitter_vim::language(), tree_sitter_vim::HIGHLIGHTS_QUERY),
        Extra::new("xml", tree_sitter_xml::LANGUAGE_XML, tree_sitter_xml::XML_HIGHLIGHT_QUERY),
        Extra::new("swift", tree_sitter_swift::LANGUAGE, tree_sitter_swift::HIGHLIGHTS_QUERY)
            .injections(tree_sitter_swift::INJECTIONS_QUERY)
            .locals(tree_sitter_swift::LOCALS_QUERY),
    ]
}

/// TSX's highlights: TypeScript's patterns first, so they win where both match, then
/// JavaScript's and JSX's, as tree-sitter's own config for TSX lists them.
fn tsx_highlights() -> String {
    [
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
    ]
    .join("\n")
}

/// JavaScript's injections for TSX, without the one that hands every comment to the JSDoc grammar:
/// Akyba bundles no JSDoc colours, so `/** … */` would lose its comment grey.
fn tsx_injections() -> String {
    let query = tree_sitter_javascript::INJECTIONS_QUERY;
    let Some(start) = query.find("((comment) @injection.content") else { return query.to_owned() };
    let end = query[start..].find("\n\n").map_or(query.len(), |end| start + end);
    format!("{}{}", &query[..start], &query[end..])
}

pub fn register() {
    let registry = LanguageRegistry::singleton();
    if let Some(mut tsx) = registry.language("tsx") {
        tsx.highlights = tsx_highlights().into();
        tsx.injections = tsx_injections().into();
        registry.register("tsx", &tsx);
    }
    for extra in extras() {
        let config =
            LanguageConfig::new(extra.name, extra.language, vec![], extra.highlights, extra.injections, extra.locals);
        registry.register(extra.name, &config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query that doesn't compile against its grammar leaves the file uncoloured without a word,
    /// so each one is compiled here.
    #[test]
    fn every_added_query_compiles_against_its_grammar() {
        for extra in extras() {
            for (kind, query) in [("highlights", extra.highlights), ("injections", extra.injections)] {
                if let Err(error) = tree_sitter::Query::new(&extra.language, query) {
                    panic!("{}'s {kind} query: {error}", extra.name);
                }
            }
        }
        let tsx: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
        if let Err(error) = tree_sitter::Query::new(&tsx, &tsx_highlights()) {
            panic!("tsx's highlights query: {error}");
        }
        if let Err(error) = tree_sitter::Query::new(&tsx, &tsx_injections()) {
            panic!("tsx's injections query: {error}");
        }
    }

    #[test]
    fn tsx_comments_stay_comments() {
        let injections = tsx_injections();
        assert!(!injections.contains("jsdoc"));
        // The other injections stay: tagged template literals and regexes.
        assert!(injections.contains("template_string") && injections.contains("regex"));
    }
}
