use runemark::Verdict;
use serde_json::json as object;

use crate::check::{Change, Level, ModuleResult, Reason, verdict};
use crate::metric::Values;

/// The machine-readable result of a check, the document `--json` prints.
#[must_use]
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
                    object!({
                        "rule": issue.metric.rule(),
                        "level": match issue.level {
                            Level::Error => "error",
                            Level::Warning => "warning",
                            Level::Note => "note",
                        },
                        "value": issue.value,
                        "limit": issue.limit,
                        "scenario": issue.scenario,
                    })
                })
                .collect();
            object!({
                "name": result.module.name,
                "values": values(&result.measured.values),
                "budget": values(&result.module.budget),
                "baseline": result.baseline.map(|entry| values(&entry.values)),
                "baseline_scenarios": result.baseline.map(|entry| &entry.scenarios),
                "baseline_contract": result.baseline.and_then(|entry| entry.contract.as_ref()),
                "initial_memory": result.measured.initial_memory,
                "sections": result.measured.sections,
                "scenarios": result.measured.scenarios,
                "issues": issues,
                "changes": result.changes.iter().map(change).collect::<Vec<_>>(),
            })
        })
        .collect();
    object!({ "verdict": verdict, "modules": modules })
}

fn change(change: &Change) -> serde_json::Value {
    let level = match change.level() {
        Level::Note => "note",
        _ => "warning",
    };
    let detail = match change {
        Change::ScenarioRemoved { scenario } | Change::ScenarioNew { scenario } => {
            object!({ "scenario": scenario })
        }
        Change::MetricMissing { metric } => object!({ "metric": metric.rule() }),
        Change::Incomparable { scenario, reason } => {
            let mut detail = match reason {
                Reason::ScenarioEdited => object!({ "reason": "scenario_edited" }),
                Reason::RepeatChanged { was, now } => {
                    object!({ "reason": "repeat_changed", "was": was, "now": now })
                }
                Reason::Compression => object!({ "reason": "compression" }),
                Reason::Runner => object!({ "reason": "runner" }),
            };
            detail["scenario"] = object!(scenario);
            detail
        }
        Change::Unobserved { scenario, what } => {
            object!({ "scenario": scenario, "unobserved": what })
        }
        Change::RuntimeChanged { was, now } => object!({ "was": was, "now": now }),
    };
    object!({ "rule": change.rule(), "level": level, "detail": detail })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_results_is_a_passing_document_without_modules() {
        let document = json(&[]);
        assert_eq!(document["verdict"], "passed");
        assert_eq!(document["modules"].as_array().unwrap().len(), 0);
    }
}
