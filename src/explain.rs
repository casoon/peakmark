//! Explaining a finding from evidence the check already holds.
//!
//! An explanation says which part of the artifact or of a scenario moved, with
//! the value before and after, how sure that statement is, what the evidence
//! does not cover and what to look at next. It is assembled from data the
//! measurement produced anyway: the inventory, the section and function
//! drift, the interface drift, the reference graph of the largest functions
//! and the memory trace. It runs no analysis of its own.
//!
//! Explanations are separate from the findings of [`crate::check`]. They never
//! decide the verdict, change a budget, accept a baseline or run anything, and
//! when there is no evidence there is no explanation: nothing is guessed from
//! names. At most [`MAX_EXPLANATIONS`] are listed, in a fixed order; the
//! findings they stand on stay complete in `changes` and `issues`.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use serde::Serialize;

use crate::baseline::ScenarioEntry;
use crate::check::{Change, ModuleResult, Reason, runner_differs};
use crate::inventory::{CustomKind, FunctionBody, Inventory};
use crate::metric::Metric;
use crate::reach::{Edge, Node, Reachability};
use crate::runner::{ScenarioResult, Unobserved};
use crate::snapshot::{
    Confidence, Drift, FunctionChange, FunctionChangeKind, FunctionDrift, FunctionKey, Part,
    PartDelta, Region,
};
use crate::units::{format_binary, format_decimal};

/// How many explanations a check lists per module.
pub const MAX_EXPLANATIONS: usize = 3;

/// How many entries of a long list (imports, sections) one explanation names.
const LISTED: usize = 3;

/// How sure the statement is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Read from the file or the trace and complete: the numbers are what the
    /// artifact or the measurement contains.
    Exact,
    /// Read from the data, but the match or the coverage is partial.
    Estimated,
    /// The data allows the statement, it does not prove it.
    Hypothesis,
}

impl Status {
    /// The word a report uses.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Estimated => "estimated",
            Self::Hypothesis => "hypothesis",
        }
    }
}

/// One value before and after, in bytes. Where only one side exists the other
/// is `None`; a declaration (`what` names it) has neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// What the bytes are.
    pub what: String,
    /// When the baseline was accepted; `None` if not known.
    pub was: Option<u64>,
    /// Now; `None` if it no longer exists.
    pub now: Option<u64>,
    /// Whether the bytes are memory capacity, which a report shows in binary
    /// units like the rest of the memory values.
    #[serde(skip)]
    pub(crate) capacity: bool,
}

impl Evidence {
    fn new(what: impl Into<String>, was: Option<u64>, now: Option<u64>) -> Self {
        Self {
            what: what.into(),
            was,
            now,
            capacity: false,
        }
    }

    fn capacity(what: impl Into<String>, was: Option<u64>, now: Option<u64>) -> Self {
        Self {
            capacity: true,
            ..Self::new(what, was, now)
        }
    }
}

/// Why a function is in the module, from the reference graph: the answer to
/// "who keeps it", never to "is it needed".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reached {
    /// The function.
    pub function: String,
    /// What the graph says.
    pub reachability: Reachability,
    /// The root the path starts at, such as `export "run"`.
    pub root: Option<String>,
    /// The functions of a representative path, root first.
    pub path: Vec<String>,
    /// How many functions call or reference it directly.
    pub callers: usize,
}

/// One ranked, evidence-backed statement about a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explanation {
    /// The stable rule id, `explain/...`.
    pub rule: &'static str,
    /// How sure the statement is.
    pub status: Status,
    /// What it is about: a scenario, a function, a part of the file.
    pub subject: String,
    /// The statement.
    pub summary: String,
    /// The values behind it.
    pub evidence: Vec<Evidence>,
    /// Why the function is in the module, if the statement is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reached: Option<Reached>,
    /// What the evidence does not cover.
    pub coverage: String,
    /// What to look at or do next. It names evidence and never a saving.
    pub action: String,
    #[serde(skip)]
    tier: Tier,
    #[serde(skip)]
    weight: u64,
}

/// What comes first: a measurement that cannot be used, then a change of the
/// interface, then a change of size or memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    Input,
    Interface,
    Cost,
}

impl Explanation {
    fn new(
        tier: Tier,
        rule: &'static str,
        status: Status,
        subject: impl Into<String>,
        summary: impl Into<String>,
        coverage: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            status,
            subject: subject.into(),
            summary: summary.into(),
            evidence: Vec::new(),
            reached: None,
            coverage: coverage.into(),
            action: action.into(),
            tier,
            weight: 0,
        }
    }

    fn with_evidence(mut self, evidence: Vec<Evidence>) -> Self {
        self.evidence = evidence;
        self
    }

    fn with_weight(mut self, weight: u64) -> Self {
        self.weight = weight;
        self
    }
}

/// The explanations of one module: the best [`MAX_EXPLANATIONS`], and how many
/// more there were.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Explained {
    /// The listed explanations, best first.
    pub listed: Vec<Explanation>,
    /// How many further explanations were left out by the cap.
    pub omitted: usize,
}

/// Explains the findings of `result`. Cheap: it reads what the measurement and
/// the comparison already produced.
#[must_use]
pub fn explain(result: &ModuleResult) -> Explained {
    let mut found = Vec::new();
    not_comparable(result, &mut found);
    if result.baseline.is_none() {
        found.extend(composition(result));
    } else {
        sizes(result, &mut found);
    }
    interface(result, &mut found);
    memory(result, &mut found);
    rank(found)
}

/// Orders by tier, status, weight, rule and subject; drops a repeated
/// (rule, subject); keeps the cap.
fn rank(mut found: Vec<Explanation>) -> Explained {
    found.sort_by(|a, b| {
        (a.tier, a.status, Reverse(a.weight), a.rule, &a.subject).cmp(&(
            b.tier,
            b.status,
            Reverse(b.weight),
            b.rule,
            &b.subject,
        ))
    });
    let mut seen = BTreeSet::new();
    found.retain(|e| seen.insert((e.rule, e.subject.clone())));
    let omitted = found.len().saturating_sub(MAX_EXPLANATIONS);
    found.truncate(MAX_EXPLANATIONS);
    Explained {
        listed: found,
        omitted,
    }
}

/// A scenario or metric that has no counterpart in the baseline: say so before
/// anything else, because every number next to it would be a guess.
fn not_comparable(result: &ModuleResult, out: &mut Vec<Explanation>) {
    for change in &result.changes {
        let (subject, summary, coverage, action) = match change {
            Change::Incomparable { scenario, reason } => {
                let why = match reason {
                    Reason::ScenarioEdited => {
                        "its script differs from the accepted one".to_string()
                    }
                    Reason::RepeatChanged { was, now } => {
                        format!("it runs {now} times, accepted with {was}")
                    }
                    Reason::Compression => {
                        "gzip and Brotli use other settings than at the baseline".to_string()
                    }
                    Reason::Runner => {
                        "memory is measured by another protocol than at the baseline".to_string()
                    }
                };
                let subject = scenario.clone().unwrap_or_else(|| "module".to_string());
                (
                    subject.clone(),
                    format!("{subject} is measured but not compared with the baseline: {why}"),
                    "The values are measured; only the comparison is missing, so growth there is not reported",
                    "Accept the baseline again when the new setup is intended",
                )
            }
            Change::MetricMissing { metric } => (
                metric.rule().to_string(),
                format!("{} was accepted and is not measured now", metric.label()),
                "Nothing is measured for it, so growth there cannot be reported",
                "Configure the scenario again in peakmark.toml, or accept the baseline to drop it on purpose",
            ),
            Change::ScenarioRemoved { scenario } => (
                scenario.clone(),
                format!("Scenario {scenario} is in the baseline and is no longer measured"),
                "Nothing is measured for it, so growth there cannot be reported",
                "Configure the scenario again in peakmark.toml, or accept the baseline to drop it on purpose",
            ),
            _ => continue,
        };
        out.push(Explanation::new(
            Tier::Input,
            "explain/not-comparable",
            Status::Exact,
            subject,
            summary,
            coverage,
            action,
        ));
    }
}

/// The first run: what the file is made of, with no growth claimed.
fn composition(result: &ModuleResult) -> Option<Explanation> {
    let inventory = &result.measured.inventory;
    let mut parts: Vec<_> = Region::ALL
        .iter()
        .map(|&part| (part, part.bytes(&inventory.composition)))
        .filter(|&(_, bytes)| bytes > 0)
        .collect();
    parts.sort_by_key(|&(part, bytes)| (Reverse(bytes), part as u8));
    let (largest, largest_bytes) = *parts.first()?;
    let total = inventory.composition.total();
    let mut evidence: Vec<_> = parts
        .iter()
        .take(LISTED)
        .map(|&(part, bytes)| Evidence::new(format!("{} bytes", part.label()), None, Some(bytes)))
        .collect();
    let mut reached = None;
    if largest == Region::Code
        && let Some(body) = inventory.functions.largest.first()
    {
        evidence.push(Evidence::new(
            format!("largest function {}", body.label()),
            None,
            Some(body.size),
        ));
        reached = reached_of(result, body);
    }
    let mut explanation = Explanation::new(
        Tier::Cost,
        "explain/composition",
        Status::Exact,
        "module",
        format!(
            "No baseline yet, so no growth is claimed. The file is {}, mostly {} ({})",
            format_decimal(total),
            largest.label(),
            format_decimal(largest_bytes)
        ),
        "The parts add up to the file size; what the data holds and what the functions do is not read",
        format!(
            "Start with the {} part; `peakmark check --details` lists its contents. Accept the baseline once these sizes are intended",
            largest.label()
        ),
    )
    .with_evidence(evidence)
    .with_weight(total);
    explanation.reached = reached;
    Some(explanation)
}

/// Explanations for a size issue, from the section and function drift.
fn sizes(result: &ModuleResult, out: &mut Vec<Explanation>) {
    let Some(issue) = result
        .issues
        .iter()
        .find(|issue| matches!(issue.metric, Metric::Raw | Metric::Gzip | Metric::Brotli))
    else {
        return;
    };
    let not_accepted = result
        .changes
        .iter()
        .any(|c| matches!(c, Change::InventoryNotAccepted | Change::SizesNotAccepted));
    if not_accepted {
        out.push(
            Explanation::new(
                Tier::Input,
                "explain/insufficient",
                Status::Exact,
                issue.metric.rule(),
                format!(
                    "{} grew, and the baseline has no section or function sizes to attribute it to",
                    capitalize(issue.metric.label())
                ),
                "Nothing of this change is attributed; the baseline predates the stored inventory",
                "Accept the baseline again on purpose to record the inventory; the next change is then attributed, this one is not",
            )
            .with_evidence(vec![Evidence::new(
                issue.metric.label(),
                result
                    .baseline
                    .and_then(|entry| entry.values.get(issue.metric)),
                Some(issue.value),
            )]),
        );
        return;
    }
    let Some(sections) = result.changes.iter().find_map(|c| match c {
        Change::Sections { drift, .. } => Some(drift),
        _ => None,
    }) else {
        return;
    };
    let functions = result.changes.iter().find_map(|c| match c {
        Change::Functions(drift) => Some(drift),
        _ => None,
    });
    let inventory = &result.measured.inventory;
    for delta in sections.changed.iter().filter(|delta| delta.delta() > 0) {
        let explanation = match delta.part {
            Region::Code => Some(code(result, delta, functions)),
            Region::Data => Some(data(delta, inventory)),
            Region::Names | Region::Debug | Region::Custom => Some(custom(delta, inventory)),
            // Declarations are explained by the interface drift, and the
            // headers follow the other parts.
            Region::Structure | Region::Overhead => None,
        };
        out.extend(explanation);
    }
}

fn part_evidence(delta: &PartDelta) -> Evidence {
    Evidence::new(
        format!("{} bytes", delta.part.label()),
        Some(delta.was),
        Some(delta.now),
    )
}

fn data(delta: &PartDelta, inventory: &Inventory) -> Explanation {
    let mut evidence = vec![part_evidence(delta)];
    let place = inventory.data.largest.first().map(|segment| {
        evidence.push(Evidence::new(
            "largest data segment",
            None,
            Some(segment.size),
        ));
        match (segment.memory, segment.offset) {
            (None, _) => "no fixed place (passive)".to_string(),
            (Some(memory), Some(offset)) => format!("memory {memory}, offset {offset:#x}"),
            (Some(memory), None) => format!("memory {memory}, a computed offset"),
        }
    });
    let action = match place {
        Some(place) => format!(
            "Examine the embedded data: the largest segment sits at {place}. peakmark does not show what it holds, so find it in the source or the build"
        ),
        None => "Examine the embedded data in the source or the build".to_string(),
    };
    Explanation::new(
        Tier::Cost,
        "explain/data-growth",
        Status::Exact,
        "data",
        format!(
            "Embedded data grew by {}",
            format_decimal(delta.delta().unsigned_abs())
        ),
        "The data section is accounted exactly; the segments of the accepted build are not stored, so the new bytes are not tied to one segment",
        action,
    )
    .with_evidence(evidence)
    .with_weight(delta.delta().unsigned_abs())
}

/// Names, debug information or other custom sections that grew.
fn custom(delta: &PartDelta, inventory: &Inventory) -> Explanation {
    let kind = match delta.part {
        Region::Names => Some(CustomKind::Names),
        Region::Debug => Some(CustomKind::Debug),
        _ => None,
    };
    let mut sections: Vec<_> = inventory
        .custom_sections
        .iter()
        .filter(|section| match kind {
            Some(kind) => section.kind == kind,
            None => section.kind == CustomKind::Other,
        })
        .collect();
    sections.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    let mut evidence = vec![part_evidence(delta)];
    evidence.extend(sections.iter().take(LISTED).map(|section| {
        Evidence::new(
            format!("custom section {}", section.name),
            None,
            Some(section.bytes),
        )
    }));
    let (rule, subject, what, action) = match delta.part {
        Region::Names => (
            "explain/names-growth",
            "names",
            "The name section",
            "If this is the artifact you deploy, decide whether it needs readable names. Keep them in a diagnostic build: they are also what lets peakmark match functions across builds. peakmark does not modify files",
        ),
        Region::Debug => (
            "explain/debug-growth",
            "debug",
            "Debug information",
            "If this is the artifact you deploy, build the deployment artifact without debug information and keep the debug build separately. peakmark does not modify files",
        ),
        _ => (
            "explain/custom-growth",
            "custom",
            "Other custom sections",
            "Find which tool writes these sections and whether the deployment artifact needs them; peakmark does not read their content",
        ),
    };
    Explanation::new(
        Tier::Cost,
        rule,
        Status::Exact,
        subject,
        format!(
            "{what} grew by {}",
            format_decimal(delta.delta().unsigned_abs())
        ),
        "The sections are accounted exactly by name; their content is not interpreted",
        action,
    )
    .with_evidence(evidence)
    .with_weight(delta.delta().unsigned_abs())
}

/// The function that grew most, or the code part if no function can be named.
fn code(
    result: &ModuleResult,
    delta: &PartDelta,
    functions: Option<&FunctionDrift>,
) -> Explanation {
    let inventory = &result.measured.inventory;
    let Some(drift) = functions else {
        return code_unattributed(delta, None);
    };
    let Some((change, growth)) = grown_function(drift) else {
        return code_unattributed(delta, Some(drift));
    };
    let (status, how) = match (change.kind, drift.confidence) {
        (FunctionChangeKind::Changed | FunctionChangeKind::Added, Confidence::High) => {
            (Status::Exact, "matched by its name or export")
        }
        (FunctionChangeKind::Changed | FunctionChangeKind::Added, _) => (
            Status::Estimated,
            "matched by its name or export, while part of the functions could not be matched",
        ),
        _ => (
            Status::Hypothesis,
            "among the largest now and not among the accepted largest, so it may be new or may have been smaller",
        ),
    };
    let summary = match change.kind {
        FunctionChangeKind::Added => format!(
            "Function {} is new and takes {}",
            change.key,
            format_decimal(growth)
        ),
        FunctionChangeKind::Changed => {
            format!("Function {} grew by {}", change.key, format_decimal(growth))
        }
        _ => format!(
            "Function {} is among the largest now ({}) and was not before",
            change.key,
            format_decimal(growth)
        ),
    };
    let mut explanation = Explanation::new(
        Tier::Cost,
        "explain/function-growth",
        status,
        change.key.to_string(),
        summary,
        format!(
            "Raw body bytes, {how}; {} of {} changed body bytes are attributed to listed functions{}",
            drift.attributed,
            drift.body_delta,
            if drift.unaccounted == 0 {
                String::new()
            } else {
                format!(", {} are not", drift.unaccounted)
            }
        ),
        format!(
            "Review the function {}: compare its source and the build flags that affect it; the reach says who keeps it in the module",
            change.key
        ),
    )
    .with_evidence(vec![
        Evidence::new(format!("function {}", change.key), change.was, change.now),
        part_evidence(delta),
    ]);
    explanation.reached =
        key_body(inventory, &change.key).and_then(|body| reached_of(result, body));
    explanation.weight = growth;
    explanation
}

/// The code grew and no function can be named for it.
fn code_unattributed(delta: &PartDelta, drift: Option<&FunctionDrift>) -> Explanation {
    let why = match drift.map(|d| d.confidence) {
        Some(Confidence::None) => {
            "no function has a name or export in the builds, and peakmark never matches by index"
        }
        _ => "no function that grew could be matched between the builds",
    };
    Explanation::new(
        Tier::Cost,
        "explain/code-growth",
        Status::Exact,
        "code",
        format!(
            "The code grew by {}, and {why}",
            format_decimal(delta.delta().unsigned_abs())
        ),
        "The code section is accounted exactly; which functions changed is unknown",
        "Keep function names in the builds you compare (a diagnostic build with the name section), accept it, and the next change is attributed to a function; `check --details` lists the largest bodies now",
    )
    .with_evidence(vec![part_evidence(delta)])
    .with_weight(delta.delta().unsigned_abs())
}

/// The listed function that gained most and how much: a changed one by its
/// delta, an added one by its size; failing both, one that entered the largest.
fn grown_function(drift: &FunctionDrift) -> Option<(&FunctionChange, u64)> {
    let growth = |change: &FunctionChange| -> Option<u64> {
        match change.kind {
            FunctionChangeKind::Changed => change.delta().filter(|&d| d > 0).map(i64::unsigned_abs),
            FunctionChangeKind::Added => change.now,
            _ => None,
        }
    };
    let best = |pick: &dyn Fn(&FunctionChange) -> Option<u64>| {
        drift
            .changes
            .iter()
            .filter_map(|c| pick(c).map(|g| (c, g)))
            .min_by(|(a, ga), (b, gb)| gb.cmp(ga).then_with(|| a.key.cmp(&b.key)))
    };
    best(&growth).or_else(|| {
        best(&|c| {
            (c.kind == FunctionChangeKind::EnteredLargest)
                .then_some(c.now)
                .flatten()
        })
    })
}

/// The body that carries `key`, if exactly one of the largest does.
fn key_body<'a>(inventory: &'a Inventory, key: &FunctionKey) -> Option<&'a FunctionBody> {
    let mut found = inventory.functions.largest.iter().filter(|body| match key {
        FunctionKey::Name(name) => body.name.as_deref() == Some(name),
        FunctionKey::Export(export) => {
            body.name.is_none() && body.exported_as.iter().any(|e| e == export)
        }
    });
    let body = found.next()?;
    found.next().is_none().then_some(body)
}

/// What the reference graph says about `body`, if it was explained.
fn reached_of(result: &ModuleResult, body: &FunctionBody) -> Option<Reached> {
    let inventory = &result.measured.inventory;
    let explanation = result
        .measured
        .reach
        .explanations
        .iter()
        .find(|e| e.function == body.index)?;
    let label = |node: Node| match node {
        Node::Function(index) => inventory
            .functions
            .body(index)
            .map_or_else(|| format!("func[{index}]"), FunctionBody::label),
        Node::Table => "table".to_string(),
    };
    let root = explanation.path.first().map(|hop| match &hop.edge {
        Edge::Export(name) => format!("export \"{name}\""),
        Edge::Start => "the start function".to_string(),
        _ => "table".to_string(),
    });
    Some(Reached {
        function: body.label(),
        reachability: explanation.reachability,
        root,
        path: explanation.path.iter().map(|hop| label(hop.node)).collect(),
        callers: explanation.callers.count,
    })
}

/// A new import or a new required feature: the host must provide or support more.
fn interface(result: &ModuleResult, out: &mut Vec<Explanation>) {
    let mut imports = Vec::new();
    let mut features = Vec::new();
    for change in &result.changes {
        match change {
            Change::Interface(Drift::Added {
                part: Part::Import,
                item,
            }) => imports.push(item),
            Change::Interface(Drift::FeatureAdded(feature)) => features.push(feature),
            _ => {}
        }
    }
    if let Some(first) = imports.first() {
        let mut evidence: Vec<_> = imports
            .iter()
            .take(LISTED)
            .map(|item| {
                Evidence::new(
                    format!("import {} {}", item.label(), item.kind_and_type()),
                    None,
                    None,
                )
            })
            .collect();
        if let Some(delta) = declarations_delta(result) {
            evidence.push(part_evidence(&delta));
        }
        let more = imports.len().saturating_sub(LISTED);
        out.push(
            Explanation::new(
                Tier::Interface,
                "explain/import-added",
                Status::Exact,
                first.label(),
                format!(
                    "The module imports {} more binding{}{}",
                    imports.len(),
                    if imports.len() == 1 { "" } else { "s" },
                    if more > 0 {
                        format!(" ({more} not listed here)")
                    } else {
                        String::new()
                    }
                ),
                "Compared with the accepted inventory by module, name and kind; why the import was added and what it costs is not known",
                format!(
                    "Make sure the host provides {} with the declared type before this build is deployed; peakmark does not instantiate or run the module",
                    first.label()
                ),
            )
            .with_evidence(evidence),
        );
    }
    if let Some(first) = features.first() {
        out.push(
            Explanation::new(
                Tier::Interface,
                "explain/feature-added",
                Status::Exact,
                (*first).clone(),
                format!(
                    "The binary now needs {} feature{}: {}",
                    features.len(),
                    if features.len() == 1 { "" } else { "s" },
                    features.iter().take(LISTED).map(|f| f.as_str()).collect::<Vec<_>>().join(", ")
                ),
                "Features are derived from the declarations and instructions in the file, not from a compiler flag; some cannot be detected",
                "Check that every runtime you target supports it; if one does not, the build needs other target features. peakmark does not rebuild",
            )
            .with_evidence(Vec::new()),
        );
    }
}

fn declarations_delta(result: &ModuleResult) -> Option<PartDelta> {
    result.changes.iter().find_map(|change| match change {
        Change::Sections { drift, .. } => drift
            .changed
            .iter()
            .find(|delta| delta.part == Region::Structure && delta.delta() > 0)
            .copied(),
        _ => None,
    })
}

/// A scenario whose memory grew, located in the trace: at instantiation, in
/// the first run, in every run, or later.
fn memory(result: &ModuleResult, out: &mut Vec<Explanation>) {
    let Some(entry) = result.baseline else {
        return;
    };
    if runner_differs(entry) {
        return;
    }
    for scenario in &result.measured.scenarios {
        let flagged = result.issues.iter().any(|issue| {
            matches!(issue.metric, Metric::PeakMemory | Metric::GrowthPerRun)
                && issue.scenario.as_deref() == Some(scenario.file.as_path())
        });
        let Some(accepted) = entry.scenarios.get(&scenario.id()) else {
            continue;
        };
        if !flagged || accepted.sha256 != scenario.sha256 || accepted.repeat != scenario.repeat {
            continue;
        }
        out.extend(memory_growth(result, scenario, accepted));
    }
}

/// Where in the trace the growth first shows, and what to look at there.
struct Phase {
    /// Whether the baseline kept a trace to locate it with.
    traced: bool,
    summary: String,
    action: String,
}

fn phase(
    result: &ModuleResult,
    scenario: &ScenarioResult,
    accepted: &ScenarioEntry,
    evidence: &mut Vec<Evidence>,
) -> Option<Phase> {
    let id = scenario.id();
    let peak_growth = scenario.peak_memory.saturating_sub(accepted.peak_memory);
    let (Some(was_start), Some(&was_first)) =
        (accepted.after_instantiate, accepted.after_run.first())
    else {
        return (peak_growth > 0).then(|| Phase {
            traced: false,
            summary: format!(
                "Peak capacity in {id} grew from {} to {}, and the baseline predates the trace",
                format_binary(accepted.peak_memory),
                format_binary(scenario.peak_memory)
            ),
            action: "Accept the baseline again on purpose to keep the trace; the next change is then located at instantiation, in the first run or later".to_string(),
        });
    };
    let now_first = *scenario.after_run.first()?;
    evidence.push(Evidence::capacity(
        "capacity after instantiation",
        Some(was_start),
        Some(scenario.after_instantiate),
    ));
    evidence.push(Evidence::capacity(
        "capacity after the first run",
        Some(was_first),
        Some(now_first),
    ));
    let per_run = accepted.growth_per_run.zip(scenario.growth_per_run);
    if let Some((was, now)) = per_run {
        evidence.push(Evidence::capacity("growth per run", Some(was), Some(now)));
    }
    let first_growth = now_first.saturating_sub(scenario.after_instantiate);
    let was_first_growth = was_first.saturating_sub(was_start);
    let (summary, action) = if scenario.after_instantiate > was_start {
        if let Some(initial) = result.measured.initial_memory {
            evidence.push(Evidence::capacity(
                "declared initial memory now",
                None,
                Some(initial),
            ));
        }
        (
            format!(
                "Capacity after instantiation grew from {} to {} in {id}",
                format_binary(was_start),
                format_binary(scenario.after_instantiate)
            ),
            "Look at what the module reserves when it starts: its declared initial memory and its start function. peakmark reads capacity, not allocations",
        )
    } else if first_growth > was_first_growth {
        (
            format!(
                "The first run of {id} reserved {} where it reserved {}",
                format_binary(first_growth),
                format_binary(was_first_growth)
            ),
            "Look at what the first call of the scenario allocates; caches and pools fill in the first run. Capacity counts reserved pages, not live data",
        )
    } else if let Some((was, now)) = per_run.filter(|&(was, now)| now > was) {
        (
            format!(
                "Capacity in {id} grows by {} with every run after the first (was {})",
                format_binary(now),
                format_binary(was)
            ),
            "Look at what each run keeps alive. Reserved capacity growing per run is not by itself a leak; a larger `repeat` shows whether it continues",
        )
    } else if peak_growth > 0 {
        (
            format!(
                "Peak capacity in {id} grew from {} to {} in the later runs",
                format_binary(accepted.peak_memory),
                format_binary(scenario.peak_memory)
            ),
            "Compare the trace in `--json` with the baseline to see which run added it; capacity counts reserved pages, not live data",
        )
    } else {
        return None;
    };
    Some(Phase {
        traced: true,
        summary,
        action: action.to_string(),
    })
}

fn memory_growth(
    result: &ModuleResult,
    scenario: &ScenarioResult,
    accepted: &ScenarioEntry,
) -> Option<Explanation> {
    let id = scenario.id();
    let peak_growth = scenario.peak_memory.saturating_sub(accepted.peak_memory);
    let mut evidence = vec![Evidence::capacity(
        "peak capacity",
        Some(accepted.peak_memory),
        Some(scenario.peak_memory),
    )];
    let Phase {
        traced,
        summary,
        action,
    } = phase(result, scenario, accepted, &mut evidence)?;
    // What did not move is not evidence of the growth.
    evidence.retain(|e| e.was != e.now || e.what == "peak capacity");
    let unobserved = &scenario.coverage.unobserved;
    let status = if traced && unobserved.is_empty() {
        Status::Exact
    } else {
        Status::Estimated
    };
    let mut coverage = format!(
        "Capacity of the exported or imported memories of {} instance{}",
        scenario.coverage.instances,
        if scenario.coverage.instances == 1 {
            ""
        } else {
            "s"
        }
    );
    for what in unobserved {
        coverage.push_str(match what {
            Unobserved::InternalMemory { .. } => {
                "; declared memory that is neither exported nor imported is not observed"
            }
            Unobserved::Workers { .. } => "; a module run in a worker is not observed",
        });
    }
    if !traced {
        coverage.push_str("; the phase of the growth is not known");
    }
    Some(
        Explanation::new(
            Tier::Cost,
            "explain/memory-growth",
            status,
            id,
            summary,
            coverage,
            action,
        )
        .with_evidence(evidence)
        .with_weight(peak_growth),
    )
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
    use super::*;
    use crate::snapshot::{FunctionKey, Status as DriftStatus};

    fn found(rule: &'static str, status: Status, tier: Tier, weight: u64) -> Explanation {
        Explanation::new(tier, rule, status, rule, "s", "c", "a").with_weight(weight)
    }

    #[test]
    fn the_order_is_input_then_interface_then_cost_and_exact_before_a_guess() {
        let explained = rank(vec![
            found("explain/data-growth", Status::Hypothesis, Tier::Cost, 9_000),
            found("explain/code-growth", Status::Exact, Tier::Cost, 10),
            found("explain/import-added", Status::Exact, Tier::Interface, 0),
            found("explain/not-comparable", Status::Exact, Tier::Input, 0),
        ]);
        let rules: Vec<_> = explained.listed.iter().map(|e| e.rule).collect();
        assert_eq!(
            rules,
            [
                "explain/not-comparable",
                "explain/import-added",
                "explain/code-growth"
            ]
        );
        assert_eq!(explained.omitted, 1);
    }

    #[test]
    fn a_repeated_rule_and_subject_is_listed_once_and_ties_break_by_name() {
        let mut twice = found("explain/data-growth", Status::Exact, Tier::Cost, 5);
        twice.subject = "a".to_string();
        let explained = rank(vec![
            found("explain/data-growth", Status::Exact, Tier::Cost, 5),
            twice.clone(),
            twice,
        ]);
        assert_eq!(explained.listed.len(), 2);
        assert_eq!(explained.listed[0].subject, "a");
    }

    fn change(
        kind: FunctionChangeKind,
        name: &str,
        was: Option<u64>,
        now: Option<u64>,
    ) -> FunctionChange {
        FunctionChange {
            kind,
            key: FunctionKey::Name(name.to_string()),
            was,
            now,
        }
    }

    fn drift(changes: Vec<FunctionChange>) -> FunctionDrift {
        FunctionDrift {
            status: DriftStatus::Estimated,
            confidence: Confidence::High,
            was_body_bytes: 0,
            now_body_bytes: 0,
            body_delta: 0,
            attributed: 0,
            unaccounted: 0,
            matched: 0,
            changes,
            unmatched_accepted: Vec::new(),
            unmatched_now: Vec::new(),
            accepted_complete: true,
            now_complete: true,
        }
    }

    #[test]
    fn the_function_that_gained_most_wins_and_a_smaller_one_never_does() {
        let drift = drift(vec![
            change(FunctionChangeKind::Changed, "small", Some(10), Some(20)),
            change(FunctionChangeKind::Changed, "shrunk", Some(90), Some(10)),
            change(FunctionChangeKind::Added, "new", None, Some(50)),
            change(FunctionChangeKind::EnteredLargest, "maybe", None, Some(900)),
        ]);
        let (best, growth) = grown_function(&drift).unwrap();
        assert_eq!((best.key.to_string(), growth), ("new".to_string(), 50));
    }

    #[test]
    fn a_function_that_only_entered_the_largest_is_a_hypothesis_candidate() {
        let drift = drift(vec![
            change(FunctionChangeKind::Changed, "shrunk", Some(90), Some(10)),
            change(FunctionChangeKind::EnteredLargest, "maybe", None, Some(900)),
        ]);
        let (best, growth) = grown_function(&drift).unwrap();
        assert_eq!(best.kind, FunctionChangeKind::EnteredLargest);
        assert_eq!(growth, 900);
    }

    #[test]
    fn nothing_grew_no_function_is_named() {
        let drift = drift(vec![
            change(FunctionChangeKind::Changed, "shrunk", Some(90), Some(10)),
            change(FunctionChangeKind::Removed, "gone", Some(5), None),
        ]);
        assert!(grown_function(&drift).is_none());
    }
}
