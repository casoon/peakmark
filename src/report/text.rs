use runemark::{
    DetailLevel, Finding, FindingGroup, Location, Metric as ReportMetric, NextStep, Report, Tone,
};

use crate::check::{Change, Issue, Level, ModuleResult, Reason, verdict};
use crate::inventory::{
    CustomKind, ExternType, FeatureUse, ImportName, Inventory, MemoryType, Names, SectionKind,
    TableType, TargetFeatureMode,
};
use crate::runner::{ScenarioResult, Unobserved};
use crate::snapshot::{Drift, Part};
use crate::units::format_binary;

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
    for issue in &result.issues {
        group = group.add_finding(finding(issue));
    }
    for violation in &result.violations {
        group = group.add_finding(
            Finding::new(Tone::Error, violation.message())
                .with_rule_id(violation.rule())
                .with_remedy(violation.action()),
        );
    }
    for change in &result.changes {
        group = group.add_finding(change_finding(change));
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
        for line in inventory_lines(&result.measured.inventory) {
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
    };
    let tone = match change.level() {
        Level::Note => Tone::Info,
        _ => Tone::Warning,
    };
    Finding::new(tone, message).with_rule_id(change.rule())
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
