//! The accepted inventory: the part of an [`Inventory`] the baseline keeps,
//! and the differences between two of them.
//!
//! A [`Snapshot`] holds what a deployment depends on: the imports and exports
//! with their types, the memories and tables the module declares for itself,
//! and the features the binary needs. It is compact (one short entry per
//! declaration) and deterministic (every list is sorted), so reordering a
//! module changes nothing. Later parts add fields next to these; a field the
//! file lacks reads as absent, never as a changed value. The size parts are
//! [`Snapshot::sections`] (the bytes of the file by purpose) and
//! [`Snapshot::functions`] (the count, the sum and only the largest function
//! bodies, so the file stays small), compared by [`section_drift`] and
//! [`function_drift`].
//!
//! # Identity
//!
//! A declaration is identified by its import module (imports only), its name
//! and its kind, never by its name alone: a module may import two things
//! called `env.f`, a function and a memory. The type is what can change under
//! an identity: a different signature, other limits. A module that imports the
//! same module, name and kind twice with different types is compared as a set
//! of types; the sorted leftovers are paired up as changes, and the rest are
//! additions and removals.
//!
//! # What is listed where
//!
//! A memory or table that is imported or exported appears as that import or
//! export, with its limits as the type. [`Snapshot::memories`] and
//! [`Snapshot::tables`] list only those the module keeps to itself, named by
//! their position among them (`#0`), so a limit change is reported once.

use std::collections::BTreeMap;
use std::fmt::{self, Write};

use serde::{Deserialize, Serialize};

use crate::inventory::{
    Composition, ExternType, Feature, FunctionBody, Inventory, MemoryType, TableType,
};

/// The default size of a memory page; any other size is written out.
const DEFAULT_PAGE_SIZE: u64 = 65_536;

/// What kind of entity a declaration is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A function.
    Func,
    /// A table.
    Table,
    /// A memory.
    Memory,
    /// A global.
    Global,
    /// A tag.
    Tag,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Func => "func",
            Self::Table => "table",
            Self::Memory => "memory",
            Self::Global => "global",
            Self::Tag => "tag",
        })
    }
}

/// One declaration: an import, an export, or a memory or table the module
/// keeps to itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Item {
    /// The import module; absent for everything but imports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// The import field, the export name, or `#n` for an internal memory or table.
    pub name: String,
    /// What it is.
    pub kind: Kind,
    /// Its type without the kind: a signature `(i32) -> i32`, limits `1..16 shared`,
    /// a table `funcref 1..`, a global `mut i32`.
    #[serde(rename = "type")]
    pub ty: String,
}

impl Item {
    /// The name as a sentence shows it: `env.log`, `add`, `#0`.
    #[must_use]
    pub fn label(&self) -> String {
        match &self.module {
            Some(module) => format!("{module}.{}", self.name),
            None => self.name.clone(),
        }
    }

    /// The kind and the type as a sentence shows them: `func (i32) -> i32`.
    #[must_use]
    pub fn kind_and_type(&self) -> String {
        format!("{} {}", self.kind, self.ty)
    }

    fn identity(&self) -> Identity {
        (self.module.clone(), self.name.clone(), self.kind)
    }
}

type Identity = (Option<String>, String, Kind);

/// The accepted inventory of a module, as stored in the baseline.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Imports, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<Item>,
    /// Exports, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<Item>,
    /// Memories that are neither imported nor exported, named `#0`, `#1`, ….
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<Item>,
    /// Tables that are neither imported nor exported, named `#0`, `#1`, ….
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<Item>,
    /// The features the binary needs, by their snake-case name, in the order of [`Feature`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// The bytes of the file by purpose. Absent in entries accepted before they were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<Composition>,
    /// The function bodies. Absent in entries accepted before they were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionList>,
}

/// One of the largest function bodies as the snapshot keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionEntry {
    /// The index in the file it was read from. Kept only where the function has
    /// no identity of its own (no name, no export, or a name that occurs twice):
    /// it is valid in that one file and never used to match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// Bytes of the body, shallow.
    pub size: u64,
    /// The name from the `name` section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The first export name, if exported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<String>,
}

/// What a name or export makes of a function: the only thing two builds are
/// matched by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "by", content = "value", rename_all = "snake_case")]
pub enum FunctionKey {
    /// The name in the `name` section.
    Name(String),
    /// An export name, for a function without a name.
    Export(String),
}

impl fmt::Display for FunctionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::Export(name) => write!(f, "export \"{name}\""),
        }
    }
}

impl FunctionEntry {
    /// The identity of the function across builds, if it has one.
    #[must_use]
    pub fn key(&self) -> Option<FunctionKey> {
        match (&self.name, &self.export) {
            (Some(name), _) => Some(FunctionKey::Name(name.clone())),
            (None, Some(export)) => Some(FunctionKey::Export(export.clone())),
            (None, None) => None,
        }
    }

    /// What a sentence calls it: the key, or the local ID.
    #[must_use]
    pub fn label(&self) -> String {
        match (self.key(), self.id) {
            (Some(key), _) => key.to_string(),
            (None, Some(id)) => format!("func[{id}]"),
            (None, None) => "a function".to_string(),
        }
    }
}

/// The function bodies of a module as the baseline keeps them: bounded, with
/// only the [`crate::inventory::LARGEST_FUNCTIONS`] largest listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionList {
    /// Number of defined functions.
    pub count: u32,
    /// The sum of all body sizes.
    pub body_bytes: u64,
    /// The largest bodies, biggest first.
    #[serde(default)]
    pub largest: Vec<FunctionEntry>,
}

impl FunctionList {
    /// Whether every function is listed, so that one missing from the list is
    /// not there.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.largest.len() as u64 == u64::from(self.count)
    }

    fn of(inventory: &Inventory) -> Self {
        let functions = &inventory.functions;
        let key_of = |body: &FunctionBody| {
            body.name.clone().map_or_else(
                || body.exported_as.first().cloned().map(FunctionKey::Export),
                |name| Some(FunctionKey::Name(name)),
            )
        };
        let mut occurrences: BTreeMap<FunctionKey, u32> = BTreeMap::new();
        for body in &functions.bodies {
            if let Some(key) = key_of(body) {
                *occurrences.entry(key).or_default() += 1;
            }
        }
        let mut largest: Vec<FunctionEntry> = functions
            .largest
            .iter()
            .map(|body| {
                let unique = key_of(body).is_some_and(|key| occurrences[&key] == 1);
                FunctionEntry {
                    id: (!unique).then_some(body.index),
                    size: body.size,
                    name: body.name.clone(),
                    export: body.exported_as.first().cloned(),
                }
            })
            .collect();
        largest.sort_by(|a, b| {
            (std::cmp::Reverse(a.size), &a.name, &a.export, a.id).cmp(&(
                std::cmp::Reverse(b.size),
                &b.name,
                &b.export,
                b.id,
            ))
        });
        FunctionList {
            count: functions.count,
            body_bytes: functions.body_bytes,
            largest,
        }
    }
}

/// The kind and type text of an extern type, as stored.
#[must_use]
pub fn type_text(ty: &ExternType) -> (Kind, String) {
    match ty {
        ExternType::Function(signature) => (Kind::Func, signature.to_string()),
        ExternType::Table(table) => (Kind::Table, table_text(table)),
        ExternType::Memory(memory) => (Kind::Memory, memory_text(memory)),
        ExternType::Global(global) => (
            Kind::Global,
            format!(
                "{}{}{}",
                if global.mutable { "mut " } else { "" },
                global.value,
                if global.shared { " shared" } else { "" }
            ),
        ),
        ExternType::Tag(signature) => (Kind::Tag, signature.to_string()),
    }
}

/// The identifier a feature has in the snapshot, the policy and `--json`.
#[must_use]
pub fn feature_id(feature: Feature) -> String {
    serde_json::to_value(feature)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .expect("a feature serializes as a string")
}

fn limits(minimum: u64, maximum: Option<u64>) -> String {
    match maximum {
        Some(maximum) => format!("{minimum}..{maximum}"),
        None => format!("{minimum}.."),
    }
}

fn memory_text(memory: &MemoryType) -> String {
    let mut text = limits(memory.minimum, memory.maximum);
    if memory.shared {
        text.push_str(" shared");
    }
    if memory.memory64 {
        text.push_str(" 64-bit");
    }
    if memory.page_size != DEFAULT_PAGE_SIZE {
        let _ = write!(text, " page-size {}", memory.page_size);
    }
    text
}

fn table_text(table: &TableType) -> String {
    let mut text = format!("{} {}", table.element, limits(table.minimum, table.maximum));
    if table.table64 {
        text.push_str(" 64-bit");
    }
    if table.shared {
        text.push_str(" shared");
    }
    text
}

impl Snapshot {
    /// What the baseline keeps of `inventory`.
    #[must_use]
    pub fn from_inventory(inventory: &Inventory) -> Self {
        let item = |module: Option<&str>, name: &str, ty: &ExternType| {
            let (kind, ty) = type_text(ty);
            Item {
                module: module.map(str::to_string),
                name: name.to_string(),
                kind,
                ty,
            }
        };
        let mut imports: Vec<_> = inventory
            .imports
            .iter()
            .map(|import| item(Some(&import.module), &import.name, &import.ty))
            .collect();
        let mut exports: Vec<_> = inventory
            .exports
            .iter()
            .map(|export| item(None, &export.name, &export.ty))
            .collect();
        // Entries are (reaches the host, type text); only the others are listed here.
        let internal = |kind, entries: Vec<(bool, String)>| {
            entries
                .into_iter()
                .filter(|(reaches_host, _)| !reaches_host)
                .enumerate()
                .map(|(position, (_, ty))| Item {
                    module: None,
                    name: format!("#{position}"),
                    kind,
                    ty,
                })
                .collect::<Vec<_>>()
        };
        let mut memories = internal(
            Kind::Memory,
            inventory
                .memories
                .iter()
                .map(|m| {
                    (
                        m.imported_from.is_some() || !m.exported_as.is_empty(),
                        memory_text(&m.ty),
                    )
                })
                .collect(),
        );
        let mut tables = internal(
            Kind::Table,
            inventory
                .tables
                .iter()
                .map(|t| {
                    (
                        t.imported_from.is_some() || !t.exported_as.is_empty(),
                        table_text(&t.ty),
                    )
                })
                .collect(),
        );
        for list in [&mut imports, &mut exports, &mut memories, &mut tables] {
            list.sort();
        }
        Snapshot {
            imports,
            exports,
            memories,
            tables,
            features: inventory
                .features
                .iter()
                .map(|used| feature_id(used.feature))
                .collect(),
            sections: Some(inventory.composition),
            functions: Some(FunctionList::of(inventory)),
        }
    }
}

/// Which list of the snapshot a difference is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// An import.
    Import,
    /// An export.
    Export,
    /// A memory the module keeps to itself.
    Memory,
    /// A table the module keeps to itself.
    Table,
}

impl Part {
    /// The word a sentence uses, capitalised.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Import => "Import",
            Self::Export => "Export",
            Self::Memory => "Internal memory",
            Self::Table => "Internal table",
        }
    }

    /// The word `--json` uses.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Export => "export",
            Self::Memory => "memory",
            Self::Table => "table",
        }
    }
}

/// One difference between the accepted and the measured inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// A declaration that was not accepted.
    Added {
        /// Where.
        part: Part,
        /// The declaration.
        item: Item,
    },
    /// An accepted declaration that is gone.
    Removed {
        /// Where.
        part: Part,
        /// The declaration.
        item: Item,
    },
    /// The same declaration with another signature or other limits.
    Changed {
        /// Where.
        part: Part,
        /// The declaration as it is now.
        item: Item,
        /// The accepted type.
        was: String,
    },
    /// A feature the binary needs now and did not when accepted.
    FeatureAdded(String),
    /// A feature the binary no longer needs.
    FeatureRemoved(String),
}

impl Drift {
    /// The rule id a finding about this difference carries.
    #[must_use]
    pub fn rule(&self) -> &'static str {
        match self {
            Self::Added { .. } => "baseline/interface-added",
            Self::Removed { .. } => "baseline/interface-removed",
            Self::Changed { .. } => "baseline/interface-changed",
            Self::FeatureAdded(_) => "baseline/feature-added",
            Self::FeatureRemoved(_) => "baseline/feature-removed",
        }
    }
}

/// Everything that differs between `accepted` and `now`, in a fixed order:
/// imports, exports, memories, tables, then features; within a list by
/// identity, changes before removals before additions. Lists that only differ
/// in order are equal.
#[must_use]
pub fn diff(accepted: &Snapshot, now: &Snapshot) -> Vec<Drift> {
    let mut drifts = Vec::new();
    let parts = [
        (Part::Import, &accepted.imports, &now.imports),
        (Part::Export, &accepted.exports, &now.exports),
        (Part::Memory, &accepted.memories, &now.memories),
        (Part::Table, &accepted.tables, &now.tables),
    ];
    for (part, was, is) in parts {
        diff_items(part, was, is, &mut drifts);
    }
    let only_in = |from: &[String], other: &[String]| -> Vec<String> {
        from.iter()
            .filter(|id| !other.contains(id))
            .cloned()
            .collect()
    };
    drifts.extend(
        only_in(&now.features, &accepted.features)
            .into_iter()
            .map(Drift::FeatureAdded),
    );
    drifts.extend(
        only_in(&accepted.features, &now.features)
            .into_iter()
            .map(Drift::FeatureRemoved),
    );
    drifts
}

fn group(items: &[Item]) -> BTreeMap<Identity, Vec<&str>> {
    let mut groups: BTreeMap<Identity, Vec<&str>> = BTreeMap::new();
    for item in items {
        groups.entry(item.identity()).or_default().push(&item.ty);
    }
    for types in groups.values_mut() {
        types.sort_unstable();
    }
    groups
}

fn diff_items(part: Part, accepted: &[Item], now: &[Item], drifts: &mut Vec<Drift>) {
    let was = group(accepted);
    let is = group(now);
    let mut identities: Vec<_> = was.keys().chain(is.keys()).collect();
    identities.sort();
    identities.dedup();
    for identity in identities {
        let mut gone: Vec<&str> = was.get(identity).cloned().unwrap_or_default();
        let mut new: Vec<&str> = is.get(identity).cloned().unwrap_or_default();
        // Types on both sides cancel out; what is left was removed, added or changed.
        gone.retain(|ty| match new.iter().position(|other| other == ty) {
            Some(position) => {
                new.remove(position);
                false
            }
            None => true,
        });
        let item = |ty: &str| Item {
            module: identity.0.clone(),
            name: identity.1.clone(),
            kind: identity.2,
            ty: ty.to_string(),
        };
        let paired = gone.len().min(new.len());
        for (old, current) in gone.iter().zip(&new) {
            drifts.push(Drift::Changed {
                part,
                item: item(current),
                was: (*old).to_string(),
            });
        }
        for ty in &gone[paired..] {
            drifts.push(Drift::Removed {
                part,
                item: item(ty),
            });
        }
        for ty in &new[paired..] {
            drifts.push(Drift::Added {
                part,
                item: item(ty),
            });
        }
    }
}

/// A part of the file, as the section accounting divides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    /// The code section.
    Code,
    /// The data section.
    Data,
    /// The `name` section.
    Names,
    /// Debug custom sections.
    Debug,
    /// Other custom sections.
    Custom,
    /// Types, imports, exports and the other declaring sections.
    Structure,
    /// The preamble and the section headers.
    Overhead,
}

impl Region {
    /// All parts, in the order a report lists them.
    pub const ALL: [Region; 7] = [
        Region::Code,
        Region::Data,
        Region::Names,
        Region::Debug,
        Region::Custom,
        Region::Structure,
        Region::Overhead,
    ];

    /// The word a sentence uses.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Data => "data",
            Self::Names => "names",
            Self::Debug => "debug",
            Self::Custom => "other custom",
            Self::Structure => "declarations",
            Self::Overhead => "headers and preamble",
        }
    }

    /// The bytes of this part.
    #[must_use]
    pub fn bytes(self, composition: &Composition) -> u64 {
        match self {
            Self::Code => composition.code,
            Self::Data => composition.data,
            Self::Names => composition.names,
            Self::Debug => composition.debug,
            Self::Custom => composition.custom,
            Self::Structure => composition.structure,
            Self::Overhead => composition.overhead,
        }
    }
}

/// How one part of the file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PartDelta {
    /// Which part.
    pub part: Region,
    /// Bytes when accepted.
    pub was: u64,
    /// Bytes now.
    pub now: u64,
}

impl PartDelta {
    /// Bytes gained (positive) or lost.
    #[must_use]
    pub fn delta(&self) -> i64 {
        self.now as i64 - self.was as i64
    }
}

/// How the parts of the file changed between two builds. The parts add up to
/// the file, so the deltas add up to the change of the file size, exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionDrift {
    /// File bytes when accepted.
    pub was_total: u64,
    /// File bytes now.
    pub now_total: u64,
    /// The parts that differ, in the order of [`Region::ALL`].
    pub changed: Vec<PartDelta>,
}

/// The parts of the file that differ between `accepted` and `now`; `None` if none does.
#[must_use]
pub fn section_drift(accepted: &Composition, now: &Composition) -> Option<SectionDrift> {
    let changed: Vec<_> = Region::ALL
        .iter()
        .map(|&part| PartDelta {
            part,
            was: part.bytes(accepted),
            now: part.bytes(now),
        })
        .filter(|delta| delta.was != delta.now)
        .collect();
    (!changed.is_empty()).then(|| SectionDrift {
        was_total: accepted.total(),
        now_total: now.total(),
        changed,
    })
}

/// A function body in one of the two builds, by its local ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FunctionRef {
    /// The index in that build. Valid there only.
    pub id: Option<u32>,
    /// Bytes of the body.
    pub size: u64,
}

/// What happened to a function that is named by a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionChangeKind {
    /// In both builds, with another size.
    Changed,
    /// Not in the accepted build, which lists every function.
    Added,
    /// Not in the build now, which lists every function.
    Removed,
    /// Among the largest now and not among the accepted largest; it may be new
    /// or have been smaller. The accepted size is at most the smallest listed.
    EnteredLargest,
    /// Among the accepted largest and no longer among the largest now; it may be
    /// gone or smaller.
    LeftLargest,
}

/// One function that changed, found through its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionChange {
    /// What happened.
    pub kind: FunctionChangeKind,
    /// The name or export it was matched by.
    pub key: FunctionKey,
    /// The accepted body; `None` if it was not listed.
    pub was: Option<u64>,
    /// The body now; `None` if it is not listed.
    pub now: Option<u64>,
}

impl FunctionChange {
    /// The change in bytes if both sizes are known.
    #[must_use]
    pub fn delta(&self) -> Option<i64> {
        Some(self.now? as i64 - self.was? as i64)
    }

    fn magnitude(&self) -> u64 {
        match (self.was, self.now) {
            (Some(was), Some(now)) => was.abs_diff(now),
            (Some(size), None) | (None, Some(size)) => size,
            (None, None) => 0,
        }
    }
}

/// How far two function lists could be matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Every listed function on both sides has a name or export that occurs once.
    High,
    /// Some could be matched by key, some have no usable key.
    Partial,
    /// None has a usable key (a stripped module): nothing is matched, and an
    /// index is never used instead.
    None,
}

/// Whether the attribution accounts for the whole change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Matching is high and the listed functions explain the body change to the byte.
    Exact,
    /// Part of the change is not attributed: functions are unmatched or not listed.
    Estimated,
}

/// How the function bodies changed, as far as the match is trustworthy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionDrift {
    /// Whether the change is fully attributed.
    pub status: Status,
    /// How far the lists could be matched.
    pub confidence: Confidence,
    /// Sum of all bodies when accepted.
    pub was_body_bytes: u64,
    /// Sum of all bodies now.
    pub now_body_bytes: u64,
    /// The change of the sum.
    pub body_delta: i64,
    /// The part of the change that changed, added and removed functions explain.
    pub attributed: i64,
    /// `body_delta - attributed`: growth or shrinkage no listed function explains.
    pub unaccounted: i64,
    /// Functions matched by key, with the same or another size.
    pub matched: usize,
    /// The changes, largest first.
    pub changes: Vec<FunctionChange>,
    /// Accepted functions without a usable key, by local ID of the accepted build.
    pub unmatched_accepted: Vec<FunctionRef>,
    /// Functions now without a usable key, by local ID of this build.
    pub unmatched_now: Vec<FunctionRef>,
    /// Whether the accepted list names every function.
    pub accepted_complete: bool,
    /// Whether the list now names every function.
    pub now_complete: bool,
}

/// The entries of a list that have a key occurring once, and the others.
fn keyed(list: &FunctionList) -> (BTreeMap<FunctionKey, &FunctionEntry>, Vec<&FunctionEntry>) {
    let mut occurrences: BTreeMap<FunctionKey, usize> = BTreeMap::new();
    for key in list.largest.iter().filter_map(FunctionEntry::key) {
        *occurrences.entry(key).or_default() += 1;
    }
    let (mut unique, mut rest) = (BTreeMap::new(), Vec::new());
    for entry in &list.largest {
        match entry.key() {
            Some(key) if occurrences[&key] == 1 => {
                unique.insert(key, entry);
            }
            _ => rest.push(entry),
        }
    }
    (unique, rest)
}

/// Compares the function lists of two builds. Functions are matched by name
/// or export only, never by index; those without a usable key are listed as
/// unmatched. `None` if nothing differs.
#[must_use]
pub fn function_drift(accepted: &FunctionList, now: &FunctionList) -> Option<FunctionDrift> {
    let matching = pair(accepted, now);
    finish_drift(accepted, now, matching)
}

/// The result of pairing two lists by key.
struct Pairing {
    changes: Vec<FunctionChange>,
    matched: usize,
    unmatched_accepted: Vec<FunctionRef>,
    unmatched_now: Vec<FunctionRef>,
    keyed_any: bool,
}

fn pair(accepted: &FunctionList, now: &FunctionList) -> Pairing {
    let (was_keyed, was_rest) = keyed(accepted);
    let (now_keyed, now_rest) = keyed(now);
    let was_all: Vec<_> = accepted
        .largest
        .iter()
        .filter_map(FunctionEntry::key)
        .collect();
    let now_all: Vec<_> = now.largest.iter().filter_map(FunctionEntry::key).collect();
    let (accepted_complete, now_complete) = (accepted.complete(), now.complete());
    let reference = |entry: &FunctionEntry| FunctionRef {
        id: entry.id,
        size: entry.size,
    };

    let mut changes = Vec::new();
    let mut matched = 0;
    let mut unmatched_accepted: Vec<_> = was_rest.iter().map(|entry| reference(entry)).collect();
    let mut unmatched_now: Vec<_> = now_rest.iter().map(|entry| reference(entry)).collect();
    for (key, was) in &was_keyed {
        if let Some(is) = now_keyed.get(key) {
            matched += 1;
            if was.size != is.size {
                changes.push(FunctionChange {
                    kind: FunctionChangeKind::Changed,
                    key: key.clone(),
                    was: Some(was.size),
                    now: Some(is.size),
                });
            }
        } else if now_all.contains(key) {
            // The name occurs several times now: no single function to compare with.
            unmatched_accepted.push(reference(was));
        } else {
            changes.push(FunctionChange {
                kind: if now_complete {
                    FunctionChangeKind::Removed
                } else {
                    FunctionChangeKind::LeftLargest
                },
                key: key.clone(),
                was: Some(was.size),
                now: None,
            });
        }
    }
    for (key, is) in &now_keyed {
        if was_keyed.contains_key(key) {
            continue;
        }
        if was_all.contains(key) {
            unmatched_now.push(reference(is));
        } else {
            changes.push(FunctionChange {
                kind: if accepted_complete {
                    FunctionChangeKind::Added
                } else {
                    FunctionChangeKind::EnteredLargest
                },
                key: key.clone(),
                was: None,
                now: Some(is.size),
            });
        }
    }
    let keyed_any = !was_keyed.is_empty() || !now_keyed.is_empty();
    Pairing {
        changes,
        matched,
        unmatched_accepted,
        unmatched_now,
        keyed_any,
    }
}

fn finish_drift(
    accepted: &FunctionList,
    now: &FunctionList,
    pairing: Pairing,
) -> Option<FunctionDrift> {
    let Pairing {
        mut changes,
        matched,
        unmatched_accepted,
        unmatched_now,
        keyed_any,
    } = pairing;
    let (accepted_complete, now_complete) = (accepted.complete(), now.complete());
    changes.sort_by(|a, b| {
        (std::cmp::Reverse(a.magnitude()), &a.key).cmp(&(std::cmp::Reverse(b.magnitude()), &b.key))
    });

    let body_delta = now.body_bytes as i64 - accepted.body_bytes as i64;
    if changes.is_empty() && body_delta == 0 && accepted.count == now.count {
        return None;
    }
    let attributed: i64 = changes
        .iter()
        .map(|change| match change.kind {
            FunctionChangeKind::Changed => change.delta().unwrap_or(0),
            FunctionChangeKind::Added => change.now.unwrap_or(0) as i64,
            FunctionChangeKind::Removed => -(change.was.unwrap_or(0) as i64),
            FunctionChangeKind::EnteredLargest | FunctionChangeKind::LeftLargest => 0,
        })
        .sum();
    let unaccounted = body_delta - attributed;
    let unmatched = !unmatched_accepted.is_empty() || !unmatched_now.is_empty();
    let confidence = if !unmatched {
        Confidence::High
    } else if !keyed_any {
        Confidence::None
    } else {
        Confidence::Partial
    };
    let status = if confidence == Confidence::High
        && unaccounted == 0
        && accepted_complete
        && now_complete
    {
        Status::Exact
    } else {
        Status::Estimated
    };
    Some(FunctionDrift {
        status,
        confidence,
        was_body_bytes: accepted.body_bytes,
        now_body_bytes: now.body_bytes,
        body_delta,
        attributed,
        unaccounted,
        matched,
        changes,
        unmatched_accepted,
        unmatched_now,
        accepted_complete,
        now_complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(wat: &str) -> Snapshot {
        let bytes = wat::parse_str(wat).unwrap();
        Snapshot::from_inventory(&Inventory::from_bytes(&bytes).unwrap())
    }

    fn drift(accepted: &str, now: &str) -> Vec<Drift> {
        diff(&snapshot(accepted), &snapshot(now))
    }

    #[test]
    fn lists_are_sorted_and_internal_memories_are_numbered_among_themselves() {
        let s = snapshot(
            r#"(module
                (import "env" "memory" (memory 1))
                (memory (export "own") 1)
                (memory 2 4)
                (table 3 funcref)
                (func (export "b")) (func (export "a")))"#,
        );
        let names: Vec<_> = s.exports.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "own"]);
        assert_eq!(s.memories.len(), 1);
        assert_eq!(
            (s.memories[0].name.as_str(), s.memories[0].ty.as_str()),
            ("#0", "2..4")
        );
        assert_eq!(s.tables[0].ty, "funcref 3..");
        assert_eq!(s.imports[0].ty, "1..");
    }

    #[test]
    fn a_reordered_module_is_the_same_snapshot_and_makes_no_drift() {
        let a = r#"(module (import "env" "x" (func)) (import "env" "y" (func (param i32)))
            (func (export "f")) (func (export "g") (param i64)))"#;
        let b = r#"(module (import "env" "y" (func (param i32))) (import "env" "x" (func))
            (func (export "g") (param i64)) (func (export "f")))"#;
        assert_eq!(snapshot(a), snapshot(b));
        assert_eq!(drift(a, b), []);
    }

    #[test]
    fn a_new_import_is_added_and_a_dropped_one_is_removed() {
        let old = r#"(module (import "env" "log" (func (param i32))))"#;
        let new = r#"(module (import "env" "log" (func (param i32))) (import "env" "fetch" (func (result i32))))"#;
        assert!(matches!(
            drift(old, new).as_slice(),
            [Drift::Added { part: Part::Import, item }] if item.label() == "env.fetch"
        ));
        assert!(matches!(
            drift(new, old).as_slice(),
            [Drift::Removed {
                part: Part::Import,
                ..
            }]
        ));
    }

    #[test]
    fn the_same_import_with_another_signature_is_changed_with_both_types() {
        let drifts = drift(
            r#"(module (import "env" "f" (func (param i32))))"#,
            r#"(module (import "env" "f" (func (param i32 i32))))"#,
        );
        match drifts.as_slice() {
            [
                Drift::Changed {
                    part: Part::Import,
                    item,
                    was,
                },
            ] => {
                assert_eq!(
                    (was.as_str(), item.ty.as_str()),
                    ("(i32) -> ()", "(i32, i32) -> ()")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_import_name_alone_is_not_an_identity() {
        let modules = drift(
            r#"(module (import "a" "f" (func)))"#,
            r#"(module (import "b" "f" (func)))"#,
        );
        assert!(matches!(
            modules.as_slice(),
            [Drift::Removed { .. }, Drift::Added { .. }]
        ));

        let kinds = drift(
            r#"(module (import "env" "x" (func)))"#,
            r#"(module (import "env" "x" (memory 1)))"#,
        );
        assert!(matches!(
            kinds.as_slice(),
            [Drift::Removed { .. }, Drift::Added { .. }]
        ));
    }

    #[test]
    fn overloaded_imports_are_compared_as_a_set_of_types() {
        let one = r#"(module (import "env" "f" (func)) (import "env" "f" (func (param i32))))"#;
        let same = r#"(module (import "env" "f" (func (param i32))) (import "env" "f" (func)))"#;
        let third = r#"(module (import "env" "f" (func)) (import "env" "f" (func (param i32))) (import "env" "f" (func (param i64))))"#;
        assert_eq!(drift(one, same), []);
        assert!(matches!(
            drift(one, third).as_slice(),
            [Drift::Added { .. }]
        ));
    }

    #[test]
    fn a_memory_limit_change_is_reported_once_whether_exported_or_internal() {
        let exported = drift(
            r#"(module (memory (export "memory") 1 4))"#,
            r#"(module (memory (export "memory") 1 8))"#,
        );
        assert!(matches!(
            exported.as_slice(),
            [Drift::Changed { part: Part::Export, was, item }] if was == "1..4" && item.ty == "1..8"
        ));
        let internal = drift("(module (memory 1 4))", "(module (memory 1 8))");
        assert!(matches!(
            internal.as_slice(),
            [Drift::Changed {
                part: Part::Memory,
                ..
            }]
        ));
    }

    #[test]
    fn shared_memory_is_a_changed_limit_and_a_new_feature() {
        let drifts = drift(
            r#"(module (memory (export "memory") 1 4))"#,
            r#"(module (memory (export "memory") 1 4 shared))"#,
        );
        assert!(drifts.iter().any(|d| matches!(
            d,
            Drift::Changed { was, item, .. } if was == "1..4" && item.ty == "1..4 shared"
        )));
        assert!(drifts.contains(&Drift::FeatureAdded("shared_memory".to_string())));
        assert!(!drifts.iter().any(|d| matches!(d, Drift::FeatureRemoved(_))));
    }

    /// A function body of `n` `nop`s, so its size is known and differs per function.
    fn body(name: &str, n: usize) -> String {
        format!("(func ${name} {})", "nop ".repeat(n))
    }

    fn sized(parts: &[&str]) -> Snapshot {
        snapshot(&format!("(module {})", parts.join(" ")))
    }

    fn functions(accepted: &Snapshot, now: &Snapshot) -> Option<FunctionDrift> {
        function_drift(
            accepted.functions.as_ref().unwrap(),
            now.functions.as_ref().unwrap(),
        )
    }

    #[test]
    fn a_larger_function_body_is_matched_by_name_and_explains_the_change_exactly() {
        let was = sized(&[&body("parse", 10), &body("render", 20)]);
        let now = sized(&[&body("render", 20), &body("parse", 110)]);
        let drift = functions(&was, &now).unwrap();
        assert_eq!(drift.confidence, Confidence::High);
        assert_eq!(drift.status, Status::Exact);
        assert_eq!(drift.body_delta, 100);
        assert_eq!(drift.unaccounted, 0);
        assert_eq!(drift.matched, 2);
        match drift.changes.as_slice() {
            [change] => {
                assert_eq!(change.kind, FunctionChangeKind::Changed);
                assert_eq!(change.key, FunctionKey::Name("parse".to_string()));
                assert_eq!(change.delta(), Some(100));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_added_and_a_removed_function_are_told_apart_when_the_lists_are_complete() {
        let was = sized(&[&body("old", 5), &body("same", 5)]);
        let now = sized(&[&body("new", 8), &body("same", 5)]);
        let drift = functions(&was, &now).unwrap();
        let kinds: Vec<_> = drift.changes.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&FunctionChangeKind::Added));
        assert!(kinds.contains(&FunctionChangeKind::Removed));
        assert_eq!(drift.status, Status::Exact);
        assert_eq!(drift.unaccounted, 0);
    }

    #[test]
    fn stripped_modules_never_match_functions_by_index() {
        let anon = |n: usize| format!("(func {})", "nop ".repeat(n));
        // The same two bodies, swapped: by index the sizes would look exchanged.
        let was = sized(&[&anon(10), &anon(50)]);
        let swapped = sized(&[&anon(50), &anon(10)]);
        assert!(functions(&was, &swapped).is_none());

        let grown = sized(&[&anon(50), &anon(90)]);
        let drift = functions(&was, &grown).unwrap();
        assert_eq!(drift.confidence, Confidence::None);
        assert_eq!(drift.status, Status::Estimated);
        assert_eq!(drift.changes.len(), 0);
        assert_eq!(drift.matched, 0);
        assert_eq!(drift.unmatched_accepted.len(), 2);
        assert_eq!(drift.unmatched_now.len(), 2);
        assert_eq!(drift.unaccounted, drift.body_delta);
        assert!(drift.unmatched_now.iter().all(|f| f.id.is_some()));
    }

    #[test]
    fn exports_identify_functions_of_a_stripped_module() {
        let was = sized(&[r#"(func (export "run") nop)"#, "(func nop nop)"]);
        let now = sized(&[
            "(func nop nop nop)",
            r#"(func (export "run") nop nop nop nop)"#,
        ]);
        let drift = functions(&was, &now).unwrap();
        assert_eq!(drift.confidence, Confidence::Partial);
        assert_eq!(drift.status, Status::Estimated);
        assert_eq!(drift.changes[0].key, FunctionKey::Export("run".to_string()));
        assert_eq!(drift.changes[0].delta(), Some(3));
        assert_eq!(drift.unmatched_accepted.len(), 1);
        assert_eq!(drift.unmatched_now.len(), 1);
        // The unexplained part is the unmatched function that grew by one byte.
        assert_eq!(drift.unaccounted, 1);
    }

    #[test]
    fn a_name_that_occurs_twice_is_not_a_key() {
        let list = |sizes: [u64; 2]| FunctionList {
            count: 2,
            body_bytes: sizes.iter().sum(),
            largest: (0..2)
                .map(|i| FunctionEntry {
                    id: Some(i),
                    size: sizes[i as usize],
                    name: Some("dup".to_string()),
                    export: None,
                })
                .collect(),
        };
        let drift = function_drift(&list([5, 7]), &list([5, 9])).unwrap();
        assert_eq!(drift.changes.len(), 0);
        assert_eq!(drift.confidence, Confidence::None);
        assert_eq!(drift.unmatched_now.len(), 2);
    }

    #[test]
    fn a_truncated_list_does_not_call_a_function_added() {
        let list = |entries: &[(&str, u64)], count| FunctionList {
            count,
            body_bytes: entries.iter().map(|e| e.1).sum::<u64>() + 1_000,
            largest: entries
                .iter()
                .map(|(name, size)| FunctionEntry {
                    id: None,
                    size: *size,
                    name: Some((*name).to_string()),
                    export: None,
                })
                .collect(),
        };
        let was = list(&[("big", 500), ("mid", 400)], 50);
        let now = list(&[("big", 500), ("risen", 450)], 50);
        let drift = function_drift(&was, &now).unwrap();
        let kinds: Vec<_> = drift.changes.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&FunctionChangeKind::EnteredLargest));
        assert!(kinds.contains(&FunctionChangeKind::LeftLargest));
        assert!(!kinds.contains(&FunctionChangeKind::Added));
        assert_eq!(drift.status, Status::Estimated);
        assert!(!drift.accepted_complete);
    }

    #[test]
    fn sections_reconcile_and_name_the_part_that_changed() {
        let was = sized(&["(memory 1)", &body("f", 3), r#"(data (i32.const 0) "ab")"#]);
        let now = sized(&[
            "(memory 1)",
            &body("f", 3),
            &format!(r#"(data (i32.const 0) "{}")"#, "x".repeat(300)),
            r#"(@custom ".debug_info" "0123456789")"#,
        ]);
        let drift = section_drift(&was.sections.unwrap(), &now.sections.unwrap()).unwrap();
        let parts: Vec<_> = drift.changed.iter().map(|d| d.part).collect();
        assert_eq!(parts, [Region::Data, Region::Debug, Region::Overhead]);
        let total: i64 = drift.changed.iter().map(PartDelta::delta).sum();
        assert_eq!(total, drift.now_total as i64 - drift.was_total as i64);
        assert!(section_drift(&was.sections.unwrap(), &was.sections.unwrap()).is_none());
    }

    #[test]
    fn composition_adds_up_to_the_file_and_the_snapshot_stays_bounded() {
        let many: Vec<_> = (0..60).map(|i| body(&format!("f{i}"), i)).collect();
        let text = format!("(module {})", many.join(" "));
        let bytes = wat::parse_str(&text).unwrap();
        let inventory = Inventory::from_bytes(&bytes).unwrap();
        assert_eq!(inventory.composition.total(), bytes.len() as u64);
        assert_eq!(inventory.functions.bodies.len(), 60);
        let snapshot = Snapshot::from_inventory(&inventory);
        let functions = snapshot.functions.unwrap();
        assert_eq!(functions.count, 60);
        assert_eq!(functions.largest.len(), crate::inventory::LARGEST_FUNCTIONS);
        assert!(!functions.complete());
        assert_eq!(functions.largest[0].name.as_deref(), Some("f59"));
    }

    #[test]
    fn a_snapshot_without_sizes_reads_them_as_absent() {
        let s: Snapshot = serde_json::from_str(r#"{"exports":[]}"#).unwrap();
        assert!(s.sections.is_none() && s.functions.is_none());
    }

    #[test]
    fn a_snapshot_without_a_field_reads_it_as_empty_and_writes_it_compactly() {
        let s: Snapshot = serde_json::from_str(r#"{"features":["simd"]}"#).unwrap();
        assert!(s.imports.is_empty() && s.exports.is_empty());
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"features":["simd"]}"#
        );
    }
}
