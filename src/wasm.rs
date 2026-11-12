//! What a WebAssembly module declares: its section sizes and the memory it
//! asks for.

use wasmparser::{Parser, Payload};

const DEFAULT_PAGE: u64 = 65_536;

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
}

/// Parses the module and reads its section sizes and declared memory.
///
/// # Errors
///
/// Fails if `bytes` is not a valid WebAssembly module.
pub fn inspect(bytes: &[u8]) -> wasmparser::Result<Inspection> {
    let mut inspection = Inspection::default();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload? {
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory?;
                    let page = memory
                        .page_size_log2
                        .map_or(DEFAULT_PAGE, |log2| 1u64 << log2);
                    *inspection.initial_memory.get_or_insert(0) += memory.initial * page;
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

fn span(range: &std::ops::Range<u64>) -> u64 {
    range.end - range.start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(wat: &str) -> Vec<u8> {
        wat::parse_str(wat).unwrap()
    }

    #[test]
    fn reads_the_declared_memory_in_bytes() {
        let bytes = module("(module (memory 3))");
        assert_eq!(inspect(&bytes).unwrap().initial_memory, Some(3 * 65_536));
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
        assert!(inspect(b"not wasm").is_err());
    }
}
