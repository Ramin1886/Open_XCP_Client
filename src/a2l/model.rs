// SPDX-License-Identifier: GPL-2.0-only
//! Core data model for parsed A2L files, including measurements, characteristics,
//! XCP events, conversion methods, and hierarchical C/C++ structure definitions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Fundamental ASAM A2L data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum A2lDataType {
    UByte,
    SByte,
    UWord,
    SWord,
    ULong,
    SLong,
    AUint64,
    AInt64,
    Float32Ieee,
    Float64Ieee,
}

impl A2lDataType {
    /// Parses an A2L keyword into an [`A2lDataType`].
    pub fn from_keyword(token: &str) -> Option<Self> {
        match token.trim() {
            "UBYTE" | "uint8" => Some(Self::UByte),
            "SBYTE" | "int8" => Some(Self::SByte),
            "UWORD" | "uint16" => Some(Self::UWord),
            "SWORD" | "int16" => Some(Self::SWord),
            "ULONG" | "uint32" => Some(Self::ULong),
            "SLONG" | "int32" => Some(Self::SLong),
            "A_UINT64" | "uint64" => Some(Self::AUint64),
            "A_INT64" | "int64" => Some(Self::AInt64),
            "FLOAT32_IEEE" | "float" | "f32" => Some(Self::Float32Ieee),
            "FLOAT64_IEEE" | "double" | "f64" => Some(Self::Float64Ieee),
            _ => None,
        }
    }

    /// Size of a single scalar element in bytes.
    pub fn byte_size(self) -> usize {
        match self {
            Self::UByte | Self::SByte => 1,
            Self::UWord | Self::SWord => 2,
            Self::ULong | Self::SLong | Self::Float32Ieee => 4,
            Self::AUint64 | Self::AInt64 | Self::Float64Ieee => 8,
        }
    }

    /// Canonical ASAM name string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UByte => "UBYTE",
            Self::SByte => "SBYTE",
            Self::UWord => "UWORD",
            Self::SWord => "SWORD",
            Self::ULong => "ULONG",
            Self::SLong => "SLONG",
            Self::AUint64 => "A_UINT64",
            Self::AInt64 => "A_INT64",
            Self::Float32Ieee => "FLOAT32_IEEE",
            Self::Float64Ieee => "FLOAT64_IEEE",
        }
    }

    /// Returns whether this type is an integer type.
    pub fn is_integer(self) -> bool {
        !matches!(self, Self::Float32Ieee | Self::Float64Ieee)
    }

    /// Returns the raw numeric bounds `(min, max)` representable by this scalar type.
    pub fn raw_bounds(self) -> (f64, f64) {
        match self {
            Self::UByte => (0.0, u8::MAX as f64),
            Self::SByte => (i8::MIN as f64, i8::MAX as f64),
            Self::UWord => (0.0, u16::MAX as f64),
            Self::SWord => (i16::MIN as f64, i16::MAX as f64),
            Self::ULong => (0.0, u32::MAX as f64),
            Self::SLong => (i32::MIN as f64, i32::MAX as f64),
            Self::AUint64 => (0.0, u64::MAX as f64),
            Self::AInt64 => (i64::MIN as f64, i64::MAX as f64),
            Self::Float32Ieee => (f32::MIN as f64, f32::MAX as f64),
            Self::Float64Ieee => (f64::MIN, f64::MAX),
        }
    }

    /// Decodes a little-endian raw byte slice into an `f64` raw value.
    pub fn decode_raw_le(self, bytes: &[u8]) -> Option<f64> {
        if bytes.len() < self.byte_size() {
            return None;
        }
        Some(match self {
            Self::UByte => bytes[0] as f64,
            Self::SByte => (bytes[0] as i8) as f64,
            Self::UWord => u16::from_le_bytes([bytes[0], bytes[1]]) as f64,
            Self::SWord => i16::from_le_bytes([bytes[0], bytes[1]]) as f64,
            Self::ULong => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
            Self::SLong => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
            Self::AUint64 => u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]) as f64,
            Self::AInt64 => i64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]) as f64,
            Self::Float32Ieee => {
                f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64
            }
            Self::Float64Ieee => f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
        })
    }

    /// Encodes a raw numeric value as little-endian bytes for this scalar type.
    pub fn encode_raw_le(self, raw_val: f64) -> Vec<u8> {
        match self {
            Self::UByte => vec![raw_val.round().clamp(0.0, u8::MAX as f64) as u8],
            Self::SByte => {
                vec![(raw_val.round().clamp(i8::MIN as f64, i8::MAX as f64) as i8) as u8]
            }
            Self::UWord => (raw_val.round().clamp(0.0, u16::MAX as f64) as u16)
                .to_le_bytes()
                .to_vec(),
            Self::SWord => (raw_val.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16)
                .to_le_bytes()
                .to_vec(),
            Self::ULong => (raw_val.round().clamp(0.0, u32::MAX as f64) as u32)
                .to_le_bytes()
                .to_vec(),
            Self::SLong => (raw_val.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
                .to_le_bytes()
                .to_vec(),
            Self::AUint64 => (raw_val.round().clamp(0.0, u64::MAX as f64) as u64)
                .to_le_bytes()
                .to_vec(),
            Self::AInt64 => (raw_val.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64)
                .to_le_bytes()
                .to_vec(),
            Self::Float32Ieee => (raw_val as f32).to_le_bytes().to_vec(),
            Self::Float64Ieee => raw_val.to_le_bytes().to_vec(),
        }
    }
}

/// XCP DAQ Event channel definition from `IF_DATA XCP`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct A2lEvent {
    pub channel_id: u16,
    pub name: String,
    pub short_name: String,
    /// Cycle period in microseconds (`0` for asynchronous/event-driven).
    pub cycle_time_us: u64,
}

impl A2lEvent {
    /// Returns the human-readable refresh rate in milliseconds (or `0.0` if async).
    pub fn refresh_rate_ms(&self) -> f64 {
        self.cycle_time_us as f64 / 1_000.0
    }
}

/// Conversion formula type in `COMPU_METHOD`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CompuType {
    Identical,
    /// `phys = a * raw + b`
    Linear {
        a: f64,
        b: f64,
    },
    /// `f(x) = (a*x^2 + b*x + c) / (d*x^2 + e*x + f)` where `x = phys` (ASAM standard)
    RatFunc {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f64,
    },
    /// Verbal look-up table (`COMPU_VTAB`) mapping integer raw keys to strings.
    TabVerb {
        table_name: String,
    },
}

/// A2L `COMPU_METHOD` definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompuMethod {
    pub name: String,
    pub description: String,
    pub unit: String,
    pub compu_type: CompuType,
}

/// Resolved or direct A2L `MEASUREMENT` definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub name: String,
    pub description: String,
    pub data_type: A2lDataType,
    pub compu_method: String,
    pub lower_limit: f64,
    pub upper_limit: f64,
    pub address: u32,
    pub address_extension: u8,
    /// Multi-dimensional array dimensions (`[1]` or empty for scalar, `[N]` for 1D, `[R, C]` for 2D).
    pub matrix_dim: Vec<usize>,
    pub unit: String,
    /// Assigned XCP DAQ Event channel ID from `IF_DATA XCP`.
    pub event_id: Option<u16>,
}

/// Upper bound on the number of scalar elements one measurement or characteristic may declare.
/// `MATRIX_DIM` values come from an A2L that may have been uploaded from the target; multiplying
/// them unchecked could overflow or request gigabytes of staging memory.
pub const MAX_ELEMENTS_PER_SIGNAL: usize = 1 << 20;

/// Product of `dims` saturated at [`MAX_ELEMENTS_PER_SIGNAL`] (and at least 1).
pub fn saturating_element_product(dims: &[usize]) -> usize {
    dims.iter()
        .try_fold(1usize, |acc, &d| acc.checked_mul(d))
        .unwrap_or(usize::MAX)
        .clamp(1, MAX_ELEMENTS_PER_SIGNAL)
}

impl Measurement {
    /// Total number of scalar elements in this measurement (`1` for scalar, product of dims for arrays).
    pub fn element_count(&self) -> usize {
        if self.matrix_dim.is_empty() {
            1
        } else {
            saturating_element_product(&self.matrix_dim)
        }
    }

    /// Total footprint in target memory in bytes.
    pub fn total_byte_size(&self) -> usize {
        self.element_count() * self.data_type.byte_size()
    }
}

/// Category of an A2L `CHARACTERISTIC` calibration parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CharacteristicType {
    Value,
    ValBlk,
    Curve,
    Map,
    Ascii,
}

impl CharacteristicType {
    pub fn from_keyword(token: &str) -> Option<Self> {
        match token.trim() {
            "VALUE" => Some(Self::Value),
            "VAL_BLK" => Some(Self::ValBlk),
            "CURVE" => Some(Self::Curve),
            "MAP" => Some(Self::Map),
            "ASCII" => Some(Self::Ascii),
            _ => None,
        }
    }
}

/// Resolved or direct A2L `CHARACTERISTIC` calibration parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Characteristic {
    pub name: String,
    pub description: String,
    pub char_type: CharacteristicType,
    pub address: u32,
    pub address_extension: u8,
    pub record_layout: String,
    pub data_type: A2lDataType,
    pub compu_method: String,
    pub lower_limit: f64,
    pub upper_limit: f64,
    pub matrix_dim: Vec<usize>,
    pub unit: String,
}

impl Characteristic {
    /// Total number of scalar elements (`1` for scalar `VALUE`, product of dims for `VAL_BLK`/`CURVE`/`MAP`).
    pub fn element_count(&self) -> usize {
        if self.matrix_dim.is_empty() {
            1
        } else {
            saturating_element_product(&self.matrix_dim)
        }
    }

    /// Total byte size in target memory.
    pub fn total_byte_size(&self) -> usize {
        self.element_count() * self.data_type.byte_size()
    }
}

/// `TYPEDEF_MEASUREMENT` template used inside `TYPEDEF_STRUCTURE`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedefMeasurement {
    pub name: String,
    pub description: String,
    pub data_type: A2lDataType,
    pub compu_method: String,
    pub lower_limit: f64,
    pub upper_limit: f64,
    pub matrix_dim: Vec<usize>,
    pub unit: String,
}

/// `TYPEDEF_CHARACTERISTIC` template used inside `TYPEDEF_STRUCTURE`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedefCharacteristic {
    pub name: String,
    pub description: String,
    pub char_type: CharacteristicType,
    pub record_layout: String,
    pub data_type: A2lDataType,
    pub compu_method: String,
    pub lower_limit: f64,
    pub upper_limit: f64,
    pub matrix_dim: Vec<usize>,
    pub unit: String,
}

/// A single member (`STRUCTURE_COMPONENT`) inside a `TYPEDEF_STRUCTURE`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructureComponent {
    pub name: String,
    pub type_ref: String,
    pub address_offset: u32,
    pub matrix_dim: Vec<usize>,
}

/// A2L v1.7 `TYPEDEF_STRUCTURE` representing a C/C++ struct or class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedefStructure {
    pub name: String,
    pub description: String,
    pub byte_size: u32,
    pub components: Vec<StructureComponent>,
}

/// A2L v1.7 `INSTANCE` instantiating a `TYPEDEF_STRUCTURE` at a target memory address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub description: String,
    pub type_ref: String,
    pub address: u32,
    pub address_extension: u8,
    pub matrix_dim: Vec<usize>,
    pub event_id: Option<u16>,
}

/// Hierarchical node representing nested C/C++ structures and leaf measurements.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StructTreeNode {
    pub name: String,
    pub full_path: String,
    pub children: BTreeMap<String, StructTreeNode>,
    /// Index into `A2lModel::measurements` if this node is a leaf measurement variable.
    pub measurement_index: Option<usize>,
}

/// Complete parsed and resolved A2L model.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct A2lModel {
    pub project_name: String,
    pub module_name: String,
    pub events: BTreeMap<u16, A2lEvent>,
    pub compu_methods: BTreeMap<String, CompuMethod>,
    pub compu_vtabs: BTreeMap<String, BTreeMap<i64, String>>,
    pub record_layouts: BTreeMap<String, A2lDataType>,
    pub typedef_measurements: BTreeMap<String, TypedefMeasurement>,
    pub typedef_characteristics: BTreeMap<String, TypedefCharacteristic>,
    pub typedef_structures: BTreeMap<String, TypedefStructure>,
    pub instances: Vec<Instance>,
    pub measurements: Vec<Measurement>,
    pub characteristics: Vec<Characteristic>,
    pub hierarchy_root: StructTreeNode,
}

impl A2lModel {
    /// Looks up the verbal enum table (`COMPU_VTAB`) for a given `compu_method` name, if any.
    pub fn enum_table_for(&self, compu_method_name: &str) -> Option<&BTreeMap<i64, String>> {
        let cm = self.compu_methods.get(compu_method_name)?;
        if let CompuType::TabVerb { table_name } = &cm.compu_type {
            self.compu_vtabs.get(table_name)
        } else {
            None
        }
    }

    /// Converts a raw value to physical value using the named `COMPU_METHOD`.
    pub fn raw_to_physical(&self, compu_method_name: &str, raw: f64) -> f64 {
        if let Some(cm) = self.compu_methods.get(compu_method_name) {
            crate::a2l::compu_method::apply_raw_to_physical(cm, raw)
        } else {
            raw
        }
    }

    /// Converts a physical value to raw value using the named `COMPU_METHOD`.
    pub fn physical_to_raw(&self, compu_method_name: &str, phys: f64) -> f64 {
        if let Some(cm) = self.compu_methods.get(compu_method_name) {
            crate::a2l::compu_method::apply_physical_to_raw(cm, phys)
        } else {
            phys
        }
    }
}
