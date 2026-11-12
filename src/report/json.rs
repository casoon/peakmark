use runemark::Verdict;
use serde_json::json as object;

use crate::check::{Level, ModuleResult, verdict};
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
                "initial_memory": result.measured.initial_memory,
                "sections": result.measured.sections,
                "scenarios": result.measured.scenarios,
                "issues": issues,
            })
        })
        .collect();
    object!({ "verdict": verdict, "modules": modules })
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
