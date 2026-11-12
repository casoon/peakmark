use runemark::{Finding, FindingGroup, Location, Metric as ReportMetric, NextStep, Report, Tone};

use crate::check::{Change, Issue, Level, ModuleResult, Reason, verdict};
use crate::runner::{ScenarioResult, Unobserved};
use crate::units::format_binary;

/// The report `check` prints. With `details`, it also lists every measured
/// value.
#[must_use]
pub fn report(results: &[ModuleResult], details: bool) -> Report {
    let names = |pick: fn(&ModuleResult) -> bool| -> Vec<&str> {
        results
            .iter()
            .filter(|result| pick(result))
            .map(|result| result.module.name.as_str())
            .collect()
    };
    // A method path cannot stand in for this closure: it would pin the lifetime
    // of `ModuleResult`, which the `fn` pointer type leaves open.
    #[allow(clippy::redundant_closure_for_method_calls)]
    let over = names(|result| result.over_budget());
    let grown = names(|result| !result.over_budget() && result.grown());
    let drifted = names(|result| !result.over_budget() && !result.grown() && result.drifted());
    let unset = names(|result| result.baseline.is_none());

    let title = if !over.is_empty() {
        format!("over budget: {}", over.join(", "))
    } else if !grown.is_empty() {
        format!("grown past the baseline: {}", grown.join(", "))
    } else if !drifted.is_empty() {
        format!("baseline no longer matches: {}", drifted.join(", "))
    } else if !unset.is_empty() {
        format!("no baseline for {}", unset.join(", "))
    } else if results.len() == 1 {
        "1 module within budget and baseline".to_string()
    } else {
        format!("{} modules within budget and baseline", results.len())
    };

    let count = |key: &str, value: usize, tone: Tone| {
        let metric = ReportMetric::new(key, value.to_string());
        if value > 0 {
            metric.with_tone(tone)
        } else {
            metric
        }
    };
    let mut report = Report::new(format!("peakmark check: {title}"), verdict(results))
        .add_metric(ReportMetric::new("Modules", results.len().to_string()))
        .add_metric(count("Over budget", over.len(), Tone::Error))
        .add_metric(count("Grown", grown.len(), Tone::Warning))
        .add_metric(count(
            "Baseline drift",
            results.iter().filter(|result| result.drifted()).count(),
            Tone::Warning,
        ));

    if details {
        let mut measured = FindingGroup::new("Measured");
        for result in results {
            measured = measured.add_finding(Finding::new(
                Tone::Info,
                format!(
                    "{}: {}",
                    result.module.name,
                    result.measured.values.summary()
                ),
            ));
            for scenario in &result.measured.scenarios {
                measured = measured.add_finding(Finding::new(Tone::Info, trace(scenario)));
            }
        }
        report = report.add_group(measured);
    }

    for result in results {
        let mut group = FindingGroup::new(&result.module.name);
        for issue in &result.issues {
            group = group.add_finding(finding(issue));
        }
        for change in &result.changes {
            group = group.add_finding(change_finding(change));
        }
        if result.baseline.is_none() {
            group = group.add_finding(
                Finding::new(Tone::Warning, "No baseline accepted yet")
                    .with_rule_id("baseline/missing"),
            );
        }
        if !result.issues.is_empty() || !result.changes.is_empty() || result.baseline.is_none() {
            report = report.add_group(group);
        }
    }

    if !over.is_empty() {
        report = report.add_next_step(NextStep::new(
            "Shrink the module, or raise its budget in the configuration on purpose",
        ));
    }
    let to_accept: Vec<&str> = results
        .iter()
        .filter(|result| {
            !result.over_budget()
                && (result.grown() || result.drifted() || result.baseline.is_none())
        })
        .map(|result| result.module.name.as_str())
        .collect();
    if !to_accept.is_empty() {
        report = report.add_next_step(
            NextStep::new("Accept the measured values as the new baseline").with_command(format!(
                "peakmark accept {} --reason \"...\"",
                to_accept.join(" ")
            )),
        );
    }
    report
}

/// One line per scenario: the capacity trace split into warm-up and later growth.
/// It states what the numbers are, not whether anything leaks.
fn trace(scenario: &ScenarioResult) -> String {
    let later = match scenario.growth_per_run {
        Some(growth) => format!(", then {} per run", format_binary(growth)),
        None => String::new(),
    };
    format!(
        "{}: capacity {} after instantiate, +{} in the first run{later} ({} runs, page steps of 64 KiB)",
        scenario.id(),
        format_binary(scenario.after_instantiate),
        format_binary(scenario.warmup_growth),
        scenario.repeat,
    )
}

fn finding(issue: &Issue) -> Finding {
    let metric = issue.metric;
    let label = capitalize(metric.label());
    let value = metric.format(issue.value);
    let (tone, message) = match issue.level {
        Level::Error => (
            Tone::Error,
            format!("{label} {value}, budget {}", metric.format(issue.limit)),
        ),
        Level::Warning | Level::Note => (
            Tone::Warning,
            format!(
                "{label} {value}, +{} over the baseline",
                metric.format(issue.value - issue.limit)
            ),
        ),
    };
    let finding = Finding::new(tone, message).with_rule_id(metric.rule());
    match &issue.scenario {
        Some(scenario) => finding.with_location(Location::file(scenario)),
        None => finding,
    }
}

fn change_finding(change: &Change) -> Finding {
    let message = match change {
        Change::ScenarioRemoved { scenario } => {
            format!("Scenario {scenario} is in the baseline and is no longer measured")
        }
        Change::ScenarioNew { scenario } => {
            format!("Scenario {scenario} has no accepted values yet")
        }
        Change::MetricMissing { metric } => format!(
            "{} was accepted and is not measured now",
            capitalize(metric.label())
        ),
        Change::Incomparable { scenario, reason } => {
            let why = match reason {
                Reason::ScenarioEdited => "its script changed since it was accepted".to_string(),
                Reason::RepeatChanged { was, now } => {
                    format!("it runs {now} times, accepted with {was}")
                }
                Reason::Compression => {
                    "gzip and brotli ran with other settings than the baseline".to_string()
                }
                Reason::Runner => "memory is measured differently than at the baseline".to_string(),
            };
            match (scenario, reason) {
                (Some(scenario), _) => format!("Scenario {scenario} is not compared: {why}"),
                (None, Reason::Compression) => format!("Sizes are not compared: {why}"),
                (None, _) => format!("Memory is not compared: {why}"),
            }
        }
        Change::Unobserved { scenario, what } => {
            let what = match what {
                Unobserved::InternalMemory { count } => format!(
                    "{count} declared memor{} neither exported nor imported, so not measured",
                    if *count == 1 { "y is" } else { "ies are" }
                ),
                Unobserved::Workers { count } => format!(
                    "{count} worker{} started, and a module run there is not measured",
                    if *count == 1 { "" } else { "s" }
                ),
            };
            format!("Scenario {scenario}: {what}")
        }
        Change::RuntimeChanged { was, now } => {
            format!("Node {now}, accepted with {was}; values stay comparable")
        }
    };
    let tone = match change.level() {
        Level::Note => Tone::Info,
        _ => Tone::Warning,
    };
    Finding::new(tone, message).with_rule_id(change.rule())
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::metric::Metric;

    #[test]
    fn capitalises_the_first_letter_only() {
        assert_eq!(capitalize("peak memory"), "Peak memory");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn an_error_names_the_budget_and_a_warning_the_growth() {
        let error = Issue {
            metric: Metric::PeakMemory,
            level: Level::Error,
            value: 256 * 1024,
            limit: 128 * 1024,
            scenario: Some(PathBuf::from("steady.mjs")),
        };
        let warning = Issue {
            metric: Metric::Raw,
            level: Level::Warning,
            value: 2_000,
            limit: 1_000,
            scenario: None,
        };
        let error = format!("{:?}", finding(&error));
        let warning = format!("{:?}", finding(&warning));
        assert!(
            error.contains("Peak memory 256.0 KiB, budget 128.0 KiB"),
            "{error}"
        );
        assert!(
            warning.contains("Raw size 2.0 kB, +1.0 kB over the baseline"),
            "{warning}"
        );
    }
}
