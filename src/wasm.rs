//! What a WebAssembly module declares: its section sizes and the memory it
//! asks for.

use std::fmt;

use wasmparser::{
    BinaryReaderError, Encoding, FuncValidatorAllocations, Parser, Payload, TypeRef, ValidPayload,
    Validator, WasmFeatures,
};

const DEFAULT_PAGE: u64 = 65_536;

/// The WebAssembly features an artifact may use: everything in WebAssembly 3.0.
///
/// That covers what Rust, C and C++ toolchains emit (bulk memory, SIMD,
/// threads, exception handling, multiple and 64-bit memories, tail calls, GC).
/// Proposals that are not part of 3.0 are refused as unsupported.
const SUPPORTED: WasmFeatures = WasmFeatures::WASM3;

/// Why an artifact cannot be measured.
#[derive(Debug)]
pub enum InspectError {
    /// The bytes are not a valid core module.
    Invalid(BinaryReaderError),
    /// The module is valid, but uses a feature outside the supported set.
    UnsupportedFeature(BinaryReaderError),
    /// The artifact is a component, not a core module.
    Component,
}

impl fmt::Display for InspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) | Self::UnsupportedFeature(error) => error.fmt(f),
            Self::Component => write!(f, "a component, not a core module"),
        }
    }
}

impl std::error::Error for InspectError {}

/// Bytes per section kind that is reported.
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct Sections {
    /// The code section.
    pub code: u64,
    /// The data section.
    pub data: u64,
    /// All custom sections together (names, debug info, producers, …).
    pub custom: u64,
}

/// What [`inspect`] reads from a module.
#[derive(Debug, Default, Clone, Copy)]
pub struct Inspection {
    /// Section sizes.
    pub sections: Sections,
    /// What the memory section asks for in bytes; `None` if it has none.
    pub initial_memory: Option<u64>,
    /// How many memories the module declares, imported ones included.
    pub memory_count: u32,
}

/// Validates the module completely, then reads its section sizes and declared
/// memory.
///
/// # Errors
///
/// Fails if `bytes` is not a valid core module, uses a feature outside the
/// supported set, or is a component.
pub fn inspect(bytes: &[u8]) -> Result<Inspection, InspectError> {
    validate(bytes)?;
    let mut inspection = Inspection::default();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(InspectError::Invalid)? {
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory.map_err(InspectError::Invalid)?;
                    inspection.memory_count += 1;
                    let page = memory
                        .page_size_log2
                        .map_or(DEFAULT_PAGE, |log2| 1u64 << log2);
                    let declared = inspection.initial_memory.get_or_insert(0);
                    *declared = declared.saturating_add(memory.initial.saturating_mul(page));
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(InspectError::Invalid)?;
                    if matches!(import.ty, TypeRef::Memory(_)) {
                        inspection.memory_count += 1;
                    }
                }
            }
            Payload::CodeSectionStart { size, .. } => inspection.sections.code = u64::from(size),
            Payload::DataSection(reader) => inspection.sections.data = span(&reader.range()),
            Payload::CustomSection(reader) => inspection.sections.custom += span(&reader.range()),
            _ => {}
        }
    }
    Ok(inspection)
}

/// Runs the whole module through the validator, function bodies included.
fn validate(bytes: &[u8]) -> Result<(), InspectError> {
    let mut validator = Validator::new_with_features(SUPPORTED);
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(InspectError::Invalid)?;
        if matches!(
            payload,
            Payload::Version {
                encoding: Encoding::Component,
                ..
            }
        ) {
            return Err(InspectError::Component);
        }
        // The validator hands function bodies back; validating them is up to us.
        let checked = validator.payload(&payload).and_then(|valid| match valid {
            ValidPayload::Func(function, body) => function
                .into_validator(FuncValidatorAllocations::default())
                .validate(&body),
            _ => Ok(()),
        });
        if let Err(error) = checked {
            return Err(classify(bytes, error));
        }
    }
    Ok(())
}

/// A module that only validates with every feature on is valid but unsupported.
fn classify(bytes: &[u8], error: BinaryReaderError) -> InspectError {
    if Validator::new_with_features(WasmFeatures::all())
        .validate_all(bytes)
        .is_ok()
    {
        InspectError::UnsupportedFeature(error)
    } else {
        InspectError::Invalid(error)
    }
}

fn span(range: &std::ops::Range<u64>) -> u64 {
    range.end - range.start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(wat: &str) -> Vec<u8> {
        wat::parse_str(wat).unwrap()
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    /// `drop` on an empty stack: the payloads parse, the function body does not type.
    const DROP_ON_EMPTY_STACK: &str = "0061736d01000000010401600000030201000a050103001a0b";

    #[test]
    fn reads_the_declared_memory_in_bytes() {
        let bytes = module("(module (memory 3))");
        assert_eq!(inspect(&bytes).unwrap().initial_memory, Some(3 * 65_536));
    }

    #[test]
    fn counts_defined_and_imported_memories() {
        let bytes = module(r#"(module (import "env" "m" (memory 1)) (memory 1) (memory 2))"#);
        assert_eq!(inspect(&bytes).unwrap().memory_count, 3);
        assert_eq!(inspect(&module("(module (func))")).unwrap().memory_count, 0);
    }

    #[test]
    fn a_module_without_memory_declares_none() {
        let bytes = module("(module (func))");
        assert_eq!(inspect(&bytes).unwrap().initial_memory, None);
    }

    #[test]
    fn measures_code_and_data_sections() {
        let bytes =
            module(r#"(module (memory 1) (func (export "f")) (data (i32.const 0) "abcdefgh"))"#);
        let sections = inspect(&bytes).unwrap().sections;
        assert!(sections.code > 0);
        assert!(sections.data >= 8);
    }

    #[test]
    fn refuses_bytes_that_are_not_a_module() {
        assert!(matches!(
            inspect(b"not wasm"),
            Err(InspectError::Invalid(_))
        ));
    }

    #[test]
    fn refuses_a_function_body_that_does_not_type() {
        let error = inspect(&hex(DROP_ON_EMPTY_STACK)).unwrap_err();
        assert!(matches!(error, InspectError::Invalid(_)), "{error}");
    }

    #[test]
    fn refuses_a_memory_whose_minimum_exceeds_its_maximum() {
        // Memory section with limits min 2, max 1.
        let bytes = hex("0061736d01000000050401010201");
        assert!(matches!(inspect(&bytes), Err(InspectError::Invalid(_))));
    }

    #[test]
    fn refuses_a_memory_larger_than_the_address_space() {
        // Memory section with a minimum of 65537 pages (LEB128 818004).
        let bytes = hex("0061736d0100000005050100818004");
        assert!(matches!(inspect(&bytes), Err(InspectError::Invalid(_))));
    }

    #[test]
    fn refuses_a_truncated_module() {
        let bytes = module("(module (memory 1) (func))");
        assert!(inspect(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn recognises_a_component_instead_of_measuring_it() {
        let bytes = module("(component)");
        assert!(matches!(inspect(&bytes), Err(InspectError::Component)));
    }

    #[test]
    fn a_component_that_nests_a_module_is_still_a_component() {
        let bytes = module("(component (core module (memory 1)))");
        assert!(matches!(inspect(&bytes), Err(InspectError::Component)));
    }

    #[test]
    fn accepts_the_features_of_webassembly_3() {
        let bytes = module(
            "(module (memory 1 1 shared) (memory i64 1) (func (result v128) v128.const i64x2 0 0))",
        );
        assert!(inspect(&bytes).is_ok());
    }

    #[test]
    fn a_feature_outside_the_set_is_unsupported_not_invalid() {
        // A custom page size is a proposal that WebAssembly 3.0 does not include.
        let bytes = module("(module (memory 1 (pagesize 1)))");
        assert!(matches!(
            inspect(&bytes),
            Err(InspectError::UnsupportedFeature(_))
        ));
    }
}
