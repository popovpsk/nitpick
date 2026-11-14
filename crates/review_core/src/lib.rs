pub mod fuzzy;
pub mod hidden;
pub mod language;
pub mod line_map;
pub mod tree;
pub mod viewed;

pub use fuzzy::{FuzzyMatch, Ranked, fuzzy_match, highlight_ranges, rank};
pub use hidden::{HideRules, RuleError};
pub use language::language_override;
pub use line_map::{Change, LineKind, LineMap, Side, line_maps};
pub use tree::{FileStatus, FileTree, Node, NodeKind, Progress, Row, TreeFile, TreeShape};
pub use viewed::{ViewedStore, fingerprint, write_atomic};

#[cfg(test)]
mod fuzzy_tests;
#[cfg(test)]
mod hidden_tests;
#[cfg(test)]
mod tests;
