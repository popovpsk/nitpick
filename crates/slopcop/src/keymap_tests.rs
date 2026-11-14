use std::collections::HashMap;

use gpui::OperatingSystem;
use rstest::rstest;

use super::{COMMANDS, default_keys, normalize};

#[rstest]
#[case::mac(OperatingSystem::Mac)]
#[case::linux(OperatingSystem::Linux)]
#[case::windows(OperatingSystem::Windows)]
fn every_default_parses_and_no_context_binds_a_key_twice(#[case] os: OperatingSystem) {
    let mut seen: HashMap<(Option<&str>, String), &str> = HashMap::new();
    for command in COMMANDS {
        let keys = default_keys(command, os);
        assert_eq!(keys.len(), command.defaults.len(), "{}", command.id);
        for key in keys {
            if let Some(other) = seen.insert((command.context, key.clone()), command.id) {
                panic!("{key} is bound to both {other} and {}", command.id);
            }
        }
    }
}

#[test]
fn ids_are_unique() {
    let mut ids: Vec<&str> = COMMANDS.iter().map(|c| c.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), COMMANDS.len());
}

#[rstest]
#[case::secondary_mac("secondary-b", OperatingSystem::Mac, "cmd-b")]
#[case::secondary_linux("secondary-b", OperatingSystem::Linux, "ctrl-b")]
#[case::modifier_order("shift-cmd-n", OperatingSystem::Mac, "shift-cmd-n")]
#[case::plain("f7", OperatingSystem::Linux, "f7")]
fn normalize_spells_keys_for_the_host(
    #[case] key: &str,
    #[case] os: OperatingSystem,
    #[case] expected: &str,
) {
    let once = normalize(key, os).unwrap();
    assert_eq!(normalize(&once, os).as_deref(), Some(once.as_str()));
    assert_eq!(normalize(expected, os).as_deref(), Some(once.as_str()));
}
