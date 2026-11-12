use runemark::Verdict;
use serde_json::json as object;

use crate::check::{Change, Level, ModuleResult, Reason, verdict};
use crate::metric::Values;
use crate::policy::Violation;
use crate::snapshot::Drift;

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
                "baseline_inventory": result.baseline.and_then(|entry| entry.inventory.as_ref()),
                "initial_memory": result.measured.initial_memory,
                "sections": result.measured.sections,
                "inventory": result.measured.inventory,
                "scenarios": result.measured.scenarios,
                "issues": issues,
                "changes": result.changes.iter().map(change).collect::<Vec<_>>(),
                "violations": result.violations.iter().map(violation).collect::<Vec<_>>(),
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
        Change::Interface(drift) => drift_detail(drift),
        Change::InventoryNotAccepted => object!({}),
    };
    object!({ "rule": change.rule(), "level": level, "detail": detail })
}

fn violation(violation: &Violation) -> serde_json::Value {
    object!({
        "rule": violation.rule(),
        "level": "error",
        "message": violation.message(),
        "action": violation.action(),
        "detail": violation,
    })
}

/// The evidence of an inventory difference: what it is, the identity of the
/// declaration, and its accepted (`was`) and current (`now`) type.
fn drift_detail(drift: &Drift) -> serde_json::Value {
    match drift {
        Drift::Added { part, item } => object!({
            "part": part.key(), "module": item.module, "name": item.name,
            "kind": item.kind, "now": item.ty,
        }),
        Drift::Removed { part, item } => object!({
            "part": part.key(), "module": item.module, "name": item.name,
            "kind": item.kind, "was": item.ty,
        }),
        Drift::Changed { part, item, was } => object!({
            "part": part.key(), "module": item.module, "name": item.name,
            "kind": item.kind, "was": was, "now": item.ty,
        }),
        Drift::FeatureAdded(feature) | Drift::FeatureRemoved(feature) => {
            object!({ "feature": feature })
        }
    }
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
