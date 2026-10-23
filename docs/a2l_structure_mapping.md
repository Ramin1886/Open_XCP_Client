# A2L Parsing & Complex C/C++ Structure Mapping

[← Back to Documentation Index](./index.md)

## Overview

Embedded ADAS software written in C/C++ frequently organizes sensor clusters, object tracks, and ego-motion states into deeply nested structures and arrays of structures. **Open_XCP_Client** preserves these complex hierarchies end-to-end — from the A2L parser ([`src/a2l/parser.rs`](../src/a2l/parser.rs)) and structure resolver ([`src/a2l/struct_resolver.rs`](../src/a2l/struct_resolver.rs)) to the Logging Tab tree view and the recorded `.sdf` (HDF5 v2) groups.

---

## Dual Representation Support

### 1. ASAM MCD-2 MC v1.7 `TYPEDEF_STRUCTURE` & `INSTANCE`
When `xcplite` 2.2.2 emits `TYPEDEF_STRUCTURE`, `STRUCTURE_COMPONENT`, `TYPEDEF_MEASUREMENT`, `TYPEDEF_CHARACTERISTIC`, and `INSTANCE` blocks:
- [`resolve_a2l_structures`](../src/a2l/struct_resolver.rs) recursively walks each `INSTANCE`, adding each component's `ADDRESS_OFFSET` (and array element stride `elem_idx * struct_byte_size` when `MATRIX_DIM` is specified on a struct or instance) to the base `ECU_ADDRESS`.
- Nested `TYPEDEF_CHARACTERISTIC` members inside structures are simultaneously expanded into concrete [`Characteristic`](../src/a2l/model.rs) calibration entries.

### 2. Dot-Separated & Array-Indexed `MEASUREMENT` Names
When `xcplite` macros register flattened hierarchical names (such as `EgoMotion.Kinematics.velocity_x` or `Perception.RadarFront.targets[0].distance_m`):
- [`split_hierarchical_segments`](../src/a2l/struct_resolver.rs) normalizes dots (`.`), slashes (`/`), and array indices (`[0]` $\rightarrow$ `_0`) into clean path segments:
  ```text
  Perception.RadarFront.targets[0].distance_m
    └── /Measurements/Perception/RadarFront/targets_0/distance_m
  ```
- [`build_measurement_hierarchy`](../src/a2l/struct_resolver.rs) constructs a unified [`StructTreeNode`](../src/a2l/model.rs) tree across both `INSTANCE`-derived and dot-path-derived measurements.

---

## Supported Data Types & `COMPU_METHOD` Conversions

Implemented in [`src/a2l/model.rs`](../src/a2l/model.rs) and [`src/a2l/compu_method.rs`](../src/a2l/compu_method.rs):

| ASAM Data Type | Rust Equivalent | Byte Size |
| :--- | :--- | :--- |
| `UBYTE` | `u8` | 1 |
| `SBYTE` | `i8` | 1 |
| `UWORD` | `u16` | 2 |
| `SWORD` | `i16` | 2 |
| `ULONG` | `u32` | 4 |
| `SLONG` | `i32` | 4 |
| `A_UINT64` | `u64` | 8 |
| `A_INT64` | `i64` | 8 |
| `FLOAT32_IEEE` | `f32` | 4 |
| `FLOAT64_IEEE` | `f64` | 8 |

Supported `COMPU_METHOD` types:
- **`IDENTICAL`**: Direct $1:1$ mapping ($\text{phys} = \text{raw}$).
- **`LINEAR`**: $\text{phys} = a \cdot \text{raw} + b$ (`COEFFS_LINEAR a b`).
- **`RAT_FUNC`**: Rational function $\text{raw} = \frac{a \cdot \text{phys}^2 + b \cdot \text{phys} + c}{d \cdot \text{phys}^2 + e \cdot \text{phys} + f}$ (`COEFFS a b c d e f`).
- **`TAB_VERB`**: Verbal enumeration table (`COMPU_VTAB`) mapping integer raw keys to symbolic state strings (e.g., `0 -> "OFF"`, `3 -> "ACTIVE_ACC"`).

---

## Next Steps
- [HDF5-Based `.sdf` File Format & Configurable Size Splitting →](./sdf_file_format.md)
