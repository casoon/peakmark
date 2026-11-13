//! The command line: parsing arguments, running a command, and turning the
//! outcome into an exit code (0 within budget, 1 over budget, 2 could not run).

mod args;
mod commands;

use std::process::ExitCode;

use runemark::{ColorMode, Console, ErrorBlock};

pub use args::{Cli, Command, Parsed, parse};

use crate::baseline;
use crate::config;
use crate::error::Error;

/// Exit code when peakmark could not produce a verdict.
const COULD_NOT_RUN: u8 = 2;

/// Runs peakmark with the arguments after the program name and returns the
/// exit code.
pub fn main(args: impl Iterator<Item = String>) -> ExitCode {
    let cli = match parse(args) {
        Ok(Parsed::Run(cli)) => cli,
        Ok(Parsed::Print(text)) => {
            println!("{text}");
            return ExitCode::SUCCESS;
        }
        Err(error) => return fail(ColorMode::Auto, &error.into(), Some("peakmark --help")),
    };
    let color = cli.color;
    match run(cli) {
        Ok(code) => code,
        Err(error) => fail(color, &error, None),
    }
}

fn fail(color: ColorMode, error: &Error, command: Option<&str>) -> ExitCode {
    let mut block = ErrorBlock::new(error.to_string());
    if let Some(command) = command {
        block = block.with_remedy("See the usage:").add_command(command);
    }
    eprint!("{}", block.render(Console::stderr(color)));
    ExitCode::from(COULD_NOT_RUN)
}

/// Runs a parsed command.
///
/// # Errors
///
/// Fails if the configuration or baseline cannot be loaded, or the command
/// cannot measure a module.
pub fn run(cli: Cli) -> Result<ExitCode, Error> {
    let console = Console::stdout(cli.color);
    let config = config::load(&cli.config)?;
    let mut baseline = baseline::load(&config.dir)?;

    match cli.command {
        Command::Check { details, json } => {
            commands::check(&config, &baseline, details, json, console)
        }
        Command::Accept { modules, reason } => commands::accept(
            &config,
            &cli.config,
            &mut baseline,
            &modules,
            &reason,
            console,
        ),
    }
}
