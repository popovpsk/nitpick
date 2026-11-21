pub mod blame;
pub mod fuzzy;
pub mod hidden;
pub mod language;
pub mod line_map;
pub mod svg;
pub mod timestamp;
pub mod tree;
pub mod viewed;

pub use fuzzy::{FuzzyMatch, Ranked, fuzzy_match, highlight_ranges, rank};
pub use hidden::{HideRules, RuleError};
pub use language::language_override;
pub use line_map::{Change, LineKind, LineMap, Side, file_spans, line_maps};
pub use svg::{SvgError, check_svg};
pub use timestamp::parse_timestamp;
pub use tree::{FileStatus, FileTree, Node, NodeKind, Progress, Row, TreeFile, TreeShape};
pub use viewed::{ViewedStore, fingerprint, fingerprint_blobs, write_atomic};

#[cfg(test)]
mod blame_tests;
#[cfg(test)]
mod fuzzy_tests;
#[cfg(test)]
mod hidden_tests;
#[cfg(test)]
mod svg_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timestamp_tests;
