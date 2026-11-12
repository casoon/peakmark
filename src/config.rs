//! `peakmark.toml`: the modules to gate, their budgets and their scenarios.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::error::Error;
use crate::metric::Values;
use crate::units::{ParseError, parse_bytes, parse_duration, parse_percent};

/// A parsed `peakmark.toml`.
#[derive(Debug)]
pub struct Config {
    /// The directory of the configuration file; every path is relative to it.
    pub dir: PathBuf,
    /// Growth over the baseline below this fraction is not reported.
    pub tolerance: f64,
    /// The modules to gate, in file order.
    pub modules: Vec<Module>,
}

/// One `[[module]]`.
#[derive(Debug)]
pub struct Module {
    /// The name baselines and reports use.
    pub name: String,
    /// The `.wasm` file.
    pub artifact: PathBuf,
    /// The JavaScript glue that instantiates the module, if it needs one.
    pub glue: Option<PathBuf>,
    /// What the module may reach; a missing value is not gated.
    pub budget: Values,
    /// The scenarios that measure memory.
    pub scenarios: Vec<Scenario>,
}

/// One `[[scenario]]` of a module.
#[derive(Debug)]
pub struct Scenario {
    /// The script, relative to the configuration file.
    pub file: PathBuf,
    /// How often the script runs on one instance.
    pub repeat: u32,
    /// How long the whole scenario may take, from starting Node to its result.
    pub timeout: Duration,
}

/// The deadline of a scenario that sets none and whose configuration sets no
/// `scenario_timeout`.
pub const DEFAULT_SCENARIO_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    tolerance: Option<String>,
    scenario_timeout: Option<String>,
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
    timeout: Option<String>,
}

fn one() -> u32 {
    1
}

/// A configuration that parses but cannot be used.
#[derive(Debug, PartialEq)]
pub enum ConfigError {
    /// There is no `[[module]]`.
    NoModules,
    /// Two modules share a name.
    DuplicateModule(String),
    /// A size or the tolerance does not parse.
    Value(ParseError),
    /// A scenario runs zero times.
    ZeroRepeat(PathBuf),
    /// A module lists the same scenario file twice, so its identity would not be stable.
    DuplicateScenario {
        /// The module.
        module: String,
        /// The scenario script.
        file: PathBuf,
    },
    /// A scenario or `scenario_timeout` allows no time at all.
    ZeroTimeout(String),
    /// A scenario names a module that is not configured.
    UnknownScenarioModule {
        /// The scenario script.
        file: PathBuf,
        /// The module name it gives.
        module: String,
    },
    /// A `peak_memory` budget with nothing to measure it.
    PeakMemoryWithoutScenario(String),
    /// A `growth_per_run` budget without a scenario that repeats.
    GrowthWithoutRepeat(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoModules => write!(f, "no [[module]] is configured"),
            Self::DuplicateModule(name) => write!(f, "module `{name}` is configured twice"),
            Self::Value(error) => error.fmt(f),
            Self::ZeroRepeat(file) => write!(f, "scenario {} has repeat = 0", file.display()),
            Self::DuplicateScenario { module, file } => write!(
                f,
                "module `{module}` lists scenario {} twice",
                file.display()
            ),
            Self::ZeroTimeout(what) => write!(f, "{what} has a timeout of zero"),
            Self::UnknownScenarioModule { file, module } => write!(
                f,
                "scenario {} names the unknown module `{module}`",
                file.display()
            ),
            Self::PeakMemoryWithoutScenario(name) => write!(
                f,
                "module `{name}` has a peak_memory budget but no [[scenario]] to measure it"
            ),
            Self::GrowthWithoutRepeat(name) => write!(
                f,
                "module `{name}` has a growth_per_run budget but no [[scenario]] with repeat >= 2"
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Value(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ParseError> for ConfigError {
    fn from(error: ParseError) -> Self {
        Self::Value(error)
    }
}

/// Reads and checks the configuration at `path`.
///
/// # Errors
///
/// Fails if the file cannot be read, is not valid TOML, has unknown keys or
/// contradicts itself.
pub fn load(path: &Path) -> Result<Config, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let raw: RawConfig = toml::from_str(&text).map_err(|source| Error::ConfigSyntax {
        path: path.to_path_buf(),
        source,
    })?;
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    resolve(raw, dir).map_err(|source| Error::Config {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve(raw: RawConfig, dir: PathBuf) -> Result<Config, ConfigError> {
    if raw.modules.is_empty() {
        return Err(ConfigError::NoModules);
    }
    let tolerance = match &raw.tolerance {
        Some(text) => parse_percent(text)?,
        None => 0.01,
    };
    let deadline = |text: &Option<String>, what: &dyn Fn() -> String| {
        let timeout = text.as_deref().map(parse_duration).transpose()?;
        if timeout == Some(Duration::ZERO) {
            return Err(ConfigError::ZeroTimeout(what()));
        }
        Ok(timeout)
    };
    let default_timeout = deadline(&raw.scenario_timeout, &|| "scenario_timeout".to_string())?
        .unwrap_or(DEFAULT_SCENARIO_TIMEOUT);

    let mut modules: Vec<Module> = Vec::new();
    for module in raw.modules {
        if modules.iter().any(|known| known.name == module.name) {
            return Err(ConfigError::DuplicateModule(module.name));
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
            return Err(ConfigError::ZeroRepeat(scenario.file));
        }
        let Some(module) = modules
            .iter_mut()
            .find(|module| module.name == scenario.module)
        else {
            return Err(ConfigError::UnknownScenarioModule {
                file: scenario.file,
                module: scenario.module,
            });
        };
        if module
            .scenarios
            .iter()
            .any(|known| known.file == scenario.file)
        {
            return Err(ConfigError::DuplicateScenario {
                module: module.name.clone(),
                file: scenario.file,
            });
        }
        let timeout = deadline(&scenario.timeout, &|| {
            format!("scenario {}", scenario.file.display())
        })?
        .unwrap_or(default_timeout);
        module.scenarios.push(Scenario {
            file: scenario.file,
            repeat: scenario.repeat,
            timeout,
        });
    }

    for module in &modules {
        if module.budget.peak_memory.is_some() && module.scenarios.is_empty() {
            return Err(ConfigError::PeakMemoryWithoutScenario(module.name.clone()));
        }
        if module.budget.growth_per_run.is_some()
            && !module.scenarios.iter().any(|scenario| scenario.repeat >= 2)
        {
            return Err(ConfigError::GrowthWithoutRepeat(module.name.clone()));
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

    fn parse(text: &str) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(text).expect("the fixture is valid TOML");
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

        assert!((config.tolerance - 0.02).abs() < f64::EPSILON);
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
        .unwrap_err();
        assert_eq!(
            error,
            ConfigError::PeakMemoryWithoutScenario("engine".to_string())
        );
        assert!(error.to_string().contains("no [[scenario]]"), "{error}");
    }

    #[test]
    fn refuses_a_growth_budget_without_a_repeating_scenario() {
        let error = parse(
            r#"
            [[module]]
            name = "engine"
            artifact = "engine.wasm"
            budget = { growth_per_run = "0" }

            [[scenario]]
            module = "engine"
            file = "once.mjs"
            "#,
        )
        .unwrap_err();
        assert_eq!(
            error,
            ConfigError::GrowthWithoutRepeat("engine".to_string())
        );
    }

    #[test]
    fn refuses_a_module_configured_twice_and_a_scenario_that_never_runs() {
        let twice = parse(
            "[[module]]\nname = 'a'\nartifact = 'a.wasm'\n[[module]]\nname = 'a'\nartifact = 'b.wasm'",
        )
        .unwrap_err();
        assert_eq!(twice, ConfigError::DuplicateModule("a".to_string()));

        let never = parse(
            "[[module]]\nname = 'a'\nartifact = 'a.wasm'\n[[scenario]]\nmodule = 'a'\nfile = 's.mjs'\nrepeat = 0",
        )
        .unwrap_err();
        assert_eq!(never, ConfigError::ZeroRepeat(PathBuf::from("s.mjs")));
    }

    #[test]
    fn refuses_the_same_scenario_file_twice_in_one_module() {
        let scenario = "[[scenario]]\nmodule = 'a'\nfile = 's.mjs'\n";
        let error = parse(&format!(
            "[[module]]\nname = 'a'\nartifact = 'a.wasm'\n{scenario}{scenario}"
        ))
        .unwrap_err();
        assert_eq!(
            error,
            ConfigError::DuplicateScenario {
                module: "a".to_string(),
                file: PathBuf::from("s.mjs"),
            }
        );
    }

    #[test]
    fn a_scenario_deadline_defaults_to_the_configured_one_and_can_be_overridden() {
        let module = "[[module]]\nname = 'a'\nartifact = 'a.wasm'\n";
        let scenarios = "[[scenario]]\nmodule = 'a'\nfile = 'x.mjs'\n\
                         [[scenario]]\nmodule = 'a'\nfile = 'y.mjs'\ntimeout = '5s'";

        let config = parse(&format!("{module}{scenarios}")).unwrap();
        assert_eq!(
            config.modules[0].scenarios[0].timeout,
            DEFAULT_SCENARIO_TIMEOUT
        );
        assert_eq!(
            config.modules[0].scenarios[1].timeout,
            Duration::from_secs(5)
        );

        let config = parse(&format!("scenario_timeout = '2m'\n{module}{scenarios}")).unwrap();
        assert_eq!(
            config.modules[0].scenarios[0].timeout,
            Duration::from_secs(120)
        );
        assert_eq!(
            config.modules[0].scenarios[1].timeout,
            Duration::from_secs(5)
        );
    }

    #[test]
    fn refuses_a_deadline_of_zero_and_one_that_is_not_a_duration() {
        let module = "[[module]]\nname = 'a'\nartifact = 'a.wasm'\n";
        let zero = parse(&format!("scenario_timeout = '0s'\n{module}")).unwrap_err();
        assert_eq!(
            zero,
            ConfigError::ZeroTimeout("scenario_timeout".to_string())
        );

        let text = format!("{module}[[scenario]]\nmodule = 'a'\nfile = 's.mjs'\ntimeout = 'soon'");
        let error = parse(&text).unwrap_err();
        assert!(error.to_string().contains("not a duration"), "{error}");
    }

    #[test]
    fn refuses_a_configuration_without_modules() {
        assert_eq!(
            parse("tolerance = '1%'").unwrap_err(),
            ConfigError::NoModules
        );
    }

    #[test]
    fn refuses_unknown_keys_and_unknown_modules() {
        let unknown_key = "[[module]]\nname = 'a'\nartifact = 'a.wasm'\nbudgett = {}";
        assert!(toml::from_str::<RawConfig>(unknown_key).is_err());

        let error = parse(
            "[[module]]\nname = 'a'\nartifact = 'a.wasm'\n[[scenario]]\nmodule = 'b'\nfile = 's.mjs'",
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown module `b`"), "{error}");
    }
}
