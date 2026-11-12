//! What a WebAssembly artifact contains and requires, read from the bytes
//! alone.
//!
//! An [`Inventory`] lists the imports and exports with their types, the
//! memories, tables, globals and tags, the features the binary needs, every
//! section with its byte accounting, and the custom sections. Nothing is
//! instantiated and no glue runs.
//!
//! Get one with [`Inventory::from_bytes`], or from [`crate::wasm::inspect`],
//! which `check` uses: the module is validated first, so an artifact that is
//! invalid, outside the supported feature set or a component never reaches the
//! inventory.
//!
//! # Derived, not declared
//!
//! [`Inventory::features`] is derived from what the binary contains:
//! declarations (a shared memory, a tag, a `v128` in a signature) and
//! instructions, classified by the proposal they come from. It is a statement
//! about the bytes, not about execution: a feature used only in code that never
//! runs is still listed, and a requirement that leaves no trace in the binary
//! cannot be listed. [`Coverage`] says what the inventory could not decide.
//! Advertised metadata (`target_features`, `producers`) is reported on its own
//! and never turned into a requirement.
//!
//! # Schema
//!
//! The serialized form is versioned by [`SCHEMA_VERSION`] and deterministic:
//! lists follow the order of the module, features follow [`Feature`]'s order.
//! Adding a field or a list entry keeps the version; renaming or removing one
//! raises it.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Serialize, Serializer};
use wasmparser::{
    BinaryReader, BinaryReaderError, BlockType, ConstExpr, DataKind, ElementItems, ElementKind,
    ExternalKind, HeapType, KnownCustom, Name, Operator, Parser, Payload, RecGroup, RefType,
    StorageType, TableInit, TypeRef, ValType,
};

use crate::wasm::{InspectError, inspect};

/// Version of the serialized inventory. It changes when a field is renamed or
/// removed, never when one is added.
pub const SCHEMA_VERSION: u32 = 1;

/// Bytes before the first section: the magic number and the binary version.
pub const PREAMBLE_BYTES: u64 = 8;

/// Everything the artifact states about itself that peakmark reads.
#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    /// [`SCHEMA_VERSION`] of this document.
    pub schema_version: u32,
    /// What kind of artifact this is.
    pub artifact: Artifact,
    /// How many functions, types and segments the module has.
    pub counts: Counts,
    /// Imports in module order.
    pub imports: Vec<Import>,
    /// Exports in module order.
    pub exports: Vec<Export>,
    /// Every memory, imported ones first, in index order.
    pub memories: Vec<Memory>,
    /// Every table, imported ones first, in index order.
    pub tables: Vec<Table>,
    /// Every global, imported ones first, in index order.
    pub globals: Vec<Global>,
    /// Every tag, imported ones first, in index order.
    pub tags: Vec<Tag>,
    /// The features the binary needs, derived from declarations and
    /// instructions. See the [module documentation](self).
    pub features: Vec<FeatureUse>,
    /// Every section in file order. With [`PREAMBLE_BYTES`] they add up to the
    /// size of the file, see [`Inventory::accounted_bytes`].
    pub sections: Vec<Section>,
    /// The custom sections, a subset of [`Inventory::sections`] with their kind.
    pub custom_sections: Vec<CustomSection>,
    /// The state of the `name` section.
    pub names: Names,
    /// The `producers` section, if any.
    pub producers: Vec<Producer>,
    /// The `target_features` section, if any. A claim of the toolchain.
    pub target_features: Vec<TargetFeature>,
    /// What this inventory could not decide.
    pub coverage: Coverage,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            artifact: Artifact::default(),
            counts: Counts::default(),
            imports: Vec::new(),
            exports: Vec::new(),
            memories: Vec::new(),
            tables: Vec::new(),
            globals: Vec::new(),
            tags: Vec::new(),
            features: Vec::new(),
            sections: Vec::new(),
            custom_sections: Vec::new(),
            names: Names::Absent,
            producers: Vec::new(),
            target_features: Vec::new(),
            coverage: Coverage::default(),
        }
    }
}

impl Inventory {
    /// Validates `bytes` and reads the inventory.
    ///
    /// # Errors
    ///
    /// Fails like [`crate::wasm::inspect`]: if `bytes` is not a valid core
    /// module, uses a feature outside the supported set, or is a component.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, InspectError> {
        inspect(bytes).map(|inspection| inspection.inventory)
    }

    /// The bytes the preamble and all sections account for. It equals the size
    /// of the file.
    #[must_use]
    pub fn accounted_bytes(&self) -> u64 {
        PREAMBLE_BYTES
            + self
                .sections
                .iter()
                .map(|section| section.header + section.payload)
                .sum::<u64>()
    }

    /// The feature entry for `feature`, if the binary needs it.
    #[must_use]
    pub fn feature(&self, feature: Feature) -> Option<&FeatureUse> {
        self.features.iter().find(|used| used.feature == feature)
    }
}

/// The kind and format of the artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Artifact {
    /// What the artifact is.
    pub kind: ArtifactKind,
    /// The binary format version in the preamble.
    pub version: u32,
}

impl Default for Artifact {
    fn default() -> Self {
        Self {
            kind: ArtifactKind::CoreModule,
            version: 1,
        }
    }
}

/// What kind of artifact was inspected. Components are refused before an
/// inventory exists ([`InspectError::Component`]), so there is one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// A WebAssembly core module.
    CoreModule,
}

/// Counts of what the module defines and declares.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Counts {
    /// Types in the type section, of every kind.
    pub types: u32,
    /// Functions the module imports.
    pub imported_functions: u32,
    /// Functions the module defines.
    pub defined_functions: u32,
    /// Imports of every kind.
    pub imports: u32,
    /// Exports of every kind.
    pub exports: u32,
    /// Data segments.
    pub data_segments: u32,
    /// Element segments.
    pub element_segments: u32,
}

/// A value type. Reference types keep the text form of the binary reader, for
/// example `funcref` or `(ref null 3)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    /// 32-bit integer.
    I32,
    /// 64-bit integer.
    I64,
    /// 32-bit float.
    F32,
    /// 64-bit float.
    F64,
    /// 128-bit vector.
    V128,
    /// A reference type in its text form.
    Ref(String),
}

impl From<ValType> for ValueType {
    fn from(ty: ValType) -> Self {
        match ty {
            ValType::I32 => Self::I32,
            ValType::I64 => Self::I64,
            ValType::F32 => Self::F32,
            ValType::F64 => Self::F64,
            ValType::V128 => Self::V128,
            ValType::Ref(reference) => Self::from(reference),
        }
    }
}

impl From<RefType> for ValueType {
    fn from(reference: RefType) -> Self {
        Self::Ref(reference.to_string())
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I32 => f.write_str("i32"),
            Self::I64 => f.write_str("i64"),
            Self::F32 => f.write_str("f32"),
            Self::F64 => f.write_str("f64"),
            Self::V128 => f.write_str("v128"),
            Self::Ref(text) => f.write_str(text),
        }
    }
}

impl Serialize for ValueType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// The parameters and results of a function or tag.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct FuncSignature {
    /// Parameter types.
    pub params: Vec<ValueType>,
    /// Result types.
    pub results: Vec<ValueType>,
}

impl fmt::Display for FuncSignature {
    /// `(i32, i32) -> i32`; several results are parenthesised, none is `()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |types: &[ValueType]| {
            types
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        write!(f, "({}) -> ", list(&self.params))?;
        match self.results.as_slice() {
            [one] => write!(f, "{one}"),
            many => write!(f, "({})", list(many)),
        }
    }
}

/// The limits and flags of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MemoryType {
    /// Minimum size in pages.
    pub minimum: u64,
    /// Maximum size in pages; `None` if unbounded.
    pub maximum: Option<u64>,
    /// Whether the memory is shared between threads.
    pub shared: bool,
    /// Whether the memory is addressed with 64 bits.
    pub memory64: bool,
    /// Bytes per page: 65 536 unless the module declares a custom size.
    pub page_size: u64,
}

impl MemoryType {
    /// The minimum size in bytes.
    #[must_use]
    pub fn minimum_bytes(&self) -> u128 {
        u128::from(self.minimum) * u128::from(self.page_size)
    }

    /// The maximum size in bytes; `None` if unbounded.
    #[must_use]
    pub fn maximum_bytes(&self) -> Option<u128> {
        self.maximum
            .map(|pages| u128::from(pages) * u128::from(self.page_size))
    }
}

/// The element type and limits of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableType {
    /// The type of the elements.
    pub element: ValueType,
    /// Minimum number of elements.
    pub minimum: u64,
    /// Maximum number of elements; `None` if unbounded.
    pub maximum: Option<u64>,
    /// Whether the table is indexed with 64 bits.
    pub table64: bool,
    /// Whether the table is shared.
    pub shared: bool,
}

/// The value type and mutability of a global.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GlobalType {
    /// The type of the value.
    pub value: ValueType,
    /// Whether the global can be written.
    pub mutable: bool,
    /// Whether the global is shared.
    pub shared: bool,
}

/// The type of an import or export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternType {
    /// A function and its signature.
    Function(FuncSignature),
    /// A table.
    Table(TableType),
    /// A memory.
    Memory(MemoryType),
    /// A global.
    Global(GlobalType),
    /// A tag and its parameter types.
    Tag(FuncSignature),
}

/// Where an imported entity comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportName {
    /// The import module.
    pub module: String,
    /// The import field.
    pub name: String,
}

/// An import with its type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Import {
    /// The import module.
    pub module: String,
    /// The import field.
    pub name: String,
    /// What is imported.
    #[serde(rename = "type")]
    pub ty: ExternType,
}

/// An export with its index and type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Export {
    /// The export name.
    pub name: String,
    /// Index in the index space of its kind.
    pub index: u32,
    /// What is exported.
    #[serde(rename = "type")]
    pub ty: ExternType,
}

/// A memory in the module's index space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Memory {
    /// Memory index.
    pub index: u32,
    /// Set if the memory is imported.
    pub imported_from: Option<ImportName>,
    /// Export names, empty if the memory is internal.
    pub exported_as: Vec<String>,
    /// Limits and flags.
    #[serde(rename = "type")]
    pub ty: MemoryType,
}

/// A table in the module's index space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    /// Table index.
    pub index: u32,
    /// Set if the table is imported.
    pub imported_from: Option<ImportName>,
    /// Export names, empty if the table is internal.
    pub exported_as: Vec<String>,
    /// Element type and limits.
    #[serde(rename = "type")]
    pub ty: TableType,
}

/// A global in the module's index space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Global {
    /// Global index.
    pub index: u32,
    /// Set if the global is imported.
    pub imported_from: Option<ImportName>,
    /// Export names, empty if the global is internal.
    pub exported_as: Vec<String>,
    /// Value type and mutability.
    #[serde(rename = "type")]
    pub ty: GlobalType,
}

/// A tag in the module's index space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tag {
    /// Tag index.
    pub index: u32,
    /// Set if the tag is imported.
    pub imported_from: Option<ImportName>,
    /// Export names, empty if the tag is internal.
    pub exported_as: Vec<String>,
    /// The types of the values the tag carries.
    #[serde(rename = "type")]
    pub ty: FuncSignature,
}

/// A core feature a binary can need. The order is the order of the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// 128-bit vector instructions and the `v128` type.
    Simd,
    /// Relaxed SIMD instructions, which may differ between engines.
    RelaxedSimd,
    /// A shared memory.
    SharedMemory,
    /// Atomic memory instructions, `memory.atomic.wait` and `notify`.
    Atomics,
    /// Acquire/release atomics.
    AcquireReleaseAtomics,
    /// Bulk memory instructions and passive data segments.
    BulkMemory,
    /// Reference types: `externref`, several tables, table instructions.
    ReferenceTypes,
    /// Typed function references and non-nullable or indexed reference types.
    FunctionReferences,
    /// Garbage-collected types: structs, arrays, subtyping, `anyref` and kin.
    Gc,
    /// Exception handling with `try_table`, or tags.
    ExceptionHandling,
    /// The legacy `try`/`catch` exception instructions.
    LegacyExceptionHandling,
    /// `return_call` and `return_call_indirect`.
    TailCall,
    /// More than one memory.
    MultiMemory,
    /// 64-bit memories or tables.
    Memory64,
    /// Functions with several results, blocks with a function type.
    MultiValue,
    /// Sign-extension instructions.
    SignExtension,
    /// Saturating float-to-int conversions.
    SaturatingFloatToInt,
    /// Mutable globals crossing the module boundary.
    MutableGlobals,
    /// Arithmetic in constant expressions.
    ExtendedConst,
    /// Wide arithmetic instructions.
    WideArithmetic,
    /// Memory control instructions.
    MemoryControl,
    /// Stack switching.
    StackSwitching,
    /// Shared-everything threads.
    SharedEverythingThreads,
    /// Custom descriptors.
    CustomDescriptors,
    /// Imports from `wasm:js-string`, the JavaScript string builtins.
    JsStringBuiltins,
}

impl Feature {
    /// A short name for reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Simd => "SIMD",
            Self::RelaxedSimd => "relaxed SIMD",
            Self::SharedMemory => "shared memory",
            Self::Atomics => "atomics",
            Self::AcquireReleaseAtomics => "acquire/release atomics",
            Self::BulkMemory => "bulk memory",
            Self::ReferenceTypes => "reference types",
            Self::FunctionReferences => "function references",
            Self::Gc => "GC",
            Self::ExceptionHandling => "exception handling",
            Self::LegacyExceptionHandling => "legacy exception handling",
            Self::TailCall => "tail calls",
            Self::MultiMemory => "multiple memories",
            Self::Memory64 => "64-bit memory",
            Self::MultiValue => "multi-value",
            Self::SignExtension => "sign extension",
            Self::SaturatingFloatToInt => "saturating float-to-int",
            Self::MutableGlobals => "mutable globals",
            Self::ExtendedConst => "extended constants",
            Self::WideArithmetic => "wide arithmetic",
            Self::MemoryControl => "memory control",
            Self::StackSwitching => "stack switching",
            Self::SharedEverythingThreads => "shared-everything threads",
            Self::CustomDescriptors => "custom descriptors",
            Self::JsStringBuiltins => "JS string builtins",
        }
    }
}

/// A feature the binary needs and where the evidence is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FeatureUse {
    /// The feature.
    pub feature: Feature,
    /// Declarations that need it: a shared memory, a tag, a type.
    pub declarations: u64,
    /// Instructions that need it, in function bodies and constant expressions.
    pub instructions: u64,
}

/// The size of one section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    /// Which section.
    pub kind: SectionKind,
    /// The name, for a custom section.
    pub name: Option<String>,
    /// Bytes of the section id and the size field in front of the payload.
    pub header: u64,
    /// Bytes of the payload. For a custom section that includes its name.
    pub payload: u64,
}

/// The kind of a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    /// Custom section (names, debug info, producers, …).
    Custom,
    /// Types.
    Type,
    /// Imports.
    Import,
    /// Function declarations.
    Function,
    /// Tables.
    Table,
    /// Memories.
    Memory,
    /// Globals.
    Global,
    /// Exports.
    Export,
    /// Start function.
    Start,
    /// Element segments.
    Element,
    /// Function bodies.
    Code,
    /// Data segments.
    Data,
    /// Number of data segments.
    DataCount,
    /// Tags.
    Tag,
}

impl SectionKind {
    fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => Self::Custom,
            1 => Self::Type,
            2 => Self::Import,
            3 => Self::Function,
            4 => Self::Table,
            5 => Self::Memory,
            6 => Self::Global,
            7 => Self::Export,
            8 => Self::Start,
            9 => Self::Element,
            10 => Self::Code,
            11 => Self::Data,
            12 => Self::DataCount,
            13 => Self::Tag,
            _ => return None,
        })
    }
}

/// What a custom section is, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomKind {
    /// The `name` section.
    Names,
    /// Debug information: `.debug_*`, `external_debug_info`, `sourceMappingURL`.
    Debug,
    /// The `producers` section.
    Producers,
    /// The `target_features` section.
    TargetFeatures,
    /// Any other custom section; its content is not interpreted.
    Other,
}

impl CustomKind {
    fn of(name: &str) -> Self {
        match name {
            "name" => Self::Names,
            "producers" => Self::Producers,
            "target_features" => Self::TargetFeatures,
            "external_debug_info" | "sourceMappingURL" => Self::Debug,
            _ if name.starts_with(".debug_") => Self::Debug,
            _ => Self::Other,
        }
    }
}

/// A custom section with its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomSection {
    /// The section name.
    pub name: String,
    /// What the name says it is.
    pub kind: CustomKind,
    /// Payload bytes, name included, as in [`Section::payload`].
    pub bytes: u64,
}

/// The state of the `name` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Names {
    /// The module has no `name` section: functions and locals are unnamed.
    Absent,
    /// The `name` section was read.
    Present {
        /// The module name, if given.
        module: Option<String>,
        /// Functions with a name.
        functions: u32,
        /// Functions that name their locals.
        functions_with_local_names: u32,
    },
    /// There is a `name` section and it could not be read.
    Unreadable,
}

/// One entry of the `producers` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Producer {
    /// `language`, `processed-by` or `sdk`.
    pub field: String,
    /// The tool or language.
    pub name: String,
    /// Its version.
    pub version: String,
}

/// How a `target_features` entry applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetFeatureMode {
    /// `+`: the module uses the feature.
    Used,
    /// `-`: the module must not be linked with code that uses it.
    Disallowed,
    /// `=`: the module requires every linked module to use it.
    Required,
}

/// One entry of the `target_features` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetFeature {
    /// How the feature applies.
    pub mode: TargetFeatureMode,
    /// The feature name, for example `simd128`.
    pub name: String,
}

/// What the inventory could not decide.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Coverage {
    /// True if every instruction was classified and every metadata section
    /// read. Even then [`Coverage::not_detectable`] applies.
    pub complete: bool,
    /// Instructions of a proposal this version of peakmark has no feature for,
    /// by proposal. They are not counted as unused.
    pub unclassified_instructions: BTreeMap<String, u64>,
    /// Custom sections that peakmark knows how to read and could not.
    pub unreadable_custom_sections: Vec<String>,
    /// What cannot be derived from a binary. The same for every module.
    pub not_detectable: Vec<Limitation>,
}

/// Something a static inventory cannot tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Limitation {
    /// What the host must provide beyond the imports: cross-origin isolation
    /// for shared memory, embedder APIs, a JavaScript glue's own needs.
    HostEnvironment,
    /// Features are found where they occur in the binary. Whether that code
    /// runs, or runs on a given path, is not known.
    Reachability,
    /// `target_features` and `producers` are claims of the toolchain and are
    /// reported next to the derived features, not merged into them.
    ToolchainClaims,
    /// Custom sections other than names, debug information, producers and
    /// target features are not interpreted.
    CustomSectionSemantics,
    /// Which engines and versions support a feature.
    EngineSupport,
}

impl fmt::Display for Limitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::HostEnvironment => {
                "what the host must provide beyond the imports (for example cross-origin isolation for shared memory)"
            }
            Self::Reachability => "whether code that needs a feature ever runs",
            Self::ToolchainClaims => {
                "target_features and producers are the toolchain's claims and are not used to derive requirements"
            }
            Self::CustomSectionSemantics => {
                "the meaning of custom sections other than names, debug, producers and target_features"
            }
            Self::EngineSupport => "which engines and versions support a feature",
        })
    }
}

const NOT_DETECTABLE: [Limitation; 5] = [
    Limitation::HostEnvironment,
    Limitation::Reachability,
    Limitation::ToolchainClaims,
    Limitation::CustomSectionSemantics,
    Limitation::EngineSupport,
];

/// Reads the inventory of a module that has been validated.
pub(crate) fn read(bytes: &[u8]) -> Parsed<Inventory> {
    let mut reader = Reader::default();
    let mut end = PREAMBLE_BYTES;
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload?;
        if let Some((id, range)) = payload.as_section() {
            let kind = SectionKind::from_id(id).expect("validated module: known section id");
            reader.inventory.sections.push(Section {
                kind,
                name: None,
                header: range.start - end,
                payload: range.end - range.start,
            });
            end = range.end;
        }
        reader.payload(payload)?;
    }
    Ok(reader.finish())
}

#[derive(Default)]
struct Reader {
    inventory: Inventory,
    /// The signature of every type; `None` for structs, arrays and continuations.
    types: Vec<Option<FuncSignature>>,
    /// The type index of every function, imported first.
    functions: Vec<u32>,
    uses: BTreeMap<Feature, FeatureUse>,
    unclassified: BTreeMap<String, u64>,
    unreadable: Vec<String>,
}

type Parsed<T> = std::result::Result<T, BinaryReaderError>;

/// A metadata section that is there and cannot be read. It lowers the coverage
/// and never fails the inventory.
struct Unreadable;

impl From<BinaryReaderError> for Unreadable {
    fn from(_: BinaryReaderError) -> Self {
        Self
    }
}

type Metadata = std::result::Result<(), Unreadable>;

impl Reader {
    fn payload(&mut self, payload: Payload<'_>) -> Parsed<()> {
        match payload {
            Payload::Version { num, .. } => self.inventory.artifact.version = u32::from(num),
            Payload::TypeSection(reader) => {
                for group in reader {
                    self.rec_group(&group?);
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import?;
                    self.import(import.module, import.name, import.ty);
                }
            }
            Payload::FunctionSection(reader) => {
                for ty in reader {
                    self.functions.push(ty?);
                    self.inventory.counts.defined_functions += 1;
                }
            }
            Payload::TableSection(reader) => {
                for table in reader {
                    let table = table?;
                    self.table(None, convert_table(&table.ty));
                    if let TableInit::Expr(expr) = &table.init {
                        self.const_expr(expr)?;
                    }
                }
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    self.memory(None, convert_memory(&memory?));
                }
            }
            Payload::TagSection(reader) => {
                for tag in reader {
                    let signature = self.signature(tag?.func_type_idx);
                    self.tag(None, signature);
                }
            }
            Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global?;
                    self.global(None, convert_global(global.ty));
                    self.const_expr(&global.init_expr)?;
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export?;
                    self.export(export.name, export.kind, export.index);
                }
            }
            Payload::ElementSection(reader) => {
                for element in reader {
                    self.element(element?)?;
                }
            }
            Payload::DataSection(reader) => {
                for data in reader {
                    self.data(data?)?;
                }
            }
            Payload::CodeSectionEntry(body) => {
                for operator in body.get_operators_reader()? {
                    self.operator(&operator?, false);
                }
            }
            Payload::CustomSection(reader) => self.custom(&reader),
            _ => {}
        }
        Ok(())
    }

    fn element(&mut self, element: wasmparser::Element<'_>) -> Parsed<()> {
        self.inventory.counts.element_segments += 1;
        match element.kind {
            ElementKind::Passive => self.declare(Feature::BulkMemory),
            ElementKind::Declared => self.declare(Feature::ReferenceTypes),
            ElementKind::Active {
                table_index,
                offset_expr,
            } => {
                if table_index.is_some() {
                    self.declare(Feature::ReferenceTypes);
                }
                self.const_expr(&offset_expr)?;
            }
        }
        if let ElementItems::Expressions(_, expressions) = element.items {
            self.declare(Feature::ReferenceTypes);
            for expr in expressions {
                self.const_expr(&expr?)?;
            }
        }
        Ok(())
    }

    fn data(&mut self, data: wasmparser::Data<'_>) -> Parsed<()> {
        self.inventory.counts.data_segments += 1;
        match data.kind {
            DataKind::Passive => self.declare(Feature::BulkMemory),
            DataKind::Active {
                memory_index,
                offset_expr,
            } => {
                if memory_index != 0 {
                    self.declare(Feature::MultiMemory);
                }
                self.const_expr(&offset_expr)?;
            }
        }
        Ok(())
    }

    fn rec_group(&mut self, group: &RecGroup) {
        if group.is_explicit_rec_group() {
            self.declare(Feature::Gc);
        }
        for sub in group.types() {
            self.inventory.counts.types += 1;
            if !sub.is_final || !sub.supertype_idxs.is_empty() {
                self.declare(Feature::Gc);
            }
            let signature = match &sub.composite_type.inner {
                wasmparser::CompositeInnerType::Func(func) => {
                    for ty in func.params().iter().chain(func.results()) {
                        self.value_type(*ty);
                    }
                    if func.results().len() > 1 {
                        self.declare(Feature::MultiValue);
                    }
                    Some(FuncSignature {
                        params: func.params().iter().map(|ty| (*ty).into()).collect(),
                        results: func.results().iter().map(|ty| (*ty).into()).collect(),
                    })
                }
                wasmparser::CompositeInnerType::Array(array) => {
                    self.declare(Feature::Gc);
                    self.storage_type(array.0.element_type);
                    None
                }
                wasmparser::CompositeInnerType::Struct(structure) => {
                    self.declare(Feature::Gc);
                    for field in &structure.fields {
                        self.storage_type(field.element_type);
                    }
                    None
                }
                wasmparser::CompositeInnerType::Cont(_) => {
                    self.declare(Feature::StackSwitching);
                    None
                }
            };
            self.types.push(signature);
        }
    }

    fn storage_type(&mut self, ty: StorageType) {
        if let StorageType::Val(ty) = ty {
            self.value_type(ty);
        }
    }

    /// The feature a value type needs, by what it is.
    fn value_type(&mut self, ty: ValType) {
        match ty {
            ValType::V128 => self.declare(Feature::Simd),
            ValType::Ref(reference) => self.reference_type(reference),
            _ => {}
        }
    }

    fn reference_type(&mut self, reference: RefType) {
        use wasmparser::AbstractHeapType as Heap;
        match reference.heap_type() {
            HeapType::Abstract { ty, .. } => match ty {
                Heap::Func | Heap::NoFunc | Heap::Extern | Heap::NoExtern => {
                    self.declare(Feature::ReferenceTypes);
                }
                Heap::Exn | Heap::NoExn => self.declare(Feature::ExceptionHandling),
                Heap::Cont | Heap::NoCont => self.declare(Feature::StackSwitching),
                _ => self.declare(Feature::Gc),
            },
            HeapType::Concrete(_) | HeapType::Exact(_) => self.declare(Feature::FunctionReferences),
        }
        if !reference.is_nullable() {
            self.declare(Feature::FunctionReferences);
        }
    }

    fn signature(&self, type_index: u32) -> FuncSignature {
        self.types[type_index as usize]
            .clone()
            .expect("validated module: function type")
    }

    fn import(&mut self, module: &str, name: &str, ty: TypeRef) {
        self.inventory.counts.imports += 1;
        if module == "wasm:js-string" {
            self.declare(Feature::JsStringBuiltins);
        }
        let origin = Some(ImportName {
            module: module.to_string(),
            name: name.to_string(),
        });
        let ty = match ty {
            TypeRef::Func(index) | TypeRef::FuncExact(index) => {
                self.functions.push(index);
                self.inventory.counts.imported_functions += 1;
                ExternType::Function(self.signature(index))
            }
            TypeRef::Table(ty) => {
                let ty = convert_table(&ty);
                self.table(origin, ty.clone());
                ExternType::Table(ty)
            }
            TypeRef::Memory(ty) => {
                let ty = convert_memory(&ty);
                self.memory(origin, ty);
                ExternType::Memory(ty)
            }
            TypeRef::Global(ty) => {
                let ty = convert_global(ty);
                self.global(origin, ty.clone());
                ExternType::Global(ty)
            }
            TypeRef::Tag(tag) => {
                let signature = self.signature(tag.func_type_idx);
                self.tag(origin, signature.clone());
                ExternType::Tag(signature)
            }
        };
        self.inventory.imports.push(Import {
            module: module.to_string(),
            name: name.to_string(),
            ty,
        });
    }

    fn table(&mut self, imported_from: Option<ImportName>, ty: TableType) {
        let index = self.inventory.tables.len();
        if ty.table64 {
            self.declare(Feature::Memory64);
        }
        if index > 0 || ty.element != ValueType::Ref("funcref".to_string()) {
            self.declare(Feature::ReferenceTypes);
        }
        self.inventory.tables.push(Table {
            index: index_of(index),
            imported_from,
            exported_as: Vec::new(),
            ty,
        });
    }

    fn memory(&mut self, imported_from: Option<ImportName>, ty: MemoryType) {
        let index = self.inventory.memories.len();
        if ty.shared {
            self.declare(Feature::SharedMemory);
        }
        if ty.memory64 {
            self.declare(Feature::Memory64);
        }
        if index == 1 {
            self.declare(Feature::MultiMemory);
        }
        self.inventory.memories.push(Memory {
            index: index_of(index),
            imported_from,
            exported_as: Vec::new(),
            ty,
        });
    }

    fn global(&mut self, imported_from: Option<ImportName>, ty: GlobalType) {
        let index = self.inventory.globals.len();
        self.inventory.globals.push(Global {
            index: index_of(index),
            imported_from,
            exported_as: Vec::new(),
            ty,
        });
    }

    fn tag(&mut self, imported_from: Option<ImportName>, ty: FuncSignature) {
        let index = self.inventory.tags.len();
        self.declare(Feature::ExceptionHandling);
        self.inventory.tags.push(Tag {
            index: index_of(index),
            imported_from,
            exported_as: Vec::new(),
            ty,
        });
    }

    fn export(&mut self, name: &str, kind: ExternalKind, index: u32) {
        self.inventory.counts.exports += 1;
        let slot = index as usize;
        let name = name.to_string();
        let ty = match kind {
            ExternalKind::Func | ExternalKind::FuncExact => {
                ExternType::Function(self.signature(self.functions[slot]))
            }
            ExternalKind::Table => {
                let table = &mut self.inventory.tables[slot];
                table.exported_as.push(name.clone());
                ExternType::Table(table.ty.clone())
            }
            ExternalKind::Memory => {
                let memory = &mut self.inventory.memories[slot];
                memory.exported_as.push(name.clone());
                ExternType::Memory(memory.ty)
            }
            ExternalKind::Global => {
                let global = &mut self.inventory.globals[slot];
                global.exported_as.push(name.clone());
                if global.ty.mutable {
                    self.declare(Feature::MutableGlobals);
                }
                ExternType::Global(self.inventory.globals[slot].ty.clone())
            }
            ExternalKind::Tag => {
                let tag = &mut self.inventory.tags[slot];
                tag.exported_as.push(name.clone());
                ExternType::Tag(tag.ty.clone())
            }
        };
        self.inventory.exports.push(Export { name, index, ty });
    }

    fn const_expr(&mut self, expr: &ConstExpr<'_>) -> Parsed<()> {
        for operator in expr.get_operators_reader() {
            self.operator(&operator?, true);
        }
        Ok(())
    }

    fn operator(&mut self, operator: &Operator<'_>, constant: bool) {
        match proposal(operator) {
            "mvp" => {}
            "sign_extension" => self.instruct(Feature::SignExtension),
            "saturating_float_to_int" => self.instruct(Feature::SaturatingFloatToInt),
            "bulk_memory" => self.instruct(Feature::BulkMemory),
            "reference_types" => self.instruct(Feature::ReferenceTypes),
            "simd" => self.instruct(Feature::Simd),
            "relaxed_simd" => self.instruct(Feature::RelaxedSimd),
            "threads" => self.instruct(Feature::Atomics),
            "acquire_release_atomics" => self.instruct(Feature::AcquireReleaseAtomics),
            "tail_call" => self.instruct(Feature::TailCall),
            "exceptions" => self.instruct(Feature::ExceptionHandling),
            "legacy_exceptions" => self.instruct(Feature::LegacyExceptionHandling),
            "function_references" => self.instruct(Feature::FunctionReferences),
            "gc" => self.instruct(Feature::Gc),
            "memory_control" => self.instruct(Feature::MemoryControl),
            "wide_arithmetic" => self.instruct(Feature::WideArithmetic),
            "stack_switching" => self.instruct(Feature::StackSwitching),
            "shared_everything_threads" => self.instruct(Feature::SharedEverythingThreads),
            "custom_descriptors" => self.instruct(Feature::CustomDescriptors),
            other => *self.unclassified.entry(other.to_string()).or_default() += 1,
        }
        match operator {
            Operator::Block { blockty } | Operator::Loop { blockty } | Operator::If { blockty }
                if matches!(blockty, BlockType::FuncType(_)) =>
            {
                self.instruct(Feature::MultiValue);
            }
            Operator::I32Add
            | Operator::I32Sub
            | Operator::I32Mul
            | Operator::I64Add
            | Operator::I64Sub
            | Operator::I64Mul
                if constant =>
            {
                self.instruct(Feature::ExtendedConst);
            }
            _ => {}
        }
    }

    fn declare(&mut self, feature: Feature) {
        self.used(feature).declarations += 1;
    }

    fn instruct(&mut self, feature: Feature) {
        self.used(feature).instructions += 1;
    }

    fn used(&mut self, feature: Feature) -> &mut FeatureUse {
        self.uses.entry(feature).or_insert(FeatureUse {
            feature,
            declarations: 0,
            instructions: 0,
        })
    }

    fn custom(&mut self, reader: &wasmparser::CustomSectionReader<'_>) {
        let name = reader.name();
        let kind = CustomKind::of(name);
        let range = reader.range();
        let bytes = range.end - range.start;
        self.inventory
            .sections
            .last_mut()
            .expect("a custom section was just recorded")
            .name = Some(name.to_string());
        self.inventory.custom_sections.push(CustomSection {
            name: name.to_string(),
            kind,
            bytes,
        });
        let already_read = match kind {
            CustomKind::Names => !matches!(self.inventory.names, Names::Absent),
            CustomKind::Producers => !self.inventory.producers.is_empty(),
            CustomKind::TargetFeatures => !self.inventory.target_features.is_empty(),
            _ => true,
        };
        if already_read {
            return;
        }
        let read = match kind {
            CustomKind::Names => self.names(reader),
            CustomKind::Producers => self.producers(reader),
            CustomKind::TargetFeatures => self.target_features(reader),
            CustomKind::Debug | CustomKind::Other => Ok(()),
        };
        if read.is_err() {
            if kind == CustomKind::Names {
                self.inventory.names = Names::Unreadable;
            }
            self.unreadable.push(name.to_string());
        }
    }

    fn names(&mut self, reader: &wasmparser::CustomSectionReader<'_>) -> Metadata {
        let KnownCustom::Name(names) = reader.as_known() else {
            return Ok(());
        };
        let (mut module, mut functions, mut with_locals) = (None, 0, 0);
        for subsection in names {
            match subsection? {
                Name::Module { name, .. } => module = Some(name.to_string()),
                Name::Function(map) => {
                    for naming in map {
                        naming?;
                        functions += 1;
                    }
                }
                Name::Local(map) => {
                    for naming in map {
                        naming?;
                        with_locals += 1;
                    }
                }
                _ => {}
            }
        }
        self.inventory.names = Names::Present {
            module,
            functions,
            functions_with_local_names: with_locals,
        };
        Ok(())
    }

    fn producers(&mut self, reader: &wasmparser::CustomSectionReader<'_>) -> Metadata {
        let KnownCustom::Producers(producers) = reader.as_known() else {
            return Ok(());
        };
        let mut entries = Vec::new();
        for field in producers {
            let field = field?;
            for value in field.values {
                let value = value?;
                entries.push(Producer {
                    field: field.name.to_string(),
                    name: value.name.to_string(),
                    version: value.version.to_string(),
                });
            }
        }
        self.inventory.producers = entries;
        Ok(())
    }

    fn target_features(&mut self, reader: &wasmparser::CustomSectionReader<'_>) -> Metadata {
        let mut data = BinaryReader::new(reader.data(), reader.data_offset());
        let mut entries = Vec::new();
        for _ in 0..data.read_var_u32()? {
            let prefix = data.read_u8()?;
            let mode = match prefix {
                b'+' => TargetFeatureMode::Used,
                b'-' => TargetFeatureMode::Disallowed,
                b'=' => TargetFeatureMode::Required,
                _ => return Err(Unreadable),
            };
            entries.push(TargetFeature {
                mode,
                name: data.read_string()?.to_string(),
            });
        }
        self.inventory.target_features = entries;
        Ok(())
    }

    fn finish(mut self) -> Inventory {
        let mut inventory = std::mem::take(&mut self.inventory);
        inventory.features = self.uses.into_values().collect();
        inventory.coverage = Coverage {
            complete: self.unclassified.is_empty() && self.unreadable.is_empty(),
            unclassified_instructions: self.unclassified,
            unreadable_custom_sections: self.unreadable,
            not_detectable: NOT_DETECTABLE.to_vec(),
        };
        inventory
    }
}

fn index_of(position: usize) -> u32 {
    u32::try_from(position).expect("a module has fewer than 2^32 entities")
}

fn convert_memory(memory: &wasmparser::MemoryType) -> MemoryType {
    MemoryType {
        minimum: memory.initial,
        maximum: memory.maximum,
        shared: memory.shared,
        memory64: memory.memory64,
        page_size: memory.page_size_log2.map_or(65_536, |log2| 1u64 << log2),
    }
}

fn convert_table(table: &wasmparser::TableType) -> TableType {
    TableType {
        element: table.element_type.into(),
        minimum: table.initial,
        maximum: table.maximum,
        table64: table.table64,
        shared: table.shared,
    }
}

fn convert_global(global: wasmparser::GlobalType) -> GlobalType {
    GlobalType {
        value: global.content_type.into(),
        mutable: global.mutable,
        shared: global.shared,
    }
}

/// The proposal an operator belongs to, as wasmparser names it.
fn proposal(operator: &Operator<'_>) -> &'static str {
    macro_rules! proposal_of {
        ($( @$proposal:ident $op:ident $({ $($arg:ident: $argty:ty),* })? => $visit:ident ($($ann:tt)*))*) => {
            match operator {
                $( Operator::$op { .. } => stringify!($proposal), )*
                _ => "unknown",
            }
        };
    }
    wasmparser::for_each_operator!(proposal_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory(wat: &str) -> Inventory {
        Inventory::from_bytes(&wat::parse_str(wat).unwrap()).unwrap()
    }

    fn uses(inventory: &Inventory, feature: Feature) -> (u64, u64) {
        inventory
            .feature(feature)
            .map_or((0, 0), |used| (used.declarations, used.instructions))
    }

    #[test]
    fn section_bytes_add_up_to_the_file() {
        let bytes = wat::parse_str(
            r#"(module (memory 1) (func (export "f") (result i32) i32.const 1)
                (data (i32.const 0) "abc") (@custom "extra" "payload"))"#,
        )
        .unwrap();
        let inventory = Inventory::from_bytes(&bytes).unwrap();
        assert_eq!(inventory.accounted_bytes(), bytes.len() as u64);
        let custom = inventory
            .sections
            .iter()
            .find(|section| section.kind == SectionKind::Custom)
            .unwrap();
        assert_eq!(custom.name.as_deref(), Some("extra"));
        assert!(custom.header >= 2);
    }

    #[test]
    fn imports_and_exports_carry_their_types() {
        let inventory = inventory(
            r#"(module
                (import "env" "log" (func (param i32 f64) (result i64)))
                (import "env" "mem" (memory 1 4))
                (func (export "add") (param i32 i32) (result i32) local.get 0)
                (global (export "g") (mut i32) (i32.const 0)))"#,
        );
        assert_eq!(inventory.counts.imported_functions, 1);
        assert_eq!(inventory.counts.defined_functions, 1);
        assert_eq!(
            inventory.imports[0].ty.clone(),
            ExternType::Function(FuncSignature {
                params: vec![ValueType::I32, ValueType::F64],
                results: vec![ValueType::I64],
            })
        );
        let ExternType::Function(add) = &inventory.exports[0].ty else {
            panic!("add is a function");
        };
        assert_eq!(add.to_string(), "(i32, i32) -> i32");
        assert!(matches!(&inventory.exports[1].ty, ExternType::Global(global) if global.mutable));
        assert_eq!(uses(&inventory, Feature::MutableGlobals), (1, 0));
    }

    #[test]
    fn memories_report_limits_sharing_and_origin() {
        let inventory = inventory(
            r#"(module
                (import "env" "imported" (memory 2))
                (memory (export "bounded") 1 8)
                (memory 3 3 shared)
                (memory i64 1))"#,
        );
        let [imported, bounded, shared, wide] = inventory.memories.as_slice() else {
            panic!("four memories");
        };
        assert!(imported.imported_from.is_some());
        assert_eq!(imported.ty.maximum, None);
        assert_eq!(bounded.exported_as, ["bounded"]);
        assert_eq!(bounded.ty.maximum, Some(8));
        assert_eq!(bounded.ty.maximum_bytes(), Some(8 * 65_536));
        assert!(shared.ty.shared && !shared.ty.memory64);
        assert!(wide.ty.memory64);
        assert_eq!(wide.ty.page_size, 65_536);
        assert_eq!(uses(&inventory, Feature::SharedMemory), (1, 0));
        assert_eq!(uses(&inventory, Feature::Memory64), (1, 0));
        assert_eq!(uses(&inventory, Feature::MultiMemory), (1, 0));
    }

    #[test]
    fn tables_and_tags_are_listed() {
        let inventory = inventory(
            r#"(module
                (type (func (param i32)))
                (table (export "t") 2 10 funcref)
                (table 1 externref)
                (tag (type 0)))"#,
        );
        assert_eq!(inventory.tables[0].ty.maximum, Some(10));
        assert_eq!(inventory.tables[0].exported_as, ["t"]);
        assert_eq!(
            inventory.tables[1].ty.element,
            ValueType::Ref("externref".to_string())
        );
        assert_eq!(inventory.tags[0].ty.params, [ValueType::I32]);
        assert!(inventory.feature(Feature::ExceptionHandling).is_some());
        assert!(inventory.feature(Feature::ReferenceTypes).is_some());
    }

    #[test]
    fn features_come_from_instructions() {
        let inventory = inventory(
            r"(module (memory 1 1 shared)
                (func (param i32) (result i32)
                  i32.const 0 i32.const 1 i32.const 2 memory.copy
                  i32.const 0 i32.const 1 memory.atomic.notify drop
                  v128.const i64x2 0 0 drop
                  local.get 0 i32.extend8_s
                  return_call 0))",
        );
        assert_eq!(uses(&inventory, Feature::BulkMemory), (0, 1));
        assert_eq!(uses(&inventory, Feature::Atomics), (0, 1));
        assert_eq!(uses(&inventory, Feature::Simd), (0, 1));
        assert_eq!(uses(&inventory, Feature::SignExtension), (0, 1));
        assert_eq!(uses(&inventory, Feature::TailCall), (0, 1));
        assert!(inventory.feature(Feature::Gc).is_none());
        assert!(inventory.coverage.complete);
    }

    #[test]
    fn features_come_from_declarations() {
        let inventory = inventory(
            r#"(module
                (type (struct (field i32)))
                (func (param v128) (result i32 i32) unreachable)
                (data "passive"))"#,
        );
        assert_eq!(uses(&inventory, Feature::Gc), (1, 0));
        assert_eq!(uses(&inventory, Feature::Simd), (1, 0));
        assert_eq!(uses(&inventory, Feature::MultiValue), (1, 0));
        assert_eq!(uses(&inventory, Feature::BulkMemory), (1, 0));
    }

    #[test]
    fn exception_instructions_are_told_apart() {
        let inventory = inventory(
            r"(module (tag $t)
                (func try_table (catch_all 0) end))",
        );
        assert_eq!(uses(&inventory, Feature::ExceptionHandling), (1, 1));
        assert!(
            inventory
                .feature(Feature::LegacyExceptionHandling)
                .is_none()
        );
    }

    #[test]
    fn arithmetic_in_a_constant_is_extended_const() {
        let inventory = inventory("(module (global i32 (i32.add (i32.const 1) (i32.const 2))))");
        assert_eq!(uses(&inventory, Feature::ExtendedConst), (0, 1));
    }

    #[test]
    fn a_plain_module_needs_no_feature() {
        let inventory =
            inventory(r#"(module (memory 1) (func (export "f") (result i32) i32.const 1))"#);
        assert_eq!(inventory.features.len(), 0);
        assert!(inventory.coverage.complete);
        assert_eq!(
            inventory.coverage.not_detectable.len(),
            NOT_DETECTABLE.len()
        );
    }

    #[test]
    fn a_named_and_a_stripped_module_differ_in_names() {
        let named = inventory(r"(module $m (func $f (local $x i32)))");
        assert_eq!(
            named.names,
            Names::Present {
                module: Some("m".to_string()),
                functions: 1,
                functions_with_local_names: 1,
            }
        );
        assert_eq!(inventory("(module (func))").names, Names::Absent);
        assert!(
            named
                .custom_sections
                .iter()
                .any(|section| section.kind == CustomKind::Names && section.bytes > 0)
        );
    }

    #[test]
    fn custom_sections_producers_and_target_features_are_read() {
        let inventory = inventory(
            r#"(module
                (@custom "producers" "\02\08language\01\04Rust\050.0.0\0cprocessed-by\01\05rustc\061.80.0")
                (@custom "target_features" "\02\2b\07simd128\2d\07atomics")
                (@custom ".debug_info" "xx")
                (@custom "mystery" "yy"))"#,
        );
        let kinds: Vec<_> = inventory
            .custom_sections
            .iter()
            .map(|section| (section.name.as_str(), section.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("producers", CustomKind::Producers),
                ("target_features", CustomKind::TargetFeatures),
                (".debug_info", CustomKind::Debug),
                ("mystery", CustomKind::Other),
            ]
        );
        assert_eq!(inventory.producers[0].name, "Rust");
        assert_eq!(inventory.producers[1].field, "processed-by");
        assert_eq!(inventory.target_features[0].mode, TargetFeatureMode::Used);
        assert_eq!(inventory.target_features[1].name, "atomics");
        assert!(inventory.feature(Feature::Atomics).is_none());
        assert!(inventory.feature(Feature::Simd).is_none());
    }

    #[test]
    fn an_unreadable_metadata_section_lowers_the_coverage() {
        let inventory = inventory(r#"(module (@custom "target_features" "\01\3f\01x"))"#);
        assert!(!inventory.coverage.complete);
        assert_eq!(
            inventory.coverage.unreadable_custom_sections,
            ["target_features"]
        );
        assert_eq!(inventory.target_features.len(), 0);
    }

    #[test]
    fn imports_of_the_js_string_builtins_are_a_feature() {
        let inventory = inventory(
            r#"(module (import "wasm:js-string" "length" (func (param externref) (result i32))))"#,
        );
        assert_eq!(uses(&inventory, Feature::JsStringBuiltins), (1, 0));
    }

    #[test]
    fn the_serialized_form_is_versioned_and_stable() {
        let inventory = inventory(r#"(module (memory (export "m") 1 2))"#);
        let first = serde_json::to_string(&inventory).unwrap();
        assert_eq!(first, serde_json::to_string(&inventory).unwrap());
        let document: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(document["schema_version"], SCHEMA_VERSION);
        assert_eq!(document["artifact"]["kind"], "core_module");
        assert_eq!(document["exports"][0]["type"]["kind"], "memory");
        assert_eq!(document["exports"][0]["type"]["maximum"], 2);
        assert_eq!(document["names"]["status"], "absent");
    }

    #[test]
    fn a_component_has_no_inventory() {
        let bytes = wat::parse_str("(component)").unwrap();
        assert!(matches!(
            Inventory::from_bytes(&bytes),
            Err(InspectError::Component)
        ));
    }
}
