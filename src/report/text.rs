use std::fmt::Write;

use runemark::{
    DetailLevel, Finding, FindingGroup, Location, Metric as ReportMetric, NextStep, Report, Tone,
};

use crate::check::{Change, Issue, Level, ModuleResult, Reason, verdict};
use crate::explain::{Evidence, Reached};
use crate::inventory::{
    CustomKind, ExternType, FeatureUse, FunctionBody, ImportName, Inventory, MemoryType, Names,
    SectionKind, TableType, TargetFeatureMode,
};
use crate::reach::{Edge, Hop, Node, ReachReport, Reachability};
use crate::runner::{ScenarioResult, Unobserved};
use crate::snapshot::{
    Confidence, Drift, FunctionChange, FunctionChangeKind, FunctionDrift, Part, SectionDrift,
    Status,
};
use crate::units::{format_binary, format_decimal};

/// The report `check` prints. With `details`, it also lists every measured
/// value.
#[must_use]
pub fn report(results: &[ModuleResult], details: bool) -> Report {
    let names = |pick: fn(&ModuleResult) -> bool| -> Vec<&str> {
        results
            .iter()
            .filter(|result| pick(result))
            .map(|result| result.module.name.as_str())
            .collect()
    };
    // A method path cannot stand in for this closure: it would pin the lifetime
    // of `ModuleResult`, which the `fn` pointer type leaves open.
    #[allow(clippy::redundant_closure_for_method_calls)]
    let over = names(|result| result.over_budget());
    let violated = names(|result| !result.over_budget() && result.violated());
    let failed = |result: &ModuleResult| result.over_budget() || result.violated();
    let grown = names(|result| !result.over_budget() && !result.violated() && result.grown());
    let drifted = names(|result| {
        !result.over_budget() && !result.violated() && !result.grown() && result.drifted()
    });
    let unset = names(|result| result.baseline.is_none());

    let title = if !over.is_empty() {
        format!("over budget: {}", over.join(", "))
    } else if !violated.is_empty() {
        format!("policy broken: {}", violated.join(", "))
    } else if !grown.is_empty() {
        format!("grown past the baseline: {}", grown.join(", "))
    } else if !drifted.is_empty() {
        format!("baseline no longer matches: {}", drifted.join(", "))
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
        .add_metric(count(
            "Policy broken",
            results.iter().filter(|result| result.violated()).count(),
            Tone::Error,
        ))
        .add_metric(count("Grown", grown.len(), Tone::Warning))
        .add_metric(count(
            "Baseline drift",
            results.iter().filter(|result| result.drifted()).count(),
            Tone::Warning,
        ));

    if details {
        // The inventory is long; without the detailed level it is cut to three lines.
        report = report.with_detail_level(DetailLevel::Detailed);
        for group in detail_groups(results) {
            report = report.add_group(group);
        }
    }

    for group in results.iter().filter_map(module_group) {
        report = report.add_group(group);
    }
    // After every finding of every module: an explanation never pushes one down.
    for group in results.iter().filter_map(explanation_group) {
        report = report.add_group(group);
    }

    if !over.is_empty() {
        report = report.add_next_step(NextStep::new(
            "Shrink the module, or raise its budget in the configuration on purpose",
        ));
    }
    if !violated.is_empty() {
        report = report.add_next_step(NextStep::new(
            "Fix the module, or change [module.policy] in the configuration on purpose; accepting the baseline does not waive it",
        ));
    }
    let to_accept: Vec<&str> = results
        .iter()
        .filter(|result| {
            !failed(result) && (result.grown() || result.drifted() || result.baseline.is_none())
        })
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

/// The findings of one module; `None` if there are none.
fn module_group(result: &ModuleResult) -> Option<FindingGroup> {
    let mut group = FindingGroup::new(&result.module.name);
    for violation in &result.violations {
        group = group.add_finding(
            Finding::new(Tone::Error, violation.message())
                .with_rule_id(violation.rule())
                .with_remedy(violation.action()),
        );
    }
    for issue in &result.issues {
        group = group.add_finding(finding(issue));
    }
    for change in &result.changes {
        group = group.add_finding(change_finding(change));
        if let Change::Functions(drift) = change {
            for finding in function_findings(drift) {
                group = group.add_finding(finding);
            }
        }
    }
    if result.baseline.is_none() {
        group = group.add_finding(
            Finding::new(Tone::Warning, "No baseline accepted yet")
                .with_rule_id("baseline/missing"),
        );
    }
    let any = !result.issues.is_empty()
        || !result.violations.is_empty()
        || !result.changes.is_empty()
        || result.baseline.is_none();
    any.then_some(group)
}

/// The explanations of one module, each a claim with its evidence and then
/// what it does not cover and what to do next; `None` if there are none.
fn explanation_group(result: &ModuleResult) -> Option<FindingGroup> {
    let explained = &result.explanations;
    if explained.listed.is_empty() {
        return None;
    }
    // One finding per explanation: the compact report shows three per group.
    let mut group =
        FindingGroup::new(format!("Explanations: {}", result.module.name)).with_advisory(true);
    for (rank, explanation) in explained.listed.iter().enumerate() {
        let evidence: Vec<_> = explanation.evidence.iter().map(evidence_text).collect();
        let evidence = if evidence.is_empty() {
            String::new()
        } else {
            format!("; {}", evidence.join("; "))
        };
        let mut message = format!(
            "{}. {} ({}{evidence}).",
            rank + 1,
            explanation.summary,
            explanation.status.label()
        );
        if let Some(reached) = &explanation.reached {
            let _ = write!(message, " {}.", reached_text(reached));
        }
        let _ = write!(
            message,
            " Next: {}. Coverage: {}.",
            explanation.action, explanation.coverage
        );
        if explained.omitted > 0 && rank + 1 == explained.listed.len() {
            let _ = write!(
                message,
                " {} further explanation{} left out; the findings above stay complete and `--json` lists them in full.",
                explained.omitted,
                if explained.omitted == 1 {
                    " was"
                } else {
                    "s were"
                }
            );
        }
        group = group.add_finding(Finding::new(Tone::Info, message).with_rule_id(explanation.rule));
    }
    Some(group)
}

/// `code bytes 3.0 kB -> 3.4 kB`; a side that is not known is left out.
fn evidence_text(evidence: &Evidence) -> String {
    let bytes = |bytes: u64| {
        if evidence.capacity {
            format_binary(bytes)
        } else {
            format_decimal(bytes)
        }
    };
    match (evidence.was, evidence.now) {
        (Some(was), Some(now)) => format!("{} {} -> {}", evidence.what, bytes(was), bytes(now)),
        (None, Some(now)) => format!("{} now {}", evidence.what, bytes(now)),
        (Some(was), None) => format!("{} was {}", evidence.what, bytes(was)),
        (None, None) => evidence.what.clone(),
    }
}

/// Why the function is in the module, from the reference graph.
fn reached_text(reached: &Reached) -> String {
    let how = match reached.reachability {
        Reachability::Direct => {
            let path = if reached.path.len() > 6 {
                format!(
                    "{} -> ... -> {}",
                    reached.path[..3].join(" -> "),
                    reached.path[reached.path.len() - 3..].join(" -> ")
                )
            } else {
                reached.path.join(" -> ")
            };
            format!(
                "reached from {}: {path}",
                reached.root.as_deref().unwrap_or("a root")
            )
        }
        Reachability::ViaTable => {
            "reached only through the table; indirect dispatch is assumed to reach every entry"
                .to_string()
        }
        Reachability::NoStaticRoot => {
            "no static root in the reference graph (not a statement that it is unused)".to_string()
        }
        Reachability::Unknown => {
            "unknown: the module uses operators the reference graph does not model".to_string()
        }
    };
    format!(
        "Why {} is in the module: {how}; {} direct caller{}",
        reached.function,
        reached.callers,
        if reached.callers == 1 { "" } else { "s" }
    )
}

/// The `--details` groups: the measured values, then the inventory per module.
fn detail_groups(results: &[ModuleResult]) -> Vec<FindingGroup> {
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
        for scenario in &result.measured.scenarios {
            measured = measured.add_finding(Finding::new(Tone::Info, trace(scenario)));
        }
    }
    let mut groups = vec![measured];
    for result in results {
        let mut group = FindingGroup::new(format!("Inventory: {}", result.module.name));
        let lines = inventory_lines(&result.measured.inventory)
            .into_iter()
            .chain(reach_lines(
                &result.measured.inventory,
                &result.measured.reach,
            ));
        for line in lines {
            group = group.add_finding(Finding::new(Tone::Info, line));
        }
        groups.push(group);
    }
    groups
}

/// What the static inventory says, one line each. Features are labelled as
/// derived, and what cannot be detected is stated every time.
fn inventory_lines(inventory: &Inventory) -> Vec<String> {
    let counts = &inventory.counts;
    let functions = counts.imported_functions + counts.defined_functions;
    let mut lines = vec![format!(
        "core module, binary version {}: {} types, {functions} functions ({} imported), {} globals, {} imports, {} exports, {} data segments, {} element segments",
        inventory.artifact.version,
        counts.types,
        counts.imported_functions,
        inventory.globals.len(),
        counts.imports,
        counts.exports,
        counts.data_segments,
        counts.element_segments,
    )];
    for import in &inventory.imports {
        lines.push(format!(
            "import {}.{}: {}",
            import.module,
            import.name,
            extern_type(&import.ty)
        ));
    }
    for export in &inventory.exports {
        lines.push(format!(
            "export {} (index {}): {}",
            export.name,
            export.index,
            extern_type(&export.ty)
        ));
    }
    for memory in &inventory.memories {
        lines.push(format!(
            "memory {}{}: {}",
            memory.index,
            origin(memory.imported_from.as_ref(), &memory.exported_as),
            memory_type(&memory.ty)
        ));
    }
    for table in &inventory.tables {
        lines.push(format!(
            "table {}{}: {}",
            table.index,
            origin(table.imported_from.as_ref(), &table.exported_as),
            table_type(&table.ty)
        ));
    }
    for tag in &inventory.tags {
        lines.push(format!(
            "tag {}{}: {}",
            tag.index,
            origin(tag.imported_from.as_ref(), &tag.exported_as),
            tag.ty
        ));
    }
    lines.push(feature_line(&inventory.features));
    let coverage = &inventory.coverage;
    let mut gaps = Vec::new();
    for (proposal, count) in &coverage.unclassified_instructions {
        gaps.push(format!(
            "{count} instructions of proposal {proposal} are not classified"
        ));
    }
    for section in &coverage.unreadable_custom_sections {
        gaps.push(format!("custom section {section} could not be read"));
    }
    if !gaps.is_empty() {
        lines.push(format!("coverage reduced: {}", gaps.join("; ")));
    }
    let limits: Vec<_> = coverage
        .not_detectable
        .iter()
        .map(ToString::to_string)
        .collect();
    lines.push(format!("not detectable: {}", limits.join("; ")));
    lines.push(sections_line(inventory));
    lines.extend(size_lines(inventory));
    lines.extend(metadata_lines(inventory));
    lines
}

/// Names, producers and target features: what the toolchain left in the file.
fn metadata_lines(inventory: &Inventory) -> Vec<String> {
    let counts = &inventory.counts;
    let functions = counts.imported_functions + counts.defined_functions;
    let mut lines = Vec::new();
    lines.push(match &inventory.names {
        Names::Absent => "names: no name section, functions and locals are unnamed".to_string(),
        Names::Unreadable => "names: the name section could not be read".to_string(),
        Names::Present {
            module,
            functions: named,
            functions_with_local_names,
        } => format!(
            "names: {named} of {functions} functions named, {functions_with_local_names} with local names{}",
            module
                .as_ref()
                .map_or_else(String::new, |name| format!(", module {name}")),
        ),
    });
    if !inventory.producers.is_empty() {
        let producers: Vec<_> = inventory
            .producers
            .iter()
            .map(|producer| format!("{} {} {}", producer.field, producer.name, producer.version))
            .collect();
        lines.push(format!("producers: {}", producers.join(", ")));
    }
    if !inventory.target_features.is_empty() {
        let features: Vec<_> = inventory
            .target_features
            .iter()
            .map(|feature| {
                let mark = match feature.mode {
                    TargetFeatureMode::Used => '+',
                    TargetFeatureMode::Disallowed => '-',
                    TargetFeatureMode::Required => '=',
                };
                format!("{mark}{}", feature.name)
            })
            .collect();
        lines.push(format!(
            "target_features (the toolchain's claim, not used for the features above): {}",
            features.join(" ")
        ));
    }
    lines
}

fn extern_type(ty: &ExternType) -> String {
    match ty {
        ExternType::Function(signature) => format!("func {signature}"),
        ExternType::Table(table) => format!("table {}", table_type(table)),
        ExternType::Memory(memory) => format!("memory {}", memory_type(memory)),
        ExternType::Global(global) => format!(
            "global {}{}",
            if global.mutable { "mut " } else { "" },
            global.value
        ),
        ExternType::Tag(signature) => format!("tag {signature}"),
    }
}

fn origin(imported_from: Option<&ImportName>, exported_as: &[String]) -> String {
    let mut parts = Vec::new();
    if let Some(import) = imported_from {
        parts.push(format!("imported from {}.{}", import.module, import.name));
    }
    if !exported_as.is_empty() {
        parts.push(format!("exported as {}", exported_as.join(", ")));
    }
    if parts.is_empty() {
        " (internal)".to_string()
    } else {
        format!(" ({})", parts.join("; "))
    }
}

fn memory_type(memory: &MemoryType) -> String {
    let pages = |count: u64| format!("{count} page{}", if count == 1 { "" } else { "s" });
    let mut parts = vec![format!(
        "min {} ({})",
        pages(memory.minimum),
        format_wide(memory.minimum_bytes())
    )];
    parts.push(match (memory.maximum, memory.maximum_bytes()) {
        (Some(maximum), Some(bytes)) => {
            format!("max {} ({})", pages(maximum), format_wide(bytes))
        }
        _ => "no maximum".to_string(),
    });
    if memory.shared {
        parts.push("shared".to_string());
    }
    if memory.memory64 {
        parts.push("64-bit".to_string());
    }
    parts.push(format!("page size {}", format_binary(memory.page_size)));
    parts.join(", ")
}

fn table_type(table: &TableType) -> String {
    let mut parts = vec![format!("{} min {}", table.element, table.minimum)];
    parts.push(table.maximum.map_or_else(
        || "no maximum".to_string(),
        |maximum| format!("max {maximum}"),
    ));
    if table.table64 {
        parts.push("64-bit".to_string());
    }
    if table.shared {
        parts.push("shared".to_string());
    }
    parts.join(", ")
}

fn format_wide(bytes: u128) -> String {
    u64::try_from(bytes).map_or_else(|_| "more than 16 EiB".to_string(), format_binary)
}

fn feature_line(features: &[FeatureUse]) -> String {
    if features.is_empty() {
        return "features (derived from declarations and instructions): none found".to_string();
    }
    let plural =
        |count: u64, noun: &str| format!("{count} {noun}{}", if count == 1 { "" } else { "s" });
    let listed: Vec<_> = features
        .iter()
        .map(|used| {
            let mut evidence = Vec::new();
            if used.declarations > 0 {
                evidence.push(plural(used.declarations, "declaration"));
            }
            if used.instructions > 0 {
                evidence.push(plural(used.instructions, "instruction"));
            }
            format!("{} ({})", used.feature.label(), evidence.join(", "))
        })
        .collect();
    format!(
        "features (derived from declarations and instructions): {}",
        listed.join(", ")
    )
}

/// How the file divides into parts, and the largest function bodies and data
/// segments. Function sizes are shallow raw bytes; a function without a name or
/// export is named by its index in this file, which means nothing in another build.
fn size_lines(inventory: &Inventory) -> Vec<String> {
    let composition = &inventory.composition;
    let functions = &inventory.functions;
    let mut parts = Vec::new();
    for region in crate::snapshot::Region::ALL {
        let bytes = region.bytes(composition);
        if region == crate::snapshot::Region::Code {
            parts.push(format!(
                "code {} ({} in {} bodies, {} for their sizes and count)",
                format_decimal(bytes),
                format_decimal(functions.body_bytes),
                functions.count,
                format_decimal(bytes - functions.body_bytes)
            ));
        } else if bytes > 0 {
            parts.push(format!("{} {}", region.label(), format_decimal(bytes)));
        }
    }
    let mut lines = vec![format!(
        "composition: {} = file {}",
        parts.join(" + "),
        format_decimal(composition.total())
    )];
    if !functions.largest.is_empty() {
        let listed: Vec<_> = functions
            .largest
            .iter()
            .take(10)
            .map(|body| format!("{} {}", body.label(), format_decimal(body.size)))
            .collect();
        lines.push(format!(
            "largest function bodies (raw bytes, not compressed savings): {}",
            listed.join(", ")
        ));
    }
    if !inventory.data.largest.is_empty() {
        let listed: Vec<_> = inventory
            .data
            .largest
            .iter()
            .map(|segment| {
                let place = match (segment.memory, segment.offset) {
                    (None, _) => "passive".to_string(),
                    (Some(memory), Some(offset)) => format!("memory {memory} at {offset:#x}"),
                    (Some(memory), None) => format!("memory {memory} at a computed offset"),
                };
                format!("{} ({place})", format_decimal(segment.size))
            })
            .collect();
        lines.push(format!(
            "largest data segments (contents not shown): {}",
            listed.join(", ")
        ));
    }
    lines
}

/// How many of the largest functions get a line about why they are in the module.
const EXPLAINED: usize = 5;

/// The longest path printed in full; a longer one keeps its start and end.
const SHOWN_HOPS: usize = 8;

/// Why the largest functions are in the module, from the reference graph:
/// a representative path from a root and the direct callers. A function
/// without a path is "no static root", never unused.
fn reach_lines(inventory: &Inventory, reach: &ReachReport) -> Vec<String> {
    let label = |function: u32| {
        inventory
            .functions
            .body(function)
            .map_or_else(|| format!("func[{function}]"), FunctionBody::label)
    };
    let node = |node: Node| match node {
        Node::Function(function) => label(function),
        Node::Table => "table".to_string(),
    };
    let mut lines = Vec::new();
    for explanation in reach.explanations.iter().take(EXPLAINED) {
        let name = label(explanation.function);
        let size = inventory
            .functions
            .body(explanation.function)
            .map_or_else(String::new, |body| {
                format!(" ({})", format_decimal(body.size))
            });
        let callers = &explanation.callers;
        let called = if callers.count == 0 {
            "no direct callers".to_string()
        } else {
            let listed: Vec<_> = callers.listed.iter().map(|c| node(c.node)).collect();
            let more = callers.count - listed.len();
            format!(
                "{} direct caller{}: {}{}",
                callers.count,
                if callers.count == 1 { "" } else { "s" },
                listed.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            )
        };
        let reached = match explanation.reachability {
            Reachability::Direct => format!("reached from {}", path_text(&explanation.path, &node)),
            Reachability::ViaTable
                if matches!(
                    explanation.path.first().map(|hop| &hop.edge),
                    Some(Edge::Host)
                ) =>
            {
                format!(
                    "reached only through the table, which the module imports or exports, so the host may call every entry: {}",
                    path_text(&explanation.path, &node)
                )
            }
            Reachability::ViaTable => format!(
                "reached only by indirect dispatch, which assumes every table entry is callable: {}",
                path_text(&explanation.path, &node)
            ),
            Reachability::NoStaticRoot => {
                "no static root in the reference graph (not a statement that it is unused)"
                    .to_string()
            }
            Reachability::Unknown => {
                "unknown: the module uses operators the reference graph does not model".to_string()
            }
        };
        lines.push(format!(
            "why {name}{size} is in the module: {reached}; {called}"
        ));
    }
    lines
}

/// `export "run": a -> b -ref.func-> c`, with the middle left out if long.
fn path_text(path: &[Hop], node: &dyn Fn(Node) -> String) -> String {
    let Some(first) = path.first() else {
        return String::new();
    };
    let root = match &first.edge {
        Edge::Export(name) => format!("export \"{name}\""),
        Edge::Start => "the start function".to_string(),
        _ => "table".to_string(),
    };
    let step = |hop: &Hop| {
        let arrow = match hop.edge {
            Edge::Reference => " -ref.func-> ",
            Edge::Dispatch => " -indirect-> ",
            Edge::Elements => " -element-> ",
            _ => " -> ",
        };
        format!("{arrow}{}", node(hop.node))
    };
    let start = node(first.node);
    let mut text = if start == root {
        start
    } else {
        format!("{root}: {start}")
    };
    let rest = &path[1..];
    if rest.len() < SHOWN_HOPS {
        text.extend(rest.iter().map(step));
    } else {
        text.extend(rest[..3].iter().map(step));
        let _ = write!(text, " -> ... {} more ...", rest.len() - 6);
        text.extend(rest[rest.len() - 3..].iter().map(step));
    }
    text
}

/// Section bytes by kind, custom sections by name, and the preamble and
/// section headers that no payload accounts for.
fn sections_line(inventory: &Inventory) -> String {
    let mut by_kind: Vec<(SectionKind, u64)> = Vec::new();
    for section in &inventory.sections {
        if section.kind == SectionKind::Custom {
            continue;
        }
        match by_kind.iter_mut().find(|(kind, _)| *kind == section.kind) {
            Some((_, bytes)) => *bytes += section.payload,
            None => by_kind.push((section.kind, section.payload)),
        }
    }
    let framing = crate::inventory::PREAMBLE_BYTES
        + inventory
            .sections
            .iter()
            .map(|section| section.header)
            .sum::<u64>();
    let mut parts: Vec<String> = by_kind
        .iter()
        .map(|(kind, bytes)| format!("{} {}", section_name(*kind), format_binary(*bytes)))
        .collect();
    for custom in &inventory.custom_sections {
        let kind = match custom.kind {
            CustomKind::Names => "names",
            CustomKind::Debug => "debug",
            CustomKind::Producers => "producers",
            CustomKind::TargetFeatures => "target features",
            CustomKind::Other => "other",
        };
        parts.push(format!(
            "custom {} ({kind}) {}",
            custom.name,
            format_binary(custom.bytes)
        ));
    }
    parts.push(format!(
        "preamble and section headers {}",
        format_binary(framing)
    ));
    format!("sections: {}", parts.join(", "))
}

fn section_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Custom => "custom",
        SectionKind::Type => "type",
        SectionKind::Import => "import",
        SectionKind::Function => "function",
        SectionKind::Table => "table",
        SectionKind::Memory => "memory",
        SectionKind::Global => "global",
        SectionKind::Export => "export",
        SectionKind::Start => "start",
        SectionKind::Element => "element",
        SectionKind::Code => "code",
        SectionKind::Data => "data",
        SectionKind::DataCount => "data count",
        SectionKind::Tag => "tag",
    }
}

/// One line per scenario: the capacity trace split into warm-up and later growth.
/// It states what the numbers are, not whether anything leaks.
fn trace(scenario: &ScenarioResult) -> String {
    let later = match scenario.growth_per_run {
        Some(growth) => format!(", then {} per run", format_binary(growth)),
        None => String::new(),
    };
    format!(
        "{}: capacity {} after instantiate, +{} in the first run{later} ({} runs, page steps of 64 KiB)",
        scenario.id(),
        format_binary(scenario.after_instantiate),
        format_binary(scenario.warmup_growth),
        scenario.repeat,
    )
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
        Level::Warning | Level::Note => (
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

fn change_finding(change: &Change) -> Finding {
    let message = match change {
        Change::ScenarioRemoved { scenario } => {
            format!("Scenario {scenario} is in the baseline and is no longer measured")
        }
        Change::ScenarioNew { scenario } => {
            format!("Scenario {scenario} has no accepted values yet")
        }
        Change::MetricMissing { metric } => format!(
            "{} was accepted and is not measured now",
            capitalize(metric.label())
        ),
        Change::Incomparable { scenario, reason } => {
            let why = match reason {
                Reason::ScenarioEdited => "its script changed since it was accepted".to_string(),
                Reason::RepeatChanged { was, now } => {
                    format!("it runs {now} times, accepted with {was}")
                }
                Reason::Compression => {
                    "gzip and brotli ran with other settings than the baseline".to_string()
                }
                Reason::Runner => "memory is measured differently than at the baseline".to_string(),
            };
            match (scenario, reason) {
                (Some(scenario), _) => format!("Scenario {scenario} is not compared: {why}"),
                (None, Reason::Compression) => format!("Sizes are not compared: {why}"),
                (None, _) => format!("Memory is not compared: {why}"),
            }
        }
        Change::Unobserved { scenario, what } => {
            let what = match what {
                Unobserved::InternalMemory { count } => format!(
                    "{count} declared memor{} neither exported nor imported, so not measured",
                    if *count == 1 { "y is" } else { "ies are" }
                ),
                Unobserved::Workers { count } => format!(
                    "{count} worker{} started, and a module run there is not measured",
                    if *count == 1 { "" } else { "s" }
                ),
            };
            format!("Scenario {scenario}: {what}")
        }
        Change::RuntimeChanged { was, now } => {
            format!("Node {now}, accepted with {was}; values stay comparable")
        }
        Change::Interface(drift) => drift_message(drift),
        Change::InventoryNotAccepted => {
            "The baseline has no inventory yet, so imports, exports and features are not compared"
                .to_string()
        }
        Change::SizesNotAccepted => {
            "The baseline has no section or function sizes yet, so a change of size is not attributed"
                .to_string()
        }
        Change::Sections { drift, .. } => sections_message(drift),
        Change::Functions(drift) => functions_message(drift),
    };
    let tone = match change.level() {
        Level::Note => Tone::Info,
        _ => Tone::Warning,
    };
    let finding = Finding::new(tone, message).with_rule_id(change.rule());
    match change {
        Change::Sections {
            action: Some(action),
            ..
        } => finding.with_remedy(action),
        _ => finding,
    }
}

fn signed(delta: i64) -> String {
    let sign = if delta < 0 { '-' } else { '+' };
    format!("{sign}{}", format_decimal(delta.unsigned_abs()))
}

fn sections_message(drift: &SectionDrift) -> String {
    let parts: Vec<_> = drift
        .changed
        .iter()
        .map(|delta| format!("{} {}", delta.part.label(), signed(delta.delta())))
        .collect();
    format!(
        "File {} ({}, was {}): {}; the parts add up to the file size",
        signed(drift.now_total as i64 - drift.was_total as i64),
        format_decimal(drift.now_total),
        format_decimal(drift.was_total),
        parts.join(", ")
    )
}

fn functions_message(drift: &FunctionDrift) -> String {
    let count = |kind: FunctionChangeKind| drift.changes.iter().filter(|c| c.kind == kind).count();
    let confidence = match drift.confidence {
        Confidence::High => "high",
        Confidence::Partial => "partial",
        Confidence::None => "none",
    };
    let status = match drift.status {
        Status::Exact => "exact",
        Status::Estimated => "estimated",
    };
    let mut message = format!(
        "Function bodies {} (raw bytes, not a compressed saving); matching by name or export: confidence {confidence}, {} matched, {} changed, {} added, {} removed, {} unmatched before, {} unmatched now; not attributed to a listed function {}; {status}",
        signed(drift.body_delta),
        drift.matched,
        count(FunctionChangeKind::Changed),
        count(FunctionChangeKind::Added),
        count(FunctionChangeKind::Removed),
        drift.unmatched_accepted.len(),
        drift.unmatched_now.len(),
        signed(drift.unaccounted),
    );
    let entered = count(FunctionChangeKind::EnteredLargest);
    let left = count(FunctionChangeKind::LeftLargest);
    if entered + left > 0 {
        let _ = write!(
            message,
            "; {entered} entered and {left} left the largest functions, which are the only ones listed"
        );
    }
    message
}

/// The functions with the largest change, one finding each, and the ones that
/// could not be matched, named by the local ID of the build they are in.
fn function_findings(drift: &FunctionDrift) -> Vec<Finding> {
    const SHOWN: usize = 5;
    let mut findings: Vec<_> = drift
        .changes
        .iter()
        .take(SHOWN)
        .map(|change| {
            Finding::new(Tone::Info, function_change_message(change))
                .with_rule_id("baseline/size-functions")
        })
        .collect();
    let unmatched = |label: &str, refs: &[crate::snapshot::FunctionRef]| {
        (!refs.is_empty()).then(|| {
            let listed: Vec<_> = refs
                .iter()
                .take(SHOWN)
                .map(|r| {
                    format!(
                        "{} {}",
                        r.id.map_or_else(|| "?".to_string(), |id| format!("func[{id}]")),
                        format_decimal(r.size)
                    )
                })
                .collect();
            Finding::new(
                Tone::Info,
                format!(
                    "Unmatched {label}, no usable name or export, so not compared (local IDs, valid in that build only): {}",
                    listed.join(", ")
                ),
            )
            .with_rule_id("baseline/size-functions")
        })
    };
    findings.extend(unmatched("before", &drift.unmatched_accepted));
    findings.extend(unmatched("now", &drift.unmatched_now));
    findings
}

fn function_change_message(change: &FunctionChange) -> String {
    let size = |bytes: Option<u64>| bytes.map_or_else(String::new, format_decimal);
    match change.kind {
        FunctionChangeKind::Changed => format!(
            "Function {}: {} -> {} ({})",
            change.key,
            size(change.was),
            size(change.now),
            signed(change.delta().unwrap_or(0))
        ),
        FunctionChangeKind::Added => {
            format!("Function {} was added: {}", change.key, size(change.now))
        }
        FunctionChangeKind::Removed => {
            format!(
                "Function {} was removed: was {}",
                change.key,
                size(change.was)
            )
        }
        FunctionChangeKind::EnteredLargest => format!(
            "Function {} is among the largest now ({}) and was not among the accepted largest; it may be new or have been smaller",
            change.key,
            size(change.now)
        ),
        FunctionChangeKind::LeftLargest => format!(
            "Function {} (was {}) is no longer among the largest; it may be gone or smaller",
            change.key,
            size(change.was)
        ),
    }
}

fn drift_message(drift: &Drift) -> String {
    match drift {
        Drift::Added { part, item } => {
            let more = match part {
                Part::Import => "; the host must provide one more binding",
                _ => "",
            };
            format!(
                "{} {} {} was added{more}",
                part.label(),
                item.label(),
                item.kind_and_type()
            )
        }
        Drift::Removed { part, item } => format!(
            "{} {} {} was removed",
            part.label(),
            item.label(),
            item.kind_and_type()
        ),
        Drift::Changed { part, item, was } => format!(
            "{} {} changed: {} {was} was, {} now",
            part.label(),
            item.label(),
            item.kind,
            item.ty
        ),
        Drift::FeatureAdded(feature) => {
            format!("The binary now needs {feature}, which the accepted inventory did not")
        }
        Drift::FeatureRemoved(feature) => {
            format!("The binary no longer needs {feature}")
        }
    }
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
    use std::path::PathBuf;

    use super::*;
    use crate::metric::Metric;

    #[test]
    fn capitalises_the_first_letter_only() {
        assert_eq!(capitalize("peak memory"), "Peak memory");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn an_error_names_the_budget_and_a_warning_the_growth() {
        let error = Issue {
            metric: Metric::PeakMemory,
            level: Level::Error,
            value: 256 * 1024,
            limit: 128 * 1024,
            scenario: Some(PathBuf::from("steady.mjs")),
        };
        let warning = Issue {
            metric: Metric::Raw,
            level: Level::Warning,
            value: 2_000,
            limit: 1_000,
            scenario: None,
        };
        let error = format!("{:?}", finding(&error));
        let warning = format!("{:?}", finding(&warning));
        assert!(
            error.contains("Peak memory 256.0 KiB, budget 128.0 KiB"),
            "{error}"
        );
        assert!(
            warning.contains("Raw size 2.0 kB, +1.0 kB over the baseline"),
            "{warning}"
        );
    }
}
