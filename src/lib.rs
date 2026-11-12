//! peakmark gates WebAssembly modules on size and memory against a baseline
//! kept in the repository.
//!
//! The crate is the engine behind the `peakmark` command. A run reads the
//! configuration ([`config`]) and the baseline ([`baseline`]), measures every
//! module ([`measure`]), judges the result against budget and baseline
//! ([`check`]) and prints it ([`report`]); [`cli`] ties these together and owns
//! the exit codes. Every fallible step returns [`Error`].
//!
//! ```no_run
//! use peakmark::{baseline, check, config, measure::measure};
//!
//! # fn main() -> Result<(), peakmark::Error> {
//! let config = config::load(std::path::Path::new("peakmark.toml"))?;
//! let baseline = baseline::load(&config.dir)?;
//! for module in &config.modules {
//!     let measured = measure(&config, module)?;
//!     let result = check::evaluate(
//!         module,
//!         measured,
//!         baseline.modules.get(&module.name),
//!         config.tolerance,
//!     );
//!     println!("{}: over budget = {}", module.name, result.over_budget());
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod baseline;
pub mod check;
pub mod cli;
mod compress;
pub mod config;
mod date;
mod digest;
pub mod error;
pub mod measure;
pub mod metric;
pub mod report;
pub mod runner;
pub mod units;
pub mod wasm;

pub use error::Error;
