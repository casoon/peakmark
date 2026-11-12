//! Measuring a module: its size on disk and on the wire, what it declares,
//! and the linear memory it holds after each scenario.

use std::path::Path;

use crate::compress::{brotli_len, gzip_len};
use crate::config::{Config, Module};
use crate::digest::sha256;
use crate::error::Error;
use crate::inventory::Inventory;
use crate::metric::{Metric, Values};
use crate::runner::{ScenarioResult, run_scenario};
use crate::wasm::{InspectError, Sections, inspect};

/// Everything measured about one module.
pub struct Measured {
    /// The gated values.
    pub values: Values,
    /// Section sizes, reported but never gated.
    pub sections: Sections,
    /// What the module's memory section asks for; `None` if it has none.
    pub initial_memory: Option<u64>,
    /// The static inventory of the artifact: interface, features, sections.
    pub inventory: Inventory,
    /// One result per configured scenario.
    pub scenarios: Vec<ScenarioResult>,
    /// SHA-256 of the artifact. It changes with every build and identifies
    /// the file; it never decides whether values are comparable.
    pub artifact_sha256: String,
}

impl Measured {
    /// The scenario a memory value comes from.
    #[must_use]
    pub fn scenario_for(&self, metric: Metric) -> Option<&Path> {
        let worst = match metric {
            Metric::PeakMemory => self.scenarios.iter().max_by_key(|s| s.peak_memory),
            Metric::GrowthPerRun => self.scenarios.iter().max_by_key(|s| s.growth_per_run),
            Metric::Raw | Metric::Gzip | Metric::Brotli => None,
        };
        worst.map(|scenario| scenario.file.as_path())
    }
}

/// Measures `module`: reads and inspects its artifact and runs its scenarios.
///
/// # Errors
///
/// Fails if the artifact cannot be read or is not a valid module, or if a
/// scenario cannot be run.
pub fn measure(config: &Config, module: &Module) -> Result<Measured, Error> {
    let path = config.dir.join(&module.artifact);
    let bytes = std::fs::read(&path).map_err(|source| Error::Read {
        path: path.clone(),
        source,
    })?;
    let inspection = inspect(&bytes).map_err(|error| match error {
        InspectError::Invalid(source) => Error::InvalidModule { path, source },
        InspectError::UnsupportedFeature(source) => Error::UnsupportedFeature { path, source },
        InspectError::Component => Error::UnsupportedComponent { path },
    })?;

    let mut scenarios = Vec::new();
    for scenario in &module.scenarios {
        scenarios.push(run_scenario(
            config,
            module,
            scenario,
            inspection.memory_count,
        )?);
    }

    let values = Values {
        raw: Some(bytes.len() as u64),
        gzip: Some(gzip_len(&bytes)),
        brotli: Some(brotli_len(&bytes)),
        peak_memory: scenarios.iter().map(|s| s.peak_memory).max(),
        growth_per_run: scenarios.iter().filter_map(|s| s.growth_per_run).max(),
    };
    Ok(Measured {
        values,
        sections: inspection.sections,
        initial_memory: inspection.initial_memory,
        inventory: inspection.inventory,
        scenarios,
        artifact_sha256: sha256(&bytes),
    })
}
