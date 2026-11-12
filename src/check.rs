//! Comparing what was measured with the budget and the baseline.

use std::path::PathBuf;

use runemark::Verdict;

use crate::baseline::Entry;
use crate::config::Module;
use crate::measure::Measured;
use crate::metric::Metric;

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Over the budget: the gate fails.
    Error,
    /// Under the budget, but past the baseline by more than the tolerance.
    Warning,
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
}

/// Judges every measured metric of `module` against its budget and baseline.
#[must_use]
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

/// The verdict over all modules: failed if any is over budget, a warning if
/// any grew or has no baseline yet.
#[must_use]
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

    #[test]
    fn a_value_at_its_budget_is_within_it() {
        assert_eq!(judge(100, Some(100), None, 0.01), None);
    }

    #[test]
    fn without_budget_or_baseline_nothing_is_judged() {
        assert_eq!(judge(100, None, None, 0.01), None);
    }
}
