//! peakmark gates WebAssembly modules on size and memory against a baseline
//! kept in the repository.

mod baseline;
mod check;
mod config;
mod measure;
mod metric;

use std::path::PathBuf;
use std::process::ExitCode;

use runemark::{ColorMode, Console, ErrorBlock, Verdict};

const USAGE: &str = "\
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

enum Command {
    Check {
        details: bool,
        json: bool,
    },
    Accept {
        modules: Vec<String>,
        reason: String,
    },
}

struct Cli {
    command: Command,
    config: PathBuf,
    color: ColorMode,
}

enum Parsed {
    Run(Cli),
    Print(String),
}

fn main() -> ExitCode {
    let cli = match parse(std::env::args().skip(1)) {
        Ok(Parsed::Run(cli)) => cli,
        Ok(Parsed::Print(text)) => {
            println!("{text}");
            return ExitCode::SUCCESS;
        }
        Err(message) => return fail(ColorMode::Auto, &message, Some("peakmark --help")),
    };
    let color = cli.color;
    match run(cli) {
        Ok(code) => code,
        Err(message) => fail(color, &message, None),
    }
}

fn fail(color: ColorMode, message: &str, command: Option<&str>) -> ExitCode {
    let mut block = ErrorBlock::new(message);
    if let Some(command) = command {
        block = block.with_remedy("See the usage:").add_command(command);
    }
    eprint!("{}", block.render(Console::stderr(color)));
    ExitCode::from(2)
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut command = None;
    let mut positional = Vec::new();
    let mut config = PathBuf::from("peakmark.toml");
    let mut color = ColorMode::Auto;
    let (mut details, mut json, mut all) = (false, false, false);
    let mut reason = None;

    let value = |args: &mut dyn Iterator<Item = String>, option: &str| {
        args.next().ok_or_else(|| format!("{option} needs a value"))
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
                    other => {
                        return Err(format!(
                            "--color takes auto, always or never, not `{other}`"
                        ));
                    }
                }
            }
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            _ if command.is_none() => command = Some(arg),
            _ => positional.push(arg),
        }
    }

    let command = match command.as_deref() {
        Some("check") => {
            if let Some(extra) = positional.first() {
                return Err(format!("check takes no argument, got `{extra}`"));
            }
            Command::Check { details, json }
        }
        Some("accept") => {
            if all != positional.is_empty() {
                return Err("accept takes either module names or --all".to_string());
            }
            let reason = reason
                .filter(|reason| !reason.trim().is_empty())
                .ok_or("accept needs --reason: a baseline changes on purpose")?;
            Command::Accept {
                modules: positional,
                reason,
            }
        }
        Some(other) => return Err(format!("unknown command `{other}`")),
        None => return Ok(Parsed::Print(USAGE.to_string())),
    };
    Ok(Parsed::Run(Cli {
        command,
        config,
        color,
    }))
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    let console = Console::stdout(cli.color);
    let config = config::load(&cli.config)?;
    let mut baseline = baseline::load(&config.dir)?;

    match cli.command {
        Command::Check { details, json } => {
            let mut results = Vec::new();
            for module in &config.modules {
                let measured = measure::measure(&config, module)?;
                results.push(check::evaluate(
                    module,
                    measured,
                    baseline.modules.get(&module.name),
                    config.tolerance,
                ));
            }
            if json {
                let text = serde_json::to_string_pretty(&check::json(&results))
                    .expect("a result is plain data");
                println!("{text}");
            } else {
                print!("{}", check::report(&results, details).render(console));
            }
            Ok(if check::verdict(&results) == Verdict::Failed {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Accept { modules, reason } => {
            let chosen: Vec<_> = if modules.is_empty() {
                config.modules.iter().collect()
            } else {
                modules
                    .iter()
                    .map(|name| {
                        config
                            .modules
                            .iter()
                            .find(|module| &module.name == name)
                            .ok_or_else(|| {
                                format!("no module `{name}` in {}", cli.config.display())
                            })
                    })
                    .collect::<Result<_, _>>()?
            };

            let mut accepted = Vec::new();
            for module in chosen {
                let measured = measure::measure(&config, module)?;
                let result = check::evaluate(module, measured, None, config.tolerance);
                if result.over_budget() {
                    return Err(format!(
                        "`{}` is over its budget; raise the budget in {} before accepting",
                        module.name,
                        cli.config.display()
                    ));
                }
                accepted.push((
                    module.name.clone(),
                    baseline::Entry {
                        accepted: metric::today(),
                        reason: reason.clone(),
                        values: result.measured.values,
                    },
                ));
            }

            for (name, entry) in &accepted {
                baseline.modules.insert(name.clone(), entry.clone());
            }
            baseline::save(&config.dir, &baseline)?;
            for (name, entry) in &accepted {
                baseline::append_history(&config.dir, name, entry)?;
            }

            let names: Vec<_> = accepted.iter().map(|(name, _)| name.as_str()).collect();
            println!(
                "{}",
                Verdict::Passed.render(
                    console,
                    &format!("Baseline accepted for {}", names.join(", "))
                )
            );
            for (name, entry) in &accepted {
                println!("  {name}: {}", entry.values.summary());
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
