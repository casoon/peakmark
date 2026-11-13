//! The optional compatibility policy of a module: the only way the inventory
//! can fail a check.
//!
//! Differences from the accepted inventory are warnings (see
//! [`crate::snapshot`]). A policy is a decision the project wrote down, so a
//! module that breaks it fails the check, and accepting a baseline does not
//! change that. Three lists exist, each optional:
//!
//! - `require_exports`: exports the module must have, optionally with the
//!   signature of a function.
//! - `allow_imports`: the only imports the module may have. Once the key is
//!   present, anything not listed is a violation, and an empty list allows no
//!   import at all.
//! - `allow_features`: the only [`Feature`]s the binary may need, same rule.
//!
//! Strict where peakmark does not know: an import that is not listed, a
//! listed import with another signature, an export of another kind than the
//! signature names, and, when `allow_features` is set, a binary with
//! instructions peakmark cannot assign to a feature, are all violations.
//! A policy is about the bytes; it states nothing about what the host grants
//! an import or whether the module is secure.

use serde::{Deserialize, Serialize};

use crate::inventory::{ExternType, Feature, Inventory};
use crate::snapshot::{Kind, feature_id, type_text};

/// The `[module.policy]` table.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Exports the module must have.
    #[serde(default)]
    pub require_exports: Vec<ExportRule>,
    /// The imports the module may have; `None` leaves imports ungated.
    pub allow_imports: Option<Vec<ImportRule>>,
    /// The features the binary may need; `None` leaves features ungated.
    pub allow_features: Option<Vec<Feature>>,
}

impl Policy {
    /// Whether the policy gates nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.require_exports.is_empty()
            && self.allow_imports.is_none()
            && self.allow_features.is_none()
    }
}

/// An export the module must have.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRule {
    /// The export name.
    pub name: String,
    /// If given, the export must be a function with this signature, written as
    /// `check --details` prints it: `(i32, i32) -> i32`. Spaces do not matter.
    pub signature: Option<String>,
}

/// An import the module may have.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRule {
    /// The import module.
    pub module: String,
    /// The import field. Module and field together name the import.
    pub name: String,
    /// If given, the import must be a function with this signature; without
    /// it any type of this module and field is allowed.
    pub signature: Option<String>,
}

/// A broken policy rule, with the declaration it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "violation", rename_all = "snake_case")]
pub enum Violation {
    /// A required export is not there.
    ExportMissing {
        /// The export name.
        name: String,
        /// The signature the policy asks for.
        signature: Option<String>,
    },
    /// A required export exists with another signature, or is not a function.
    ExportType {
        /// The export name.
        name: String,
        /// The signature the policy asks for.
        expected: String,
        /// What the module exports: `func (i32) -> i32`, `memory 1..`.
        found: String,
    },
    /// An import that the allow-list does not name.
    ImportNotAllowed {
        /// The import module.
        module: String,
        /// The import field.
        name: String,
        /// What it is: `func (i32) -> i32`.
        found: String,
    },
    /// An allowed import with another signature than the policy accepts.
    ImportType {
        /// The import module.
        module: String,
        /// The import field.
        name: String,
        /// The signatures the policy accepts for it.
        expected: Vec<String>,
        /// What the module imports.
        found: String,
    },
    /// A feature the binary needs that the policy does not allow.
    FeatureNotAllowed {
        /// The feature, as in `--json`.
        feature: String,
        /// Declarations that need it.
        declarations: u64,
        /// Instructions that need it.
        instructions: u64,
    },
    /// Features cannot be checked because instructions are unclassified.
    FeaturesUnverifiable {
        /// The proposals peakmark has no feature for, with their instruction counts.
        unclassified: Vec<(String, u64)>,
    },
}

impl Violation {
    /// The rule id a finding about this violation carries.
    #[must_use]
    pub fn rule(&self) -> &'static str {
        match self {
            Self::ExportMissing { .. } => "policy/export-missing",
            Self::ExportType { .. } => "policy/export-signature",
            Self::ImportNotAllowed { .. } => "policy/import-not-allowed",
            Self::ImportType { .. } => "policy/import-signature",
            Self::FeatureNotAllowed { .. } => "policy/feature-not-allowed",
            Self::FeaturesUnverifiable { .. } => "policy/feature-unverifiable",
        }
    }

    /// The declaration the violation is about, in a sentence.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::ExportMissing { name, signature } => match signature {
                Some(signature) => format!("Required export {name} {signature} is missing"),
                None => format!("Required export {name} is missing"),
            },
            Self::ExportType {
                name,
                expected,
                found,
            } => format!("Export {name} is {found}, the policy requires func {expected}"),
            Self::ImportNotAllowed {
                module,
                name,
                found,
            } => format!("Import {module}.{name} ({found}) is not in allow_imports"),
            Self::ImportType {
                module,
                name,
                expected,
                found,
            } => format!(
                "Import {module}.{name} is {found}, allow_imports accepts func {}",
                expected.join(" or func ")
            ),
            Self::FeatureNotAllowed {
                feature,
                declarations,
                instructions,
            } => format!(
                "The binary needs {feature} ({declarations} declarations, {instructions} instructions), which allow_features does not list"
            ),
            Self::FeaturesUnverifiable { unclassified } => format!(
                "allow_features cannot be checked: instructions of {} are not assigned to any feature",
                unclassified
                    .iter()
                    .map(|(proposal, count)| format!("{proposal} ({count})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// What to do about it. Accepting the baseline is never one of the options.
    #[must_use]
    pub fn action(&self) -> String {
        match self {
            Self::ExportMissing { name, .. } => {
                format!("Export {name} from the module, or remove it from require_exports")
            }
            Self::ExportType { name, .. } => format!(
                "Restore the signature of export {name}, or change it in require_exports on purpose"
            ),
            Self::ImportNotAllowed { module, name, .. } => format!(
                "Remove the import {module}.{name}, or add it to allow_imports on purpose"
            ),
            Self::ImportType { module, name, .. } => format!(
                "Restore the signature of import {module}.{name}, or change it in allow_imports on purpose"
            ),
            Self::FeatureNotAllowed { feature, .. } => format!(
                "Build without {feature}, or add it to allow_features on purpose"
            ),
            Self::FeaturesUnverifiable { .. } => {
                "Update peakmark to a version that classifies these instructions, or remove allow_features"
                    .to_string()
            }
        }
    }
}

fn normalized(signature: &str) -> String {
    signature.split_whitespace().collect()
}

fn describe(ty: &ExternType) -> (Kind, String, String) {
    let (kind, text) = type_text(ty);
    let full = format!("{kind} {text}");
    (kind, text, full)
}

/// Judges `inventory` against `policy`: every broken rule, in the order of the
/// policy's lists, then the order of the module.
#[must_use]
pub fn evaluate(policy: &Policy, inventory: &Inventory) -> Vec<Violation> {
    let mut violations = Vec::new();

    for rule in &policy.require_exports {
        let Some(export) = inventory.exports.iter().find(|e| e.name == rule.name) else {
            violations.push(Violation::ExportMissing {
                name: rule.name.clone(),
                signature: rule.signature.clone(),
            });
            continue;
        };
        if let Some(expected) = &rule.signature {
            let (kind, text, full) = describe(&export.ty);
            if kind != Kind::Func || normalized(&text) != normalized(expected) {
                violations.push(Violation::ExportType {
                    name: rule.name.clone(),
                    expected: expected.clone(),
                    found: full,
                });
            }
        }
    }

    if let Some(allowed) = &policy.allow_imports {
        for import in &inventory.imports {
            let rules: Vec<_> = allowed
                .iter()
                .filter(|rule| rule.module == import.module && rule.name == import.name)
                .collect();
            let (kind, text, full) = describe(&import.ty);
            if rules.is_empty() {
                violations.push(Violation::ImportNotAllowed {
                    module: import.module.clone(),
                    name: import.name.clone(),
                    found: full,
                });
                continue;
            }
            let accepted = rules.iter().any(|rule| {
                rule.signature.as_ref().is_none_or(|expected| {
                    kind == Kind::Func && normalized(&text) == normalized(expected)
                })
            });
            if !accepted {
                violations.push(Violation::ImportType {
                    module: import.module.clone(),
                    name: import.name.clone(),
                    expected: rules
                        .iter()
                        .filter_map(|rule| rule.signature.clone())
                        .collect(),
                    found: full,
                });
            }
        }
    }

    if let Some(allowed) = &policy.allow_features {
        for used in &inventory.features {
            if !allowed.contains(&used.feature) {
                violations.push(Violation::FeatureNotAllowed {
                    feature: feature_id(used.feature),
                    declarations: used.declarations,
                    instructions: used.instructions,
                });
            }
        }
        let unclassified = &inventory.coverage.unclassified_instructions;
        if !unclassified.is_empty() {
            violations.push(Violation::FeaturesUnverifiable {
                unclassified: unclassified
                    .iter()
                    .map(|(proposal, count)| (proposal.clone(), *count))
                    .collect(),
            });
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory(wat: &str) -> Inventory {
        Inventory::from_bytes(&wat::parse_str(wat).unwrap()).unwrap()
    }

    fn policy(toml: &str) -> Policy {
        toml::from_str(toml).unwrap()
    }

    const MODULE: &str = r#"(module
        (import "env" "log" (func (param i32)))
        (memory (export "memory") 1)
        (func (export "add") (param i32 i32) (result i32) local.get 0))"#;

    #[test]
    fn a_module_that_keeps_to_its_policy_has_no_violation() {
        let policy = policy(
            r#"require_exports = [{ name = "add", signature = "(i32,i32)->i32" }, { name = "memory" }]
               allow_imports = [{ module = "env", name = "log", signature = "(i32) -> ()" }]
               allow_features = []"#,
        );
        assert_eq!(evaluate(&policy, &inventory(MODULE)), []);
    }

    #[test]
    fn a_missing_export_and_a_changed_signature_name_the_declaration() {
        let policy = policy(
            r#"require_exports = [{ name = "add", signature = "(i64, i64) -> i64" },
                                  { name = "memory", signature = "(i32) -> ()" },
                                  { name = "gone" }]"#,
        );
        let violations = evaluate(&policy, &inventory(MODULE));
        assert!(
            matches!(&violations[0], Violation::ExportType { name, found, .. }
            if name == "add" && found == "func (i32, i32) -> i32")
        );
        assert!(
            matches!(&violations[1], Violation::ExportType { name, found, .. }
            if name == "memory" && found == "memory 1..")
        );
        assert!(matches!(&violations[2], Violation::ExportMissing { name, .. } if name == "gone"));
        assert_eq!(violations[0].rule(), "policy/export-signature");
        assert!(violations[2].action().contains("require_exports"));
    }

    #[test]
    fn once_allow_imports_is_set_everything_unlisted_is_refused_even_with_the_same_name() {
        let none = policy("allow_imports = []");
        assert!(matches!(
            evaluate(&none, &inventory(MODULE)).as_slice(),
            [Violation::ImportNotAllowed { module, name, .. }] if module == "env" && name == "log"
        ));

        let other_module = policy(r#"allow_imports = [{ module = "other", name = "log" }]"#);
        assert_eq!(evaluate(&other_module, &inventory(MODULE)).len(), 1);

        let other_type = policy(
            r#"allow_imports = [{ module = "env", name = "log", signature = "(i64) -> ()" }]"#,
        );
        assert!(matches!(
            evaluate(&other_type, &inventory(MODULE)).as_slice(),
            [Violation::ImportType { expected, .. }] if expected == &["(i64) -> ()"]
        ));

        let any_type = policy(r#"allow_imports = [{ module = "env", name = "log" }]"#);
        assert_eq!(evaluate(&any_type, &inventory(MODULE)), []);
    }

    #[test]
    fn an_unlisted_feature_is_refused_and_an_unset_list_gates_nothing() {
        let threads = inventory(r"(module (memory 1 2 shared) (func v128.const i64x2 0 0 drop))");
        let allowed = policy(r#"allow_features = ["simd"]"#);
        let violations = evaluate(&allowed, &threads);
        assert!(violations.iter().any(|v| matches!(v,
            Violation::FeatureNotAllowed { feature, .. } if feature == "shared_memory")));
        assert!(!violations.iter().any(|v| matches!(v,
            Violation::FeatureNotAllowed { feature, .. } if feature == "simd")));
        assert_eq!(evaluate(&Policy::default(), &threads), []);
    }

    #[test]
    fn unknown_keys_and_unknown_features_do_not_parse() {
        assert!(toml::from_str::<Policy>("allow_feature = []").is_err());
        assert!(toml::from_str::<Policy>(r#"allow_features = ["telepathy"]"#).is_err());
        assert!(toml::from_str::<Policy>(r#"allow_imports = [{ module = "a" }]"#).is_err());
    }

    #[test]
    fn features_that_cannot_be_assigned_make_an_allow_list_unverifiable() {
        let mut open = inventory("(module)");
        open.coverage
            .unclassified_instructions
            .insert("future-proposal".to_string(), 2);
        let violations = evaluate(&policy("allow_features = []"), &open);
        assert!(matches!(
            violations.as_slice(),
            [Violation::FeaturesUnverifiable { unclassified }] if unclassified.len() == 1
        ));
        assert_eq!(evaluate(&Policy::default(), &open), []);
    }
}
