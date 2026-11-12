//! The accepted inventory: the part of an [`Inventory`] the baseline keeps,
//! and the differences between two of them.
//!
//! A [`Snapshot`] holds what a deployment depends on: the imports and exports
//! with their types, the memories and tables the module declares for itself,
//! and the features the binary needs. It is compact (one short entry per
//! declaration) and deterministic (every list is sorted), so reordering a
//! module changes nothing. Later parts add fields next to these; a field the
//! file lacks reads as absent, never as a changed value.
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

use crate::inventory::{ExternType, Feature, Inventory, MemoryType, TableType};

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
