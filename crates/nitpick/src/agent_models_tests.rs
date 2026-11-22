use agent::{AcpFlavor, HarnessKind, Model};
use rstest::rstest;

use super::{ModelCache, choices, effective_command, position};

fn model(id: &str, name: &str, description: Option<&str>, is_default: bool) -> Model {
    Model {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.map(str::to_owned),
        is_default,
    }
}

fn sample() -> Vec<Model> {
    vec![
        model("opus", "Opus 5.5", Some("complex work"), true),
        model("haiku", "Haiku 5.5", None, false),
        model("p/plain", "p/plain", None, false),
    ]
}

#[test]
fn the_harness_default_comes_first_and_names_the_default_model() {
    let list = choices(&sample(), "", "Harness default", "Custom");
    assert_eq!(list[0].value, "");
    assert_eq!(list[0].title, "Harness default (Opus 5.5)");
    assert_eq!(list.len(), 4);
}

#[test]
fn the_default_entry_is_plain_when_the_default_is_unknown() {
    let list = choices(
        &[model("a", "A", None, false)],
        "",
        "Harness default",
        "Custom",
    );
    assert_eq!(list[0].title, "Harness default");
}

#[test]
fn a_model_shows_its_name_and_id_unless_they_are_the_same() {
    let list = choices(&sample(), "", "D", "C");
    assert_eq!(list[1].title, "Opus 5.5 (opus)");
    assert_eq!(list[1].description.as_deref(), Some("complex work"));
    assert_eq!(list[3].title, "p/plain");
}

#[rstest]
#[case::default_for_empty("", 0)]
#[case::default_for_blank("  ", 0)]
#[case::listed("haiku", 2)]
#[case::listed_padded(" haiku ", 2)]
fn a_stored_model_selects_its_entry(#[case] stored: &str, #[case] expected: usize) {
    let list = choices(&sample(), stored, "D", "C");
    assert_eq!(position(&list, stored), expected);
    assert_eq!(list.len(), 4);
}

#[test]
fn a_stored_id_outside_the_list_stays_selected_as_a_custom_entry() {
    let list = choices(&sample(), "my-fine-tune", "D", "Custom model id");
    assert_eq!(list.len(), 5);
    let last = list.last().unwrap();
    assert_eq!(last.value, "my-fine-tune");
    assert_eq!(last.description.as_deref(), Some("Custom model id"));
    assert_eq!(position(&list, "my-fine-tune"), 4);
}

#[test]
fn an_empty_list_still_offers_the_default_and_the_stored_id() {
    let list = choices(&[], "x", "D", "C");
    assert_eq!(list.len(), 2);
    assert_eq!(position(&list, "x"), 1);
}

#[rstest]
#[case::typed("claude --x", "claude", "claude --x")]
#[case::blank("  ", "claude", "claude")]
#[case::padded(" opencode acp ", "claude", "opencode acp")]
fn the_fetch_uses_the_command_that_would_launch(
    #[case] typed: &str,
    #[case] default: &str,
    #[case] expected: &str,
) {
    assert_eq!(effective_command(typed, default), expected);
}

#[test]
fn the_cache_is_per_kind_and_command() {
    let mut cache = ModelCache::default();
    cache.put(HarnessKind::Claude, "claude", sample());
    assert!(cache.get(HarnessKind::Claude, "claude").is_some());
    assert!(cache.get(HarnessKind::Claude, "claude --other").is_none());
    assert!(
        cache
            .get(HarnessKind::Acp(AcpFlavor::Generic), "claude")
            .is_none()
    );
    cache.put(HarnessKind::Claude, "claude", Vec::new());
    assert!(cache.get(HarnessKind::Claude, "claude").unwrap().is_empty());
}
