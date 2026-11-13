use runemark::Verdict;
use serde_json::json as object;

use crate::check::{Change, Level, ModuleResult, Reason, verdict};
use crate::inventory::{FunctionBody, Inventory};
use crate::metric::Values;
use crate::policy::Violation;
use crate::reach::{Node, ReachReport};
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
                "reach": reach(&result.measured.inventory, &result.measured.reach),
                "scenarios": result.measured.scenarios,
                "issues": issues,
                "changes": result.changes.iter().map(change).collect::<Vec<_>>(),
                "violations": result.violations.iter().map(violation).collect::<Vec<_>>(),
                "explanations": result.explanations.listed,
                "explanations_omitted": result.explanations.omitted,
            })
        })
        .collect();
    object!({ "verdict": verdict, "modules": modules })
}

/// Why the largest functions are in the module. Functions are given by index in
/// this file, with their name and exports; the index means nothing in another build.
fn reach(inventory: &Inventory, report: &ReachReport) -> serde_json::Value {
    let function = |index: u32| {
        let body = inventory.functions.body(index);
        object!({
            "index": index,
            "label": body.map_or_else(|| format!("func[{index}]"), FunctionBody::label),
            "size": body.map(|body| body.size),
        })
    };
    let node = |node: Node| match node {
        Node::Function(index) => object!({ "function": function(index) }),
        Node::Table => object!("table"),
    };
    let explanations: Vec<_> = report
        .explanations
        .iter()
        .map(|explanation| {
            let path: Vec<_> = explanation
                .path
                .iter()
                .map(|hop| object!({ "node": node(hop.node), "edge": hop.edge }))
                .collect();
            let listed: Vec<_> = explanation
                .callers
                .listed
                .iter()
                .map(|caller| object!({ "node": node(caller.node), "edge": caller.edge }))
                .collect();
            object!({
                "function": function(explanation.function),
                "reachability": explanation.reachability,
                "path": path,
                "callers": { "count": explanation.callers.count, "listed": listed },
            })
        })
        .collect();
    object!({
        "assumptions": report.assumptions,
        "coverage": report.coverage,
        "functions": explanations,
    })
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
        Change::InventoryNotAccepted | Change::SizesNotAccepted => object!({}),
        Change::Sections { drift, action } => {
            let mut detail = serde_json::to_value(drift).expect("plain data");
            detail["basis"] =
                object!("payload and header bytes of the file; the parts add up to the file size");
            detail["action"] = object!(action);
            detail
        }
        Change::Functions(drift) => {
            let mut detail = serde_json::to_value(drift).expect("plain data");
            detail["basis"] =
                object!("raw shallow body bytes; not additive to gzip or Brotli sizes");
            detail
        }
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
