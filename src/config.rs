//! `peakmark.toml`: the modules to gate, their budgets and their scenarios.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::metric::{Values, parse_bytes, parse_percent};

pub struct Config {
    /// The directory of the configuration file; every path is relative to it.
    pub dir: PathBuf,
    /// Growth over the baseline below this fraction is not reported.
    pub tolerance: f64,
    pub modules: Vec<Module>,
}

pub struct Module {
    pub name: String,
    pub artifact: PathBuf,
    pub glue: Option<PathBuf>,
    pub budget: Values,
    pub scenarios: Vec<Scenario>,
}

pub struct Scenario {
    pub file: PathBuf,
    pub repeat: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    tolerance: Option<String>,
    #[serde(default, rename = "module")]
    modules: Vec<RawModule>,
    #[serde(default, rename = "scenario")]
    scenarios: Vec<RawScenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModule {
    name: String,
    artifact: PathBuf,
    glue: Option<PathBuf>,
    #[serde(default)]
    budget: RawBudget,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawBudget {
    raw: Option<String>,
    gzip: Option<String>,
    brotli: Option<String>,
    peak_memory: Option<String>,
    growth_per_run: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    module: String,
    file: PathBuf,
    #[serde(default = "one")]
    repeat: u32,
}

fn one() -> u32 {
    1
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let raw: RawConfig =
        toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    resolve(raw, dir).map_err(|error| format!("{}: {error}", path.display()))
}

fn resolve(raw: RawConfig, dir: PathBuf) -> Result<Config, String> {
    if raw.modules.is_empty() {
        return Err("no [[module]] is configured".to_string());
    }
    let tolerance = match &raw.tolerance {
        Some(text) => parse_percent(text)?,
        None => 0.01,
    };

    let mut modules: Vec<Module> = Vec::new();
    for module in raw.modules {
        if modules.iter().any(|known| known.name == module.name) {
            return Err(format!("module `{}` is configured twice", module.name));
        }
        let size = |text: &Option<String>| text.as_deref().map(parse_bytes).transpose();
        let budget = Values {
            raw: size(&module.budget.raw)?,
            gzip: size(&module.budget.gzip)?,
            brotli: size(&module.budget.brotli)?,
            peak_memory: size(&module.budget.peak_memory)?,
            growth_per_run: size(&module.budget.growth_per_run)?,
        };
        modules.push(Module {
            name: module.name,
            artifact: module.artifact,
            glue: module.glue,
            budget,
            scenarios: Vec::new(),
        });
    }

    for scenario in raw.scenarios {
        if scenario.repeat == 0 {
            return Err(format!(
                "scenario {} has repeat = 0",
                scenario.file.display()
            ));
        }
        let module = modules
            .iter_mut()
            .find(|module| module.name == scenario.module)
            .ok_or_else(|| {
                format!(
                    "scenario {} names the unknown module `{}`",
                    scenario.file.display(),
                    scenario.module
                )
            })?;
        module.scenarios.push(Scenario {
            file: scenario.file,
            repeat: scenario.repeat,
        });
    }

    for module in &modules {
        if module.budget.peak_memory.is_some() && module.scenarios.is_empty() {
            return Err(format!(
                "module `{}` has a peak_memory budget but no [[scenario]] to measure it",
                module.name
            ));
        }
        if module.budget.growth_per_run.is_some()
            && !module.scenarios.iter().any(|scenario| scenario.repeat >= 2)
        {
            return Err(format!(
                "module `{}` has a growth_per_run budget but no [[scenario]] with repeat >= 2",
                module.name
            ));
        }
    }

    Ok(Config {
        dir,
        tolerance,
        modules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, String> {
        let raw: RawConfig = toml::from_str(text).map_err(|error| error.to_string())?;
        resolve(raw, PathBuf::from("."))
    }

    #[test]
    fn reads_modules_budgets_and_scenarios() {
        let config = parse(
            r#"
            tolerance = "2%"

            [[module]]
            name = "engine"
            artifact = "pkg/engine_bg.wasm"

            [module.budget]
            brotli = "258 kB"
            peak_memory = "24 MiB"

            [[scenario]]
            module = "engine"
            file = "bench/pivot.mjs"
            repeat = 20
            "#,
        )
        .unwrap();

        assert_eq!(config.tolerance, 0.02);
        let engine = &config.modules[0];
        assert_eq!(engine.budget.brotli, Some(258_000));
        assert_eq!(engine.budget.peak_memory, Some(24 * 1024 * 1024));
        assert_eq!(engine.scenarios[0].repeat, 20);
    }

    #[test]
    fn refuses_a_memory_budget_nothing_can_measure() {
        let error = parse(
            r#"
            [[module]]
            name = "engine"
            artifact = "engine.wasm"
            budget = { peak_memory = "1 MiB" }
            "#,
        )
        .err()
        .unwrap();
        assert!(error.contains("no [[scenario]]"), "{error}");
    }

    #[test]
    fn refuses_unknown_keys_and_unknown_modules() {
        assert!(parse("[[module]]\nname = 'a'\nartifact = 'a.wasm'\nbudgett = {}").is_err());
        let error = parse(
            "[[module]]\nname = 'a'\nartifact = 'a.wasm'\n[[scenario]]\nmodule = 'b'\nfile = 's.mjs'",
        )
        .err()
        .unwrap();
        assert!(error.contains("unknown module `b`"), "{error}");
    }
}
