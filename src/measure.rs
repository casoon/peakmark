//! Measuring a module: its size on disk and on the wire, what it declares,
//! and the linear memory it holds after each scenario.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use wasmparser::{Parser, Payload};

use crate::config::{Config, Module, Scenario};
use crate::metric::{Metric, Values};

const RUNNER: &str = include_str!("runner.mjs");
const DEFAULT_PAGE: u64 = 65_536;

pub struct Measured {
    pub values: Values,
    pub sections: Sections,
    /// What the module's memory section asks for; `None` if it has none.
    pub initial_memory: Option<u64>,
    pub scenarios: Vec<ScenarioResult>,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct Sections {
    pub code: u64,
    pub data: u64,
    pub custom: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub file: PathBuf,
    pub after_instantiate: u64,
    pub peak_memory: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub growth_per_run: Option<u64>,
    pub instantiate_ms: f64,
}

impl Measured {
    /// The scenario a memory value comes from.
    pub fn scenario_for(&self, metric: Metric) -> Option<&Path> {
        let worst = match metric {
            Metric::PeakMemory => self.scenarios.iter().max_by_key(|s| s.peak_memory),
            Metric::GrowthPerRun => self.scenarios.iter().max_by_key(|s| s.growth_per_run),
            Metric::Raw | Metric::Gzip | Metric::Brotli => None,
        };
        worst.map(|scenario| scenario.file.as_path())
    }
}

pub fn measure(config: &Config, module: &Module) -> Result<Measured, String> {
    let path = config.dir.join(&module.artifact);
    let bytes =
        std::fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let (sections, initial_memory) = inspect(&bytes).map_err(|error| {
        format!(
            "{} is not a valid WebAssembly module: {error}",
            path.display()
        )
    })?;

    let mut scenarios = Vec::new();
    for scenario in &module.scenarios {
        scenarios.push(run_scenario(config, module, scenario)?);
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
        sections,
        initial_memory,
        scenarios,
    })
}

fn inspect(bytes: &[u8]) -> wasmparser::Result<(Sections, Option<u64>)> {
    let mut sections = Sections::default();
    let mut initial_memory = None;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload? {
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory?;
                    let page = memory
                        .page_size_log2
                        .map_or(DEFAULT_PAGE, |log2| 1u64 << log2);
                    *initial_memory.get_or_insert(0) += memory.initial * page;
                }
            }
            Payload::CodeSectionStart { size, .. } => sections.code = u64::from(size),
            Payload::DataSection(reader) => sections.data = span(reader.range()),
            Payload::CustomSection(reader) => sections.custom += span(reader.range()),
            _ => {}
        }
    }
    Ok((sections, initial_memory))
}

fn span(range: std::ops::Range<u64>) -> u64 {
    range.end - range.start
}

/// gzip at level 9.
fn gzip_len(bytes: &[u8]) -> u64 {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).expect("writing to a Vec");
    encoder.finish().expect("writing to a Vec").len() as u64
}

/// brotli at quality 11 with a 22-bit window, the settings of the reference
/// encoder and of `node:zlib`. This encoder is a Rust port: on the same
/// settings it lands about 0.1 % above the reference, not on the same byte.
fn brotli_len(bytes: &[u8]) -> u64 {
    let mut compressed = Vec::new();
    brotli::CompressorWriter::new(&mut compressed, 4096, 11, 22)
        .write_all(bytes)
        .expect("writing to a Vec");
    compressed.len() as u64
}

#[derive(Serialize)]
struct Job {
    artifact: PathBuf,
    glue: Option<PathBuf>,
    scenario: PathBuf,
    repeat: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerResult {
    after_instantiate: u64,
    after_run: Vec<u64>,
    instantiate_ms: f64,
}

fn run_scenario(
    config: &Config,
    module: &Module,
    scenario: &Scenario,
) -> Result<ScenarioResult, String> {
    let absolute = |path: &Path| {
        std::path::absolute(config.dir.join(path))
            .map_err(|error| format!("cannot resolve {}: {error}", path.display()))
    };
    let job = Job {
        artifact: absolute(&module.artifact)?,
        glue: module.glue.as_deref().map(absolute).transpose()?,
        scenario: absolute(&scenario.file)?,
        repeat: scenario.repeat,
    };

    let output = Command::new("node")
        .args(["--input-type=module", "-e", RUNNER])
        .env(
            "PEAKMARK_JOB",
            serde_json::to_string(&job).expect("a job is plain data"),
        )
        .current_dir(&config.dir)
        .output()
        .map_err(|error| format!("cannot start node, which scenarios run in: {error}"))?;

    let name = scenario.file.display();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("scenario {name} failed: {}", stderr.trim()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("PEAKMARK_RESULT "))
        .ok_or_else(|| format!("scenario {name} ended without a result"))?;
    let result: RunnerResult = serde_json::from_str(line)
        .map_err(|error| format!("scenario {name} returned an unreadable result: {error}"))?;

    // Linear memory never shrinks, so what is held after the last run is the peak.
    let first = result.after_run[0];
    let last = result.after_run[result.after_run.len() - 1];
    let later_runs = result.after_run.len() as u64 - 1;
    Ok(ScenarioResult {
        file: scenario.file.clone(),
        after_instantiate: result.after_instantiate,
        peak_memory: last,
        // The first run may fill caches and pools; growth is what the runs after it add.
        growth_per_run: (later_runs > 0).then(|| (last - first) / later_runs),
        instantiate_ms: result.instantiate_ms,
    })
}
