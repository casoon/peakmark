//! The one error type every fallible function of the crate returns.
//!
//! Each variant carries what went wrong and where; `Display` renders the
//! sentence the command line prints, so the text lives here and nowhere else.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::baseline;
use crate::config::ConfigError;
use crate::units::format_duration;

/// Everything that stops peakmark from producing a verdict.
///
/// A verdict of "over budget" is not an error: it is a result with exit code 1.
/// An `Error` always maps to exit code 2, "could not run".
#[derive(Debug)]
pub enum Error {
    /// The command line could not be understood.
    Usage(UsageError),
    /// A file could not be read.
    Read {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A file could not be written.
    Write {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A path could not be made absolute.
    Resolve {
        /// The path.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The configuration is not valid TOML or has unknown keys.
    ConfigSyntax {
        /// The configuration file.
        path: PathBuf,
        /// The cause.
        source: toml::de::Error,
    },
    /// The configuration is readable but contradicts itself.
    Config {
        /// The configuration file.
        path: PathBuf,
        /// The cause.
        source: ConfigError,
    },
    /// The baseline file is not valid JSON of the expected shape.
    BaselineSyntax {
        /// The baseline file.
        path: PathBuf,
        /// The cause.
        source: serde_json::Error,
    },
    /// The baseline file declares a format version this peakmark does not read.
    BaselineVersion {
        /// The baseline file.
        path: PathBuf,
        /// The version it declares.
        found: u32,
    },
    /// The artifact is not a valid WebAssembly module.
    InvalidModule {
        /// The artifact.
        path: PathBuf,
        /// The cause.
        source: wasmparser::BinaryReaderError,
    },
    /// The artifact is valid but uses a WebAssembly feature peakmark does not support.
    UnsupportedFeature {
        /// The artifact.
        path: PathBuf,
        /// What the validator rejected.
        source: wasmparser::BinaryReaderError,
    },
    /// The artifact is a WebAssembly component; only core modules are measured.
    UnsupportedComponent {
        /// The artifact.
        path: PathBuf,
    },
    /// Node, which scenarios run in, could not be started.
    NodeStart(std::io::Error),
    /// A scenario exited with an error.
    ScenarioFailed {
        /// The scenario file as configured.
        scenario: PathBuf,
        /// The end of what the scenario wrote to stderr.
        stderr: String,
    },
    /// A scenario did not finish within its deadline and was stopped.
    ScenarioTimeout {
        /// The module the scenario drives.
        module: String,
        /// The scenario file as configured.
        scenario: PathBuf,
        /// The deadline that passed.
        timeout: Duration,
        /// The end of what the scenario wrote to stderr before it was stopped.
        stderr: String,
    },
    /// Waiting for a scenario's Node process failed.
    ScenarioWait {
        /// The scenario file as configured.
        scenario: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A scenario finished without printing a result.
    ScenarioNoResult {
        /// The scenario file as configured.
        scenario: PathBuf,
    },
    /// A scenario printed a result that could not be parsed.
    ScenarioUnreadable {
        /// The scenario file as configured.
        scenario: PathBuf,
        /// The cause.
        source: serde_json::Error,
    },
    /// `accept` named a module the configuration does not define.
    UnknownModule {
        /// The module name from the command line.
        name: String,
        /// The configuration file.
        config: PathBuf,
    },
    /// `accept` was asked to store values that exceed the budget.
    OverBudget {
        /// The module name.
        module: String,
        /// The configuration file.
        config: PathBuf,
    },
    /// `accept` was asked to store a module that breaks its policy.
    PolicyViolated {
        /// The module name.
        module: String,
        /// The configuration file.
        config: PathBuf,
    },
}

/// A command line that does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageError {
    /// An option that takes a value was last.
    MissingValue(&'static str),
    /// `--color` got something other than `auto`, `always` or `never`.
    BadColor(String),
    /// An option peakmark does not have.
    UnknownOption(String),
    /// A command peakmark does not have.
    UnknownCommand(String),
    /// `check` got a positional argument.
    CheckTakesNoArgument(String),
    /// `accept` got both or neither of module names and `--all`.
    AcceptSelection,
    /// `accept` got no `--reason`, or an empty one.
    AcceptNeedsReason,
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(option) => write!(f, "{option} needs a value"),
            Self::BadColor(other) => {
                write!(f, "--color takes auto, always or never, not `{other}`")
            }
            Self::UnknownOption(option) => write!(f, "unknown option `{option}`"),
            Self::UnknownCommand(command) => write!(f, "unknown command `{command}`"),
            Self::CheckTakesNoArgument(extra) => {
                write!(f, "check takes no argument, got `{extra}`")
            }
            Self::AcceptSelection => write!(f, "accept takes either module names or --all"),
            Self::AcceptNeedsReason => {
                write!(f, "accept needs --reason: a baseline changes on purpose")
            }
        }
    }
}

impl std::error::Error for UsageError {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(error) => error.fmt(f),
            Self::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Write { path, source } => write!(f, "cannot write {}: {source}", path.display()),
            Self::Resolve { path, source } => {
                write!(f, "cannot resolve {}: {source}", path.display())
            }
            Self::ConfigSyntax { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Config { path, source } => write!(f, "{}: {source}", path.display()),
            Self::BaselineSyntax { path, source } => write!(f, "{}: {source}", path.display()),
            Self::BaselineVersion { path, found } => {
                let (oldest, newest) = (baseline::OLDEST_VERSION, baseline::VERSION);
                write!(f, "{} has format version {found}; ", path.display())?;
                if *found > newest {
                    write!(
                        f,
                        "this peakmark reads versions {oldest} to {newest}. \
                         The file was written by a newer peakmark: update peakmark"
                    )
                } else {
                    write!(
                        f,
                        "this peakmark reads versions {oldest} to {newest}. \
                         Delete the file and run `peakmark accept --all --reason \"...\"` \
                         to accept the current values again"
                    )
                }
            }
            Self::InvalidModule { path, source } => write!(
                f,
                "{} is not a valid WebAssembly module: {source}",
                path.display()
            ),
            Self::UnsupportedFeature { path, source } => write!(
                f,
                "{} uses a WebAssembly feature peakmark does not support: {source}",
                path.display()
            ),
            Self::UnsupportedComponent { path } => write!(
                f,
                "{} is a WebAssembly component; peakmark measures core modules only",
                path.display()
            ),
            Self::NodeStart(source) => {
                write!(f, "cannot start node, which scenarios run in: {source}")
            }
            Self::ScenarioFailed { scenario, stderr } => {
                write!(
                    f,
                    "scenario {} failed: {}",
                    scenario.display(),
                    stderr.trim()
                )
            }
            Self::ScenarioTimeout {
                module,
                scenario,
                timeout,
                stderr,
            } => {
                write!(
                    f,
                    "scenario {} of module `{module}` did not finish within {} and was stopped; \
                     raise `timeout` of the scenario or `scenario_timeout` if it needs longer",
                    scenario.display(),
                    format_duration(*timeout)
                )?;
                if !stderr.trim().is_empty() {
                    write!(f, "\nstderr: {}", stderr.trim())?;
                }
                Ok(())
            }
            Self::ScenarioWait { scenario, source } => write!(
                f,
                "cannot wait for scenario {}: {source}",
                scenario.display()
            ),
            Self::ScenarioNoResult { scenario } => {
                write!(f, "scenario {} ended without a result", scenario.display())
            }
            Self::ScenarioUnreadable { scenario, source } => write!(
                f,
                "scenario {} returned an unreadable result: {source}",
                scenario.display()
            ),
            Self::UnknownModule { name, config } => {
                write!(f, "no module `{name}` in {}", config.display())
            }
            Self::OverBudget { module, config } => write!(
                f,
                "`{module}` is over its budget; raise the budget in {} before accepting",
                config.display()
            ),
            Self::PolicyViolated { module, config } => write!(
                f,
                "`{module}` breaks its policy; accepting does not waive it, so fix the module or change [module.policy] in {}",
                config.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Usage(error) => Some(error),
            Self::Read { source, .. }
            | Self::Write { source, .. }
            | Self::Resolve { source, .. }
            | Self::NodeStart(source)
            | Self::ScenarioWait { source, .. } => Some(source),
            Self::ConfigSyntax { source, .. } => Some(source),
            Self::Config { source, .. } => Some(source),
            Self::BaselineSyntax { source, .. } | Self::ScenarioUnreadable { source, .. } => {
                Some(source)
            }
            Self::InvalidModule { source, .. } | Self::UnsupportedFeature { source, .. } => {
                Some(source)
            }
            Self::BaselineVersion { .. }
            | Self::UnsupportedComponent { .. }
            | Self::ScenarioFailed { .. }
            | Self::ScenarioTimeout { .. }
            | Self::ScenarioNoResult { .. }
            | Self::UnknownModule { .. }
            | Self::OverBudget { .. }
            | Self::PolicyViolated { .. } => None,
        }
    }
}

impl From<UsageError> for Error {
    fn from(error: UsageError) -> Self {
        Self::Usage(error)
    }
}
