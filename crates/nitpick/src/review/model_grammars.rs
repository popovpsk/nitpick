use std::ops::Range;
use std::sync::{Arc, Once};

use gpui::HighlightStyle;
use gpui_component::Rope;
use gpui_component::highlighter::{
    GrammarConfig, HighlightTheme, LanguageRegistry, SyntaxHighlighter,
};

const PROTO_HIGHLIGHTS: &str = include_str!("../../queries/proto/highlights.scm");
const GRAPHQL_HIGHLIGHTS: &str = include_str!("../../queries/graphql/highlights.scm");
const VUE_HIGHLIGHTS: &str = include_str!("../../queries/vue/highlights.scm");
const VUE_INJECTIONS: &str = include_str!("../../queries/vue/injections.scm");

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
        register_vue();
    });
}

fn register_vue() {
    let config = GrammarConfig::new(
        "vue",
        tree_sitter_vue_next::LANGUAGE.into(),
        vec!["typescript".into(), "css".into()],
        VUE_HIGHLIGHTS,
        VUE_INJECTIONS,
        "",
    );
    LanguageRegistry::singleton().register("vue", &config);
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

pub fn needs_injections(language: &str) -> bool {
    language == "vue"
}

pub fn injecting_highlighter(
    theme: Arc<HighlightTheme>,
) -> impl Fn(&str, &str) -> Vec<(Range<usize>, HighlightStyle)> + Send + Sync + 'static {
    move |language, text| {
        let mut highlighter = SyntaxHighlighter::new(language);
        highlighter.update(None, &Rope::from(text), None);
        highlighter.styles(&(0..text.len()), &*theme)
    }
}
