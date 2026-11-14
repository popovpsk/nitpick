use std::sync::Once;

use gpui_component::highlighter::LanguageRegistry;

const PROTO_HIGHLIGHTS: &str = include_str!("../../queries/proto/highlights.scm");
const GRAPHQL_HIGHLIGHTS: &str = include_str!("../../queries/graphql/highlights.scm");

pub fn register_highlight_queries() {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        let cpp = [
            tree_sitter_cpp::HIGHLIGHT_QUERY,
            tree_sitter_c::HIGHLIGHT_QUERY,
        ]
        .join("\n");
        set_highlights("cpp", &cpp);
        set_highlights("csharp", tree_sitter_c_sharp::HIGHLIGHTS_QUERY);
        set_highlights("swift", tree_sitter_swift::HIGHLIGHTS_QUERY);
        set_highlights("cmake", tree_sitter_cmake::HIGHLIGHTS_QUERY);
        set_highlights("proto", PROTO_HIGHLIGHTS);
        set_highlights("graphql", GRAPHQL_HIGHLIGHTS);
    });
}

fn set_highlights(language: &str, highlights: &str) {
    let registry = LanguageRegistry::singleton();
    let Some(mut config) = registry.language(language) else {
        log::warn!("highlighting: {language} is not compiled in");
        return;
    };
    config.highlights = highlights.to_string().into();
    registry.register(language, &config);
}
