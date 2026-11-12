//! Comparing what was measured with the budget and the baseline.

use std::path::PathBuf;

use runemark::Verdict;

use crate::baseline::Entry;
use crate::compress::SETTINGS;
use crate::config::Module;
use crate::measure::Measured;
use crate::metric::Metric;
use crate::runner::{RUNNER_SCHEMA, Unobserved, WARMUP_RUNS};

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Over the budget: the gate fails.
    Error,
    /// Under the budget, but past the baseline by more than the tolerance, or
    /// the baseline no longer covers what is measured.
    Warning,
    /// Worth knowing, never changes the verdict.
    Note,
}

/// One metric that is over its budget or past its baseline.
#[derive(Debug)]
pub struct Issue {
    /// The metric.
    pub metric: Metric,
    /// Whether it fails the gate.
    pub level: Level,
    /// The measured value.
    pub value: u64,
    /// The budget for an error, the baseline value for a warning.
    pub limit: u64,
    /// The scenario a memory value comes from.
    pub scenario: Option<PathBuf>,
}

/// Why a measurement cannot be compared with the baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The scenario script is not the one that was accepted.
    ScenarioEdited,
    /// The scenario runs more or fewer times than when it was accepted.
    RepeatChanged {
        /// The accepted number of runs.
        was: u32,
        /// The number of runs now.
        now: u32,
    },
    /// gzip or brotli ran with other settings than when the baseline was accepted.
    Compression,
    /// Memory is measured by another protocol or warm-up than when the baseline was accepted.
    Runner,
}

/// Where the baseline and the measurement no longer line up, apart from a
/// value that grew. Every change but a [`Change::RuntimeChanged`] stays a
/// warning until the baseline is accepted again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The baseline has a scenario that is no longer configured.
    ScenarioRemoved {
        /// The scenario file as accepted.
        scenario: String,
    },
    /// A scenario is measured that the baseline has no values for.
    ScenarioNew {
        /// The scenario file as configured.
        scenario: String,
    },
    /// The baseline has a value for the metric, and nothing is measured for it now.
    MetricMissing {
        /// The metric.
        metric: Metric,
    },
    /// Values are measured but cannot be compared with the baseline.
    Incomparable {
        /// The scenario; `None` if the whole module is affected.
        scenario: Option<String>,
        /// Why.
        reason: Reason,
    },
    /// Part of what a scenario is meant to measure was not observed. The values
    /// cover less than the module does.
    Unobserved {
        /// The scenario file as configured.
        scenario: String,
        /// What was not observed.
        what: Unobserved,
    },
    /// The scenarios ran in another Node version than when the baseline was accepted.
    /// The comparison stays valid: memory is read from the instance.
    RuntimeChanged {
        /// The accepted version.
        was: String,
        /// The version now.
        now: String,
    },
}

impl Change {
    /// The rule id a finding about this change carries.
    #[must_use]
    pub fn rule(&self) -> &'static str {
        match self {
            Change::ScenarioRemoved { .. } => "baseline/scenario-removed",
            Change::ScenarioNew { .. } => "baseline/scenario-new",
            Change::MetricMissing { .. } => "baseline/metric-missing",
            Change::Incomparable { .. } => "baseline/incomparable",
            Change::Unobserved { .. } => "memory/coverage",
            Change::RuntimeChanged { .. } => "baseline/runtime",
        }
    }

    /// Whether the change is a warning or only a note.
    #[must_use]
    pub fn level(&self) -> Level {
        match self {
            Change::Unobserved { .. } | Change::RuntimeChanged { .. } => Level::Note,
            _ => Level::Warning,
        }
    }
}

/// The outcome of measuring and judging one module.
pub struct ModuleResult<'a> {
    /// The configured module.
    pub module: &'a Module,
    /// What was measured.
    pub measured: Measured,
    /// The accepted values, if any were accepted.
    pub baseline: Option<&'a Entry>,
    /// Every metric that is over budget or past the baseline.
    pub issues: Vec<Issue>,
    /// Where the baseline and the measurement no longer line up.
    pub changes: Vec<Change>,
}

impl ModuleResult<'_> {
    /// Whether any metric is over its budget.
    #[must_use]
    pub fn over_budget(&self) -> bool {
        self.issues.iter().any(|issue| issue.level == Level::Error)
    }

    /// Whether any metric is past its baseline by more than the tolerance.
    #[must_use]
    pub fn grown(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.level == Level::Warning)
    }

    /// Whether the baseline lost or never had coverage of what is measured,
    /// or holds values that cannot be compared. Accepting the baseline again
    /// resolves it.
    #[must_use]
    pub fn drifted(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.level() == Level::Warning)
    }
}

/// Judges every measured metric of `module` against its budget and baseline.
///
/// The budget is judged on the module-wide values. Against the baseline, size
/// is judged on the module, memory per scenario, so growth shows even where
/// another scenario holds more. A baseline from format 1 has no scenarios;
/// then the module-wide memory values are compared, as they were.
#[must_use]
pub fn evaluate<'a>(
    module: &'a Module,
    measured: Measured,
    baseline: Option<&'a Entry>,
    tolerance: f64,
) -> ModuleResult<'a> {
    let mut issues: Vec<_> = Metric::ALL
        .iter()
        .filter_map(|&metric| {
            let value = measured.values.get(metric)?;
            let (level, limit) = judge(
                value,
                module.budget.get(metric),
                baseline.and_then(|entry| module_baseline(entry, metric)),
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
    let mut changes = Vec::new();
    for scenario in &measured.scenarios {
        for what in &scenario.coverage.unobserved {
            changes.push(Change::Unobserved {
                scenario: scenario.id(),
                what: what.clone(),
            });
        }
    }
    if let Some(entry) = baseline {
        compare_scenarios(entry, &measured, tolerance, &mut issues, &mut changes);
        compare_module(entry, &measured, &mut changes);
    }
    ModuleResult {
        module,
        measured,
        baseline,
        issues,
        changes,
    }
}

fn compression_differs(entry: &Entry) -> bool {
    entry
        .contract
        .as_ref()
        .is_some_and(|c| c.compression != SETTINGS)
}

fn runner_differs(entry: &Entry) -> bool {
    entry
        .contract
        .as_ref()
        .is_some_and(|c| c.runner_schema != RUNNER_SCHEMA || c.warmup_runs != WARMUP_RUNS)
}

/// The accepted module-wide value `metric` is compared with, if it can be.
fn module_baseline(entry: &Entry, metric: Metric) -> Option<u64> {
    let comparable = match metric {
        Metric::Raw => true,
        Metric::Gzip | Metric::Brotli => !compression_differs(entry),
        Metric::PeakMemory | Metric::GrowthPerRun => {
            entry.scenarios.is_empty() && !runner_differs(entry)
        }
    };
    comparable.then(|| entry.values.get(metric)).flatten()
}

fn compare_module(entry: &Entry, measured: &Measured, changes: &mut Vec<Change>) {
    if compression_differs(entry) {
        changes.push(Change::Incomparable {
            scenario: None,
            reason: Reason::Compression,
        });
    }
    if runner_differs(entry) {
        changes.push(Change::Incomparable {
            scenario: None,
            reason: Reason::Runner,
        });
    }
    for metric in Metric::ALL {
        if entry.values.get(metric).is_some() && measured.values.get(metric).is_none() {
            changes.push(Change::MetricMissing { metric });
        }
    }
    let was = entry.contract.as_ref().and_then(|c| c.node.as_ref());
    let now = measured.scenarios.first().map(|s| &s.node);
    if let (Some(was), Some(now)) = (was, now)
        && was != now
    {
        changes.push(Change::RuntimeChanged {
            was: was.clone(),
            now: now.clone(),
        });
    }
}

fn compare_scenarios(
    entry: &Entry,
    measured: &Measured,
    tolerance: f64,
    issues: &mut Vec<Issue>,
    changes: &mut Vec<Change>,
) {
    for id in entry.scenarios.keys() {
        if !measured.scenarios.iter().any(|s| &s.id() == id) {
            changes.push(Change::ScenarioRemoved {
                scenario: id.clone(),
            });
        }
    }
    let runner_differs = runner_differs(entry);
    for scenario in &measured.scenarios {
        let id = scenario.id();
        let Some(accepted) = entry.scenarios.get(&id) else {
            changes.push(Change::ScenarioNew { scenario: id });
            continue;
        };
        let reason = if accepted.sha256 != scenario.sha256 {
            Some(Reason::ScenarioEdited)
        } else if accepted.repeat != scenario.repeat {
            Some(Reason::RepeatChanged {
                was: accepted.repeat,
                now: scenario.repeat,
            })
        } else {
            None
        };
        if let Some(reason) = reason {
            changes.push(Change::Incomparable {
                scenario: Some(id),
                reason,
            });
            continue;
        }
        if runner_differs {
            continue;
        }
        let values = [
            (
                Metric::PeakMemory,
                Some(scenario.peak_memory),
                Some(accepted.peak_memory),
            ),
            (
                Metric::GrowthPerRun,
                scenario.growth_per_run,
                accepted.growth_per_run,
            ),
        ];
        for (metric, value, accepted) in values {
            if let (Some(value), Some(accepted)) = (value, accepted)
                && let Some((level, limit)) = judge(value, None, Some(accepted), tolerance)
            {
                issues.push(Issue {
                    metric,
                    level,
                    value,
                    limit,
                    scenario: Some(scenario.file.clone()),
                });
            }
        }
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

/// The verdict over all modules: failed if any is over budget, a warning if
/// any grew, has no baseline yet or no longer lines up with its baseline.
#[must_use]
pub fn verdict(results: &[ModuleResult]) -> Verdict {
    if results.iter().any(ModuleResult::over_budget) {
        Verdict::Failed
    } else if results
        .iter()
        .any(|result| result.grown() || result.drifted() || result.baseline.is_none())
    {
        Verdict::Warning
    } else {
        Verdict::Passed
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::metric::Values;
    use crate::runner::{Coverage, ScenarioResult};
    use crate::wasm::Sections;

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

    #[test]
    fn a_value_at_its_budget_is_within_it() {
        assert_eq!(judge(100, Some(100), None, 0.01), None);
    }

    #[test]
    fn without_budget_or_baseline_nothing_is_judged() {
        assert_eq!(judge(100, None, None, 0.01), None);
    }

    const PAGE: u64 = 65_536;

    fn scenario(file: &str, pages: u64, repeat: u32, sha: &str) -> ScenarioResult {
        ScenarioResult {
            file: PathBuf::from(file),
            after_instantiate: PAGE,
            after_run: vec![pages * PAGE; repeat as usize],
            warmup_growth: pages * PAGE - PAGE,
            memories: Vec::new(),
            coverage: Coverage {
                instances: 1,
                unrelated_instances: 0,
                unobserved: Vec::new(),
            },
            peak_memory: pages * PAGE,
            growth_per_run: (repeat > 1).then_some(0),
            instantiate_ms: 1.0,
            repeat,
            sha256: sha.to_string(),
            node: "v22.0.0".to_string(),
        }
    }

    fn measured(scenarios: Vec<ScenarioResult>) -> Measured {
        Measured {
            values: Values {
                raw: Some(1_000),
                gzip: Some(500),
                brotli: Some(400),
                peak_memory: scenarios.iter().map(|s| s.peak_memory).max(),
                growth_per_run: scenarios.iter().filter_map(|s| s.growth_per_run).max(),
            },
            sections: Sections::default(),
            initial_memory: None,
            inventory: crate::inventory::Inventory::default(),
            scenarios,
            artifact_sha256: "artifact".to_string(),
        }
    }

    fn module() -> Module {
        Module {
            name: "engine".to_string(),
            artifact: PathBuf::from("engine.wasm"),
            glue: None,
            budget: Values::default(),
            scenarios: Vec::new(),
        }
    }

    fn accepted(scenarios: Vec<ScenarioResult>) -> Entry {
        Entry::from_measured(&measured(scenarios), "test")
    }

    fn two() -> Vec<ScenarioResult> {
        vec![scenario("a.mjs", 8, 3, "a"), scenario("b.mjs", 1, 3, "b")]
    }

    #[test]
    fn growth_of_one_scenario_shows_below_the_maximum_of_another() {
        let module = module();
        let entry = accepted(two());
        let now = measured(vec![
            scenario("a.mjs", 8, 3, "a"),
            scenario("b.mjs", 4, 3, "b"),
        ]);
        assert_eq!(now.values.peak_memory, entry.values.peak_memory);

        let result = evaluate(&module, now, Some(&entry), 0.01);

        assert_eq!(result.issues.len(), 1);
        let issue = &result.issues[0];
        assert_eq!(issue.metric, Metric::PeakMemory);
        assert_eq!(issue.level, Level::Warning);
        assert_eq!(issue.scenario.as_deref(), Some(Path::new("b.mjs")));
        assert_eq!((issue.value, issue.limit), (4 * PAGE, PAGE));
        assert_eq!(verdict(&[result]), Verdict::Warning);
    }

    #[test]
    fn removing_the_scenarios_reports_what_the_baseline_covered() {
        let module = module();
        let entry = accepted(two());

        let result = evaluate(&module, measured(Vec::new()), Some(&entry), 0.01);

        assert!(result.issues.is_empty());
        assert_eq!(
            result.changes,
            [
                Change::ScenarioRemoved {
                    scenario: "a.mjs".to_string()
                },
                Change::ScenarioRemoved {
                    scenario: "b.mjs".to_string()
                },
                Change::MetricMissing {
                    metric: Metric::PeakMemory
                },
                Change::MetricMissing {
                    metric: Metric::GrowthPerRun
                },
            ]
        );
        assert_eq!(verdict(&[result]), Verdict::Warning);
    }

    #[test]
    fn a_scenario_without_accepted_values_is_new() {
        let module = module();
        let entry = accepted(vec![scenario("a.mjs", 8, 3, "a")]);

        let result = evaluate(&module, measured(two()), Some(&entry), 0.01);

        assert_eq!(
            result.changes,
            [Change::ScenarioNew {
                scenario: "b.mjs".to_string()
            }]
        );
    }

    #[test]
    fn a_renamed_scenario_is_one_removed_and_one_new() {
        let module = module();
        let entry = accepted(vec![scenario("old.mjs", 2, 3, "a")]);

        let result = evaluate(
            &module,
            measured(vec![scenario("new.mjs", 2, 3, "a")]),
            Some(&entry),
            0.01,
        );

        assert_eq!(
            result.changes,
            [
                Change::ScenarioRemoved {
                    scenario: "old.mjs".to_string()
                },
                Change::ScenarioNew {
                    scenario: "new.mjs".to_string()
                },
            ]
        );
    }

    #[test]
    fn a_changed_repeat_or_script_is_not_compared_and_says_why() {
        let module = module();
        let entry = accepted(vec![
            scenario("a.mjs", 1, 3, "a"),
            scenario("b.mjs", 1, 3, "b"),
        ]);
        let now = measured(vec![
            scenario("a.mjs", 5, 10, "a"),
            scenario("b.mjs", 5, 3, "edited"),
        ]);

        let result = evaluate(&module, now, Some(&entry), 0.01);

        assert!(result.issues.is_empty(), "{:?}", result.issues);
        assert_eq!(
            result.changes,
            [
                Change::Incomparable {
                    scenario: Some("a.mjs".to_string()),
                    reason: Reason::RepeatChanged { was: 3, now: 10 }
                },
                Change::Incomparable {
                    scenario: Some("b.mjs".to_string()),
                    reason: Reason::ScenarioEdited
                },
            ]
        );
    }

    #[test]
    fn a_changed_artifact_digest_and_a_new_node_do_not_make_values_incomparable() {
        let module = module();
        let entry = accepted(two());
        let mut now = measured(two());
        now.artifact_sha256 = "another build".to_string();
        for scenario in &mut now.scenarios {
            scenario.node = "v24.0.0".to_string();
        }

        let result = evaluate(&module, now, Some(&entry), 0.01);

        assert!(!result.drifted());
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].level(), Level::Note);
        assert_eq!(verdict(&[result]), Verdict::Passed);
    }

    #[test]
    fn a_budget_still_applies_to_a_scenario_that_cannot_be_compared() {
        let entry = accepted(vec![scenario("a.mjs", 1, 3, "a")]);
        let mut module = module();
        module.budget.peak_memory = Some(2 * PAGE);

        let result = evaluate(
            &module,
            measured(vec![scenario("a.mjs", 5, 3, "edited")]),
            Some(&entry),
            0.01,
        );

        assert!(result.over_budget());
        assert!(result.drifted());
    }

    #[test]
    fn a_format_1_entry_is_compared_on_the_module_maximum_and_its_scenarios_are_new() {
        let module = module();
        let mut entry = accepted(two());
        entry.contract = None;
        entry.scenarios.clear();
        let now = measured(vec![
            scenario("a.mjs", 9, 3, "a"),
            scenario("b.mjs", 1, 3, "b"),
        ]);

        let result = evaluate(&module, now, Some(&entry), 0.01);

        assert_eq!(result.issues.len(), 1);
        assert_eq!(
            result.issues[0].scenario.as_deref(),
            Some(Path::new("a.mjs"))
        );
        assert_eq!(result.changes.len(), 2);
        assert!(
            result
                .changes
                .iter()
                .all(|change| matches!(change, Change::ScenarioNew { .. }))
        );
    }

    #[test]
    fn other_compression_settings_or_runner_make_only_the_affected_values_incomparable() {
        let module = module();
        let mut entry = accepted(two());
        let contract = entry.contract.as_mut().unwrap();
        contract.compression = "gzip-6".to_string();
        contract.runner_schema = 0;
        entry.values.gzip = Some(1);
        entry.values.raw = Some(1);
        let mut now = measured(two());
        now.scenarios[1].peak_memory *= 4;

        let result = evaluate(&module, now, Some(&entry), 0.01);

        let metrics: Vec<_> = result.issues.iter().map(|issue| issue.metric).collect();
        assert_eq!(metrics, [Metric::Raw]);
        assert_eq!(
            result.changes,
            [
                Change::Incomparable {
                    scenario: None,
                    reason: Reason::Compression
                },
                Change::Incomparable {
                    scenario: None,
                    reason: Reason::Runner
                },
            ]
        );
    }
}
