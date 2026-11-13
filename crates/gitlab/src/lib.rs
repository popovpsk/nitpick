mod client;
mod error;
mod models;
mod mr_url;
mod request;

#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod models_tests;
#[cfg(test)]
mod mr_url_tests;

pub use client::Client;
pub use error::Error;
pub use models::{
    DiffRefs, Discussion, LineRange, LineRangeEnd, MergeRequest, Note, Position, Project, User,
};
pub use mr_url::{MrUrl, parse_mr_reference, parse_mr_url, parse_mr_url_with_base};
pub use request::{MrQuery, MrState, NewPosition, ProjectRef};
