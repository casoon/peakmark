//! The one error type every fallible function of the crate returns.
//!
//! Each variant carries what went wrong and where; `Display` renders the
//! sentence the command line prints, so the text lives here and nowhere else.

use std::fmt;
use std::path::PathBuf;

use crate::config::ConfigError;

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
    /// The artifact is not a valid WebAssembly module.
    InvalidModule {
        /// The artifact.
        path: PathBuf,
        /// The cause.
        source: wasmparser::BinaryReaderError,
    },
    /// Node, which scenarios run in, could not be started.
    NodeStart(std::io::Error),
    /// A scenario exited with an error.
    ScenarioFailed {
        /// The scenario file as configured.
        scenario: PathBuf,
        /// What the scenario wrote to stderr.
        stderr: String,
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
            Self::InvalidModule { path, source } => write!(
                f,
                "{} is not a valid WebAssembly module: {source}",
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
            | Self::NodeStart(source) => Some(source),
            Self::ConfigSyntax { source, .. } => Some(source),
            Self::Config { source, .. } => Some(source),
            Self::BaselineSyntax { source, .. } | Self::ScenarioUnreadable { source, .. } => {
                Some(source)
            }
            Self::InvalidModule { source, .. } => Some(source),
            Self::ScenarioFailed { .. }
            | Self::ScenarioNoResult { .. }
            | Self::UnknownModule { .. }
            | Self::OverBudget { .. } => None,
        }
    }
}

impl From<UsageError> for Error {
    fn from(error: UsageError) -> Self {
        Self::Usage(error)
    }
}
