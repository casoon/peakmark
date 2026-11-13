use std::path::PathBuf;

use runemark::ColorMode;

use crate::error::UsageError;

pub(super) const USAGE: &str = "\
peakmark — gate WebAssembly modules on size and memory

Usage:
  peakmark check [--details] [--json]
  peakmark accept (<module>... | --all) --reason <text>

Commands:
  check     Measure every module and compare it with its budget and baseline
  accept    Store the measured values as the new baseline

Options:
      --config <file>   Configuration file [default: peakmark.toml]
      --details         check: also list the measured values
      --json            check: print JSON instead of the report
      --all             accept: every configured module
      --reason <text>   accept: why the baseline changes
      --color <mode>    auto, always or never [default: auto]
  -h, --help            Show this help
  -V, --version         Show the version

Exit codes: 0 within budget, 1 over budget, 2 could not run.";

/// What the user asked peakmark to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Measure every module and judge it.
    Check {
        /// List the measured values too.
        details: bool,
        /// Print JSON instead of the report.
        json: bool,
    },
    /// Store the measured values as the new baseline.
    Accept {
        /// The modules to accept; empty means all of them.
        modules: Vec<String>,
        /// Why the baseline changes.
        reason: String,
    },
}

/// A command with the options every command shares.
#[derive(Debug)]
pub struct Cli {
    /// What to do.
    pub command: Command,
    /// The configuration file.
    pub config: PathBuf,
    /// Whether output is colored.
    pub color: ColorMode,
}

/// The outcome of parsing the command line.
#[derive(Debug)]
pub enum Parsed {
    /// Run a command.
    Run(Cli),
    /// Print this text and exit successfully: help or the version.
    Print(String),
}

/// Parses the arguments after the program name.
///
/// # Errors
///
/// Fails if an option is unknown or lacks its value, the command is unknown, or
/// the arguments do not fit the command.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Parsed, UsageError> {
    let mut command = None;
    let mut positional = Vec::new();
    let mut config = PathBuf::from("peakmark.toml");
    let mut color = ColorMode::Auto;
    let (mut details, mut json, mut all) = (false, false, false);
    let mut reason = None;

    let value = |args: &mut dyn Iterator<Item = String>, option: &'static str| {
        args.next().ok_or(UsageError::MissingValue(option))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Print(USAGE.to_string())),
            "-V" | "--version" => {
                return Ok(Parsed::Print(format!(
                    "peakmark {}",
                    env!("CARGO_PKG_VERSION")
                )));
            }
            "--details" => details = true,
            "--json" => json = true,
            "--all" => all = true,
            "--config" => config = value(&mut args, "--config")?.into(),
            "--reason" => reason = Some(value(&mut args, "--reason")?),
            "--color" => {
                color = match value(&mut args, "--color")?.as_str() {
                    "auto" => ColorMode::Auto,
                    "always" => ColorMode::Always,
                    "never" => ColorMode::Never,
                    other => return Err(UsageError::BadColor(other.to_string())),
                }
            }
            option if option.starts_with('-') => {
                return Err(UsageError::UnknownOption(option.to_string()));
            }
            _ if command.is_none() => command = Some(arg),
            _ => positional.push(arg),
        }
    }

    let command = match command.as_deref() {
        Some("check") => {
            if let Some(extra) = positional.first() {
                return Err(UsageError::CheckTakesNoArgument(extra.clone()));
            }
            Command::Check { details, json }
        }
        Some("accept") => {
            if all != positional.is_empty() {
                return Err(UsageError::AcceptSelection);
            }
            let reason = reason
                .filter(|reason| !reason.trim().is_empty())
                .ok_or(UsageError::AcceptNeedsReason)?;
            Command::Accept {
                modules: positional,
                reason,
            }
        }
        Some(other) => return Err(UsageError::UnknownCommand(other.to_string())),
        None => return Ok(Parsed::Print(USAGE.to_string())),
    };
    Ok(Parsed::Run(Cli {
        command,
        config,
        color,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Parsed, UsageError> {
        parse(args.iter().map(ToString::to_string))
    }

    fn run(args: &[&str]) -> Cli {
        match parse_args(args).unwrap() {
            Parsed::Run(cli) => cli,
            Parsed::Print(text) => panic!("expected a command, got `{text}`"),
        }
    }

    #[test]
    fn check_takes_its_flags_in_any_order() {
        let cli = run(&[
            "--json",
            "check",
            "--details",
            "--config",
            "a/peakmark.toml",
        ]);
        assert_eq!(
            cli.command,
            Command::Check {
                details: true,
                json: true
            }
        );
        assert_eq!(cli.config, PathBuf::from("a/peakmark.toml"));
    }

    #[test]
    fn accept_needs_modules_or_all_and_a_reason() {
        let cli = run(&["accept", "engine", "ui", "--reason", "new feature"]);
        assert_eq!(
            cli.command,
            Command::Accept {
                modules: vec!["engine".to_string(), "ui".to_string()],
                reason: "new feature".to_string()
            }
        );
        assert_eq!(
            parse_args(&["accept", "--reason", "x"]).unwrap_err(),
            UsageError::AcceptSelection
        );
        assert_eq!(
            parse_args(&["accept", "engine", "--all", "--reason", "x"]).unwrap_err(),
            UsageError::AcceptSelection
        );
        assert_eq!(
            parse_args(&["accept", "--all"]).unwrap_err(),
            UsageError::AcceptNeedsReason
        );
        assert_eq!(
            parse_args(&["accept", "--all", "--reason", "  "]).unwrap_err(),
            UsageError::AcceptNeedsReason
        );
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        assert_eq!(
            parse_args(&["check", "--nope"]).unwrap_err(),
            UsageError::UnknownOption("--nope".to_string())
        );
        assert_eq!(
            parse_args(&["bogus"]).unwrap_err(),
            UsageError::UnknownCommand("bogus".to_string())
        );
        assert_eq!(
            parse_args(&["check", "extra"]).unwrap_err(),
            UsageError::CheckTakesNoArgument("extra".to_string())
        );
        assert_eq!(
            parse_args(&["check", "--color", "pink"]).unwrap_err(),
            UsageError::BadColor("pink".to_string())
        );
        assert_eq!(
            parse_args(&["check", "--config"]).unwrap_err(),
            UsageError::MissingValue("--config")
        );
    }

    #[test]
    fn help_version_and_no_command_print_instead_of_running() {
        assert!(matches!(parse_args(&[]).unwrap(), Parsed::Print(text) if text == USAGE));
        assert!(matches!(parse_args(&["-h"]).unwrap(), Parsed::Print(text) if text == USAGE));
        assert!(matches!(
            parse_args(&["--version"]).unwrap(),
            Parsed::Print(text) if text.starts_with("peakmark ")
        ));
    }
}
