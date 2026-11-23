//! Turning check results and the history into output: a text report for
//! people, JSON for machines.

mod history;
mod json;
mod text;

pub use history::{history_json, history_report};
pub use json::json;
pub use text::report;
