//! Turning check results into output: a text report for people, JSON for
//! machines.

mod json;
mod text;

pub use json::json;
pub use text::report;
