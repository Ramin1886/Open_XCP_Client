// SPDX-License-Identifier: GPL-2.0-only
//! ASAM MCD-2 MC (A2L) parser, complex C/C++ structure resolver, and conversion methods
//! tailored for Vector `xcplite` 2.2.2 targets.

pub mod compu_method;
pub mod lexer;
pub mod model;
pub mod parser;
pub mod struct_resolver;

pub use compu_method::{apply_physical_to_raw, apply_raw_to_physical};
pub use model::{
    A2lDataType, A2lEvent, A2lModel, Characteristic, CharacteristicType, CompuMethod, CompuType,
    Instance, Measurement, StructTreeNode, StructureComponent, TypedefCharacteristic,
    TypedefMeasurement, TypedefStructure,
};
pub use parser::{parse_a2l, A2lParseError};
pub use struct_resolver::{build_measurement_hierarchy, resolve_a2l_structures};
