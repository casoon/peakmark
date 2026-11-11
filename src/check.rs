//! Comparing what was measured with the budget and the baseline, and turning
//! the outcome into a report or JSON.

use std::path::PathBuf;

use runemark::{
    Finding, FindingGroup, Location, Metric as ReportMetric, NextStep, Report, Tone, Verdict,
};
use serde_json::json;

use crate::baseline::Entry;
use crate::config::Module;
use crate::measure::Measured;
use crate::metric::{Metric, Values};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Over the budget: the gate fails.
    Error,
    /// Under the budget, but past the baseline by more than the tolerance.
    Warning,
}

#[derive(Debug)]
pub struct Issue {
    pub metric: Metric,
    pub level: Level,
    pub value: u64,
    /// The budget for an error, the baseline value for a warning.
    pub limit: u64,
    pub scenario: Option<PathBuf>,
}

pub struct ModuleResult<'a> {
    pub module: &'a Module,
    pub measured: Measured,
    pub baseline: Option<&'a Entry>,
    pub issues: Vec<Issue>,
}

impl ModuleResult<'_> {
    pub fn over_budget(&self) -> bool {
        self.issues.iter().any(|issue| issue.level == Level::Error)
    }

    fn grown(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.level == Level::Warning)
    }
}

pub fn evaluate<'a>(
    module: &'a Module,
    measured: Measured,
    baseline: Option<&'a Entry>,
    tolerance: f64,
) -> ModuleResult<'a> {
    let issues = Metric::ALL
        .iter()
        .filter_map(|&metric| {
            let value = measured.values.get(metric)?;
            let (level, limit) = judge(
                value,
                module.budget.get(metric),
                baseline.and_then(|entry| entry.values.get(metric)),
                tolerance,
            )?;
            Some(Issue {
                metric,
                level,
                value,
                limit,
                scenario: measured.scenario_for(metric).map(PathBuf::from),
            })
        })
        .collect();
    ModuleResult {
        module,
        measured,
        baseline,
        issues,
    }
}

fn judge(
    value: u64,
    budget: Option<u64>,
    baseline: Option<u64>,
    tolerance: f64,
) -> Option<(Level, u64)> {
    if let Some(budget) = budget.filter(|&budget| value > budget) {
        return Some((Level::Error, budget));
    }
    let baseline = baseline.filter(|&baseline| value > baseline)?;
    ((value - baseline) as f64 > baseline as f64 * tolerance).then_some((Level::Warning, baseline))
}

pub fn verdict(results: &[ModuleResult]) -> Verdict {
    if results.iter().any(ModuleResult::over_budget) {
        Verdict::Failed
    } else if results
        .iter()
        .any(|result| result.grown() || result.baseline.is_none())
    {
        Verdict::Warning
    } else {
        Verdict::Passed
    }
}

pub fn report(results: &[ModuleResult], details: bool) -> Report {
    let names = |pick: fn(&ModuleResult) -> bool| -> Vec<&str> {
        results
            .iter()
            .filter(|result| pick(result))
            .map(|result| result.module.name.as_str())
            .collect()
    };
    let over = names(|result| result.over_budget());
    let grown = names(|result| !result.over_budget() && result.grown());
    let unset = names(|result| result.baseline.is_none());

    let title = if !over.is_empty() {
        format!("over budget: {}", over.join(", "))
    } else if !grown.is_empty() {
        format!("grown past the baseline: {}", grown.join(", "))
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
        .add_metric(count("Grown", grown.len(), Tone::Warning));

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
        }
        report = report.add_group(measured);
    }

    for result in results {
        let mut group = FindingGroup::new(&result.module.name);
        for issue in &result.issues {
            group = group.add_finding(finding(issue));
        }
        if result.baseline.is_none() {
            group = group.add_finding(
                Finding::new(Tone::Warning, "No baseline accepted yet")
                    .with_rule_id("baseline/missing"),
            );
        }
        if !result.issues.is_empty() || result.baseline.is_none() {
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
        .filter(|result| !result.over_budget() && (result.grown() || result.baseline.is_none()))
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

fn finding(issue: &Issue) -> Finding {
    let metric = issue.metric;
    let label = capitalize(metric.label());
    let value = metric.format(issue.value);
    let (tone, message) = match issue.level {
        Level::Error => (
            Tone::Error,
            format!("{label} {value}, budget {}", metric.format(issue.limit)),
        ),
        Level::Warning => (
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

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn json(results: &[ModuleResult]) -> serde_json::Value {
    let verdict = match verdict(results) {
        Verdict::Failed => "failed",
        Verdict::Warning => "warning",
        _ => "passed",
    };
    let values = |values: &Values| serde_json::to_value(values).expect("plain data");
    let modules: Vec<_> = results
        .iter()
        .map(|result| {
            let issues: Vec<_> = result
                .issues
                .iter()
                .map(|issue| {
                    json!({
                        "rule": issue.metric.rule(),
                        "level": match issue.level {
                            Level::Error => "error",
                            Level::Warning => "warning",
                        },
                        "value": issue.value,
                        "limit": issue.limit,
                        "scenario": issue.scenario,
                    })
                })
                .collect();
            json!({
                "name": result.module.name,
                "values": values(&result.measured.values),
                "budget": values(&result.module.budget),
                "baseline": result.baseline.map(|entry| values(&entry.values)),
                "initial_memory": result.measured.initial_memory,
                "sections": result.measured.sections,
                "scenarios": result.measured.scenarios,
                "issues": issues,
            })
        })
        .collect();
    json!({ "verdict": verdict, "modules": modules })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_over_the_budget_fails_whatever_the_baseline_says() {
        assert_eq!(
            judge(101, Some(100), Some(200), 0.01),
            Some((Level::Error, 100))
        );
    }

    #[test]
    fn growth_past_the_tolerance_warns_and_growth_within_it_does_not() {
        assert_eq!(
            judge(1020, None, Some(1000), 0.01),
            Some((Level::Warning, 1000))
        );
        assert_eq!(judge(1010, None, Some(1000), 0.01), None);
        assert_eq!(judge(900, Some(1000), Some(1000), 0.01), None);
    }

    #[test]
    fn any_growth_from_a_baseline_of_zero_warns() {
        assert_eq!(judge(1, None, Some(0), 0.01), Some((Level::Warning, 0)));
    }
}
