use std::path::Path;
use std::process::ExitCode;

use runemark::{Console, Verdict};

use crate::baseline::{self, Baseline};
use crate::check;
use crate::config::{Config, Module};
use crate::date::today;
use crate::error::Error;
use crate::measure::measure;
use crate::report;

/// Exit code of a check that found a metric over its budget.
const OVER_BUDGET: u8 = 1;

pub(super) fn check(
    config: &Config,
    baseline: &Baseline,
    details: bool,
    json: bool,
    console: Console,
) -> Result<ExitCode, Error> {
    let mut results = Vec::new();
    for module in &config.modules {
        let measured = measure(config, module)?;
        results.push(check::evaluate(
            module,
            measured,
            baseline.modules.get(&module.name),
            config.tolerance,
        ));
    }
    if json {
        let text =
            serde_json::to_string_pretty(&report::json(&results)).expect("a result is plain data");
        println!("{text}");
    } else {
        print!("{}", report::report(&results, details).render(console));
    }
    Ok(if check::verdict(&results) == Verdict::Failed {
        ExitCode::from(OVER_BUDGET)
    } else {
        ExitCode::SUCCESS
    })
}

pub(super) fn accept(
    config: &Config,
    config_path: &Path,
    baseline: &mut Baseline,
    modules: &[String],
    reason: &str,
    console: Console,
) -> Result<ExitCode, Error> {
    let chosen = choose(config, config_path, modules)?;

    let mut accepted = Vec::new();
    for module in chosen {
        let measured = measure(config, module)?;
        let result = check::evaluate(module, measured, None, config.tolerance);
        if result.over_budget() {
            return Err(Error::OverBudget {
                module: module.name.clone(),
                config: config_path.to_path_buf(),
            });
        }
        accepted.push((
            module.name.clone(),
            baseline::Entry {
                accepted: today(),
                reason: reason.to_string(),
                values: result.measured.values,
            },
        ));
    }

    for (name, entry) in &accepted {
        baseline.modules.insert(name.clone(), entry.clone());
    }
    baseline::save(&config.dir, baseline)?;
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

/// The named modules, or every module if none is named.
fn choose<'a>(
    config: &'a Config,
    config_path: &Path,
    names: &[String],
) -> Result<Vec<&'a Module>, Error> {
    if names.is_empty() {
        return Ok(config.modules.iter().collect());
    }
    names
        .iter()
        .map(|name| {
            config
                .modules
                .iter()
                .find(|module| &module.name == name)
                .ok_or_else(|| Error::UnknownModule {
                    name: name.clone(),
                    config: config_path.to_path_buf(),
                })
        })
        .collect()
}
