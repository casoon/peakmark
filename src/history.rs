//! The accepted baselines of each module over time, read from the history log,
//! and what changed from one to the next.
//!
//! It only reads what `accept` wrote: no module is measured, nothing is judged
//! and the baseline is not touched. A difference between two entries is a
//! [`Delta`]; where the two were measured under different settings it is
//! marked as not comparable instead of being shown as growth or shrinkage.

use crate::baseline::{Accepted, Contract, Entry};
use crate::metric::Metric;

/// The accepted entries of one module, oldest first.
#[derive(Debug, Clone)]
pub struct Timeline {
    /// The module name.
    pub module: String,
    /// The entries shown, each with what changed since the entry before it.
    pub steps: Vec<Step>,
    /// How many entries the log has for the module, shown or not.
    pub accepted: usize,
}

/// One accepted entry and how it differs from the one accepted before it.
#[derive(Debug, Clone)]
pub struct Step {
    /// The entry as the log keeps it.
    pub entry: Entry,
    /// The module values that differ from the previous entry; empty for the
    /// first entry of the module and when nothing changed.
    pub deltas: Vec<Delta>,
    /// Whether an earlier entry exists, shown or not.
    pub has_previous: bool,
}

/// A module value that differs between two accepted entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delta {
    /// The value that differs.
    pub metric: Metric,
    /// The value in the previous entry; `None` if it was not measured.
    pub was: Option<u64>,
    /// The value in this entry; `None` if it is no longer measured.
    pub now: Option<u64>,
    /// Whether the two were measured the same way, so that the difference is a
    /// change of the module and not of the measurement.
    pub comparable: bool,
}

/// The timelines of the history log: one per module, in the order the modules
/// first appear in it. With `modules`, only those, in that order; a module
/// without an entry gets an empty timeline. With `since`, only entries
/// accepted on or after that `YYYY-MM-DD` day are listed, but each one is
/// still compared with the entry before it.
#[must_use]
pub fn timelines(history: Vec<Accepted>, modules: &[String], since: Option<&str>) -> Vec<Timeline> {
    let mut all: Vec<(String, Vec<Entry>)> = Vec::new();
    for Accepted { module, entry } in history {
        match all.iter_mut().find(|(name, _)| *name == module) {
            Some((_, entries)) => entries.push(entry),
            None => all.push((module, vec![entry])),
        }
    }
    let chosen: Vec<(String, Vec<Entry>)> = if modules.is_empty() {
        all
    } else {
        modules
            .iter()
            .map(|name| {
                let entries = all
                    .iter()
                    .find(|(module, _)| module == name)
                    .map_or_default(|(_, entries)| entries.clone());
                (name.clone(), entries)
            })
            .collect()
    };
    chosen
        .into_iter()
        .map(|(module, entries)| {
            let steps = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| since.is_none_or(|day| entry.accepted.as_str() >= day))
                .map(|(index, entry)| {
                    let previous = index.checked_sub(1).map(|before| &entries[before]);
                    Step {
                        entry: entry.clone(),
                        deltas: previous.map_or_default(|was| deltas(was, entry)),
                        has_previous: previous.is_some(),
                    }
                })
                .collect();
            Timeline {
                module,
                steps,
                accepted: entries.len(),
            }
        })
        .collect()
}

/// The module values that differ between `was` and `now`.
fn deltas(was: &Entry, now: &Entry) -> Vec<Delta> {
    Metric::ALL
        .into_iter()
        .filter_map(|metric| {
            let (before, after) = (was.values.get(metric), now.values.get(metric));
            (before != after).then(|| Delta {
                metric,
                was: before,
                now: after,
                comparable: comparable(metric, was, now),
            })
        })
        .collect()
}

/// Whether `metric` was measured the same way in both entries. An entry
/// without a contract (format 1) is taken as measured the same way, as `check`
/// does. The memory values are the highest over the scenarios, so they only
/// compare when the same scenarios ran.
fn comparable(metric: Metric, was: &Entry, now: &Entry) -> bool {
    let both = was.contract.as_ref().zip(now.contract.as_ref());
    match metric {
        Metric::Raw => true,
        Metric::Gzip | Metric::Brotli => both.is_none_or(|(a, b)| a.compression == b.compression),
        Metric::PeakMemory | Metric::GrowthPerRun => {
            both.is_none_or(|(a, b)| runner(a) == runner(b)) && same_scenarios(was, now)
        }
    }
}

/// The parts of the contract that decide how memory is measured.
fn runner(contract: &Contract) -> (u32, u32) {
    (contract.runner_schema, contract.warmup_runs)
}

fn same_scenarios(was: &Entry, now: &Entry) -> bool {
    // Entries from format 1 list no scenarios; they are compared on their values.
    was.scenarios.is_empty()
        || now.scenarios.is_empty()
        || was.scenarios.keys().eq(now.scenarios.keys())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::baseline::ScenarioEntry;
    use crate::metric::Values;

    fn entry(accepted: &str, brotli: u64, peak: u64) -> Entry {
        Entry {
            accepted: accepted.to_string(),
            reason: format!("on {accepted}"),
            values: Values {
                raw: Some(brotli * 3),
                brotli: Some(brotli),
                peak_memory: Some(peak),
                ..Values::default()
            },
            contract: None,
            scenarios: BTreeMap::new(),
            inventory: None,
        }
    }

    fn line(module: &str, entry: Entry) -> Accepted {
        Accepted {
            module: module.to_string(),
            entry,
        }
    }

    fn contract(compression: &str, runner_schema: u32) -> Contract {
        Contract {
            runner_schema,
            warmup_runs: 1,
            compression: compression.to_string(),
            node: None,
            artifact_sha256: String::new(),
        }
    }

    fn scenario() -> ScenarioEntry {
        ScenarioEntry {
            sha256: String::new(),
            repeat: 1,
            peak_memory: 0,
            growth_per_run: None,
            after_instantiate: None,
            after_run: Vec::new(),
            unobserved: Vec::new(),
        }
    }

    #[test]
    fn groups_by_module_in_order_of_appearance_and_compares_with_the_entry_before() {
        let history = vec![
            line("engine", entry("2026-01-01", 100, 65_536)),
            line("ui", entry("2026-01-02", 50, 65_536)),
            line("engine", entry("2026-01-03", 120, 65_536)),
        ];
        let timelines = timelines(history, &[], None);
        let names: Vec<_> = timelines.iter().map(|t| t.module.as_str()).collect();
        assert_eq!(names, ["engine", "ui"]);

        let engine = &timelines[0].steps;
        assert!(engine[0].deltas.is_empty() && !engine[0].has_previous);
        assert_eq!(
            engine[1].deltas,
            [
                Delta {
                    metric: Metric::Raw,
                    was: Some(300),
                    now: Some(360),
                    comparable: true
                },
                Delta {
                    metric: Metric::Brotli,
                    was: Some(100),
                    now: Some(120),
                    comparable: true
                },
            ]
        );
    }

    #[test]
    fn since_hides_older_entries_but_keeps_the_comparison_with_them() {
        let history = vec![
            line("engine", entry("2026-01-01", 100, 65_536)),
            line("engine", entry("2026-02-01", 100, 131_072)),
        ];
        let timelines = timelines(history, &[], Some("2026-01-15"));
        let steps = &timelines[0].steps;
        assert_eq!(steps.len(), 1);
        assert!(steps[0].has_previous);
        assert_eq!(steps[0].deltas[0].metric, Metric::PeakMemory);
    }

    #[test]
    fn named_modules_come_in_the_order_asked_and_may_have_no_entry() {
        let history = vec![
            line("engine", entry("2026-01-01", 100, 0)),
            line("ui", entry("2026-01-01", 50, 0)),
        ];
        let asked = ["ui".to_string(), "worker".to_string()];
        let timelines = timelines(history, &asked, None);
        assert_eq!(timelines[0].module, "ui");
        assert_eq!(timelines[0].steps.len(), 1);
        assert_eq!(timelines[1].module, "worker");
        assert!(timelines[1].steps.is_empty());
    }

    #[test]
    fn other_settings_or_scenarios_make_only_the_affected_values_incomparable() {
        let mut was = entry("2026-01-01", 100, 65_536);
        was.contract = Some(contract("gzip 9, brotli 11", 1));
        was.scenarios.insert("a.mjs".to_string(), scenario());

        let mut compression = entry("2026-01-02", 110, 131_072);
        compression.contract = Some(contract("gzip 6, brotli 11", 1));
        compression
            .scenarios
            .insert("a.mjs".to_string(), scenario());
        let changed = deltas(&was, &compression);
        let comparable = |metric| {
            changed
                .iter()
                .find(|d| d.metric == metric)
                .unwrap()
                .comparable
        };
        assert!(comparable(Metric::Raw));
        assert!(!comparable(Metric::Brotli));
        assert!(comparable(Metric::PeakMemory));

        let mut scenarios = entry("2026-01-03", 100, 131_072);
        scenarios.contract = Some(contract("gzip 9, brotli 11", 1));
        scenarios.scenarios.insert("b.mjs".to_string(), scenario());
        let changed = deltas(&was, &scenarios);
        assert!(!changed[0].comparable, "{changed:?}");

        let mut runner = entry("2026-01-04", 100, 131_072);
        runner.contract = Some(contract("gzip 9, brotli 11", 2));
        runner.scenarios.insert("a.mjs".to_string(), scenario());
        assert!(!deltas(&was, &runner)[0].comparable);
    }

    #[test]
    fn an_entry_without_a_contract_is_compared_on_its_values() {
        let was = entry("2026-01-01", 100, 0);
        let mut now = entry("2026-01-02", 90, 0);
        now.contract = Some(contract("gzip 6, brotli 5", 9));
        assert!(deltas(&was, &now).iter().all(|delta| delta.comparable));
    }
}
