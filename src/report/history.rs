use runemark::{
    DetailLevel, Finding, FindingGroup, Metric as ReportMetric, NextStep, Report, Tone, Verdict,
};
use serde_json::json as object;

use crate::history::{Delta, Step, Timeline};

/// The report `history` prints: one group per module, its accepted entries
/// oldest first, each with what changed since the entry before it. It
/// describes what was accepted and judges nothing, so its verdict is
/// information.
#[must_use]
pub fn history_report(timelines: &[Timeline]) -> Report {
    let entries: usize = timelines.iter().map(|timeline| timeline.steps.len()).sum();
    let title = if entries == 0 {
        "peakmark history: no accepted baseline to list".to_string()
    } else {
        let names: Vec<_> = timelines.iter().map(|t| t.module.as_str()).collect();
        format!("peakmark history: {}", names.join(", "))
    };
    // Every entry is listed; a history cut to three lines would hide most of it.
    let mut report = Report::new(title, Verdict::Info)
        .with_detail_level(DetailLevel::Detailed)
        .add_metric(ReportMetric::new("Modules", timelines.len().to_string()))
        .add_metric(ReportMetric::new("Accepted", entries.to_string()));
    for timeline in timelines {
        let mut group = FindingGroup::new(&timeline.module);
        if timeline.accepted == 0 {
            group = group.add_finding(Finding::new(Tone::Muted, "No baseline accepted yet"));
        } else if timeline.steps.is_empty() {
            group = group.add_finding(Finding::new(
                Tone::Muted,
                format!(
                    "No baseline accepted since the given day; {} before it",
                    timeline.accepted
                ),
            ));
        }
        for step in &timeline.steps {
            group = group.add_finding(step_finding(step));
        }
        report = report.add_group(group);
    }
    if timelines.iter().all(|timeline| timeline.accepted == 0) {
        report = report.add_next_step(
            NextStep::new("Accept a measurement to start the history")
                .with_command("peakmark accept --all --reason \"...\""),
        );
    }
    report
}

/// One accepted entry: the day, the reason and either all its values (the
/// first entry of a module) or what changed since the entry before.
fn step_finding(step: &Step) -> Finding {
    let entry = &step.entry;
    let values = if !step.has_previous {
        entry.values.summary()
    } else if step.deltas.is_empty() {
        "gated values unchanged".to_string()
    } else {
        step.deltas
            .iter()
            .map(delta_text)
            .collect::<Vec<_>>()
            .join(", ")
    };
    // Only a value measured the same way on both sides grows or shrinks.
    let moved = |grew: bool| {
        step.deltas.iter().any(|d| match (d.was, d.now) {
            (Some(was), Some(now)) if d.comparable => (now > was) == grew && now != was,
            _ => false,
        })
    };
    let (grown, shrunk) = (moved(true), moved(false));
    let tone = match (grown, shrunk) {
        (true, _) => Tone::Warning,
        (false, true) => Tone::Success,
        (false, false) => Tone::Info,
    };
    Finding::new(
        tone,
        format!("{} \"{}\": {values}", entry.accepted, entry.reason),
    )
}

fn delta_text(delta: &Delta) -> String {
    let metric = delta.metric;
    match (delta.was, delta.now) {
        (Some(was), Some(now)) if delta.comparable => {
            let sign = if now < was { '-' } else { '+' };
            format!(
                "{} {} ({sign}{})",
                metric.label(),
                metric.format(now),
                metric.format(now.abs_diff(was))
            )
        }
        (Some(was), Some(now)) => format!(
            "{} {} (was {}, measured differently, not comparable)",
            metric.label(),
            metric.format(now),
            metric.format(was)
        ),
        (None, Some(now)) => format!("{} {} (new)", metric.label(), metric.format(now)),
        (Some(_), None) => format!("{} no longer measured", metric.label()),
        (None, None) => unreachable!("a delta has a value on at least one side"),
    }
}

/// The machine-readable history, the document `history --json` prints. Each
/// entry carries its values, its per-scenario memory, its contract and the
/// `changes` since the entry before; the inventory snapshot is left out.
#[must_use]
pub fn history_json(timelines: &[Timeline]) -> serde_json::Value {
    let modules: Vec<_> = timelines
        .iter()
        .map(|timeline| {
            let entries: Vec<_> = timeline
                .steps
                .iter()
                .map(|step| {
                    let entry = &step.entry;
                    object!({
                        "accepted": entry.accepted,
                        "reason": entry.reason,
                        "values": entry.values,
                        "scenarios": entry.scenarios,
                        "contract": entry.contract,
                        "first": !step.has_previous,
                        "changes": step.deltas.iter().map(|delta| object!({
                            "rule": delta.metric.rule(),
                            "was": delta.was,
                            "now": delta.now,
                            "comparable": delta.comparable,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            object!({ "name": timeline.module, "entries": entries })
        })
        .collect();
    object!({ "modules": modules })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::baseline::Entry;
    use crate::metric::{Metric, Values};

    fn step(reason: &str, has_previous: bool, deltas: Vec<Delta>) -> Step {
        Step {
            entry: Entry {
                accepted: "2026-11-20".to_string(),
                reason: reason.to_string(),
                values: Values {
                    brotli: Some(159_200),
                    ..Values::default()
                },
                contract: None,
                scenarios: BTreeMap::new(),
                inventory: None,
            },
            deltas,
            has_previous,
        }
    }

    fn delta(metric: Metric, was: Option<u64>, now: Option<u64>, comparable: bool) -> Delta {
        Delta {
            metric,
            was,
            now,
            comparable,
        }
    }

    #[test]
    fn the_first_entry_lists_its_values_and_a_later_one_what_changed() {
        let first = step_finding(&step("first measurement", false, Vec::new()));
        assert_eq!(
            first.message,
            "2026-11-20 \"first measurement\": brotli size 159.2 kB"
        );
        let later = step_finding(&step(
            "pivot",
            true,
            vec![
                delta(Metric::Brotli, Some(157_900), Some(159_200), true),
                delta(Metric::PeakMemory, Some(2 << 20), Some(1 << 20), true),
            ],
        ));
        assert_eq!(
            later.message,
            "2026-11-20 \"pivot\": brotli size 159.2 kB (+1.3 kB), peak memory 1.0 MiB (-1.0 MiB)"
        );
        assert_eq!(later.tone, Tone::Warning);
        let same = step_finding(&step("rebuild", true, Vec::new()));
        assert!(
            same.message.ends_with("gated values unchanged"),
            "{}",
            same.message
        );
    }

    #[test]
    fn an_incomparable_or_new_value_is_not_shown_as_growth() {
        let finding = step_finding(&step(
            "new settings",
            true,
            vec![
                delta(Metric::Brotli, Some(150_000), Some(159_200), false),
                delta(Metric::PeakMemory, None, Some(1 << 20), true),
                delta(Metric::Gzip, Some(1), None, true),
            ],
        ));
        assert_eq!(
            finding.message,
            "2026-11-20 \"new settings\": brotli size 159.2 kB (was 150.0 kB, measured differently, not comparable), peak memory 1.0 MiB (new), gzip size no longer measured"
        );
        assert_eq!(finding.tone, Tone::Info);
    }

    #[test]
    fn an_empty_history_points_to_accept() {
        let report = history_report(&[]);
        assert!(
            report.title.contains("no accepted baseline"),
            "{}",
            report.title
        );
        assert_eq!(report.next_steps.len(), 1);
    }

    #[test]
    fn json_lists_entries_with_their_changes() {
        let timelines = [Timeline {
            module: "engine".to_string(),
            accepted: 1,
            steps: vec![step(
                "pivot",
                true,
                vec![delta(Metric::Brotli, Some(1), Some(2), true)],
            )],
        }];
        let json = history_json(&timelines);
        let entry = &json["modules"][0]["entries"][0];
        assert_eq!(json["modules"][0]["name"], "engine");
        assert_eq!(entry["first"], false);
        assert_eq!(entry["changes"][0]["rule"], "size/brotli");
        assert_eq!(entry["changes"][0]["now"], 2);
        assert!(entry.get("inventory").is_none());
    }
}
