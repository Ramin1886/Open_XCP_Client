# HDF5-Based `.sdf` File Format & Configurable Size Splitting

[← Back to Documentation Index](./index.md)

## Overview

Implemented in [`src/recording/`](../src/recording/mod.rs), **Open_XCP_Client** records all A2L measurement variables at their native XCP event refresh rates into an **HDF5-based Scientific Data Format (`.sdf`)** file.

---

## Logfile Naming & Part Splitting

Managed by [`RecordingFileNamer`](../src/recording/file_namer.rs) and [`SizeSplitter`](../src/recording/size_splitter.rs):

- **Default Storage Folder**: `./logfile` (configurable in the **Logging Tab** and in **`⚙ Settings`**).
- **Default Prefix**: `Measurement` (configurable in the **Logging Tab** and in **`⚙ Settings`**).
- **Filename Pattern**:
  ```text
  <Prefix>_<YYYY-MM-DD_HH-MM-SSZ>_PART<001>.sdf
  ```
  Example:
  ```text
  ./logfile/Measurement_2026-10-23_01-30-45Z_PART001.sdf
  ./logfile/Measurement_2026-10-23_01-30-45Z_PART002.sdf
  ```
- **Constant Start Timestamp Across Parts**: The `YYYY-MM-DD_HH-MM-SSZ` UTC timestamp is captured at the exact instant the user clicks **Start Recording** and remains identical across all split parts (`PART001`, `PART002`, ...) belonging to that recording session.
- **Configurable Split Threshold (Default 500 MB)**:
  - The maximum logfile size is stored in `./config/settings.json` under `recording.max_logfile_size_mb` (default: **`500` MB**, i.e., `500,000,000` bytes).
  - Users can adjust **Logfile Split Size (MB)** inside the **`⚙ Settings`** menu.
  - Before appending each new DAQ event sample, [`SdfSessionRecorder::record_event_sample`](../src/recording/sdf_writer.rs) checks [`SizeSplitter::should_split_before_adding`](../src/recording/size_splitter.rs). If adding the sample would exceed the limit, the current part is finalized and closed on disk and recording continues seamlessly in `PART{N+1}.sdf`.

---

## Internal Hierarchical Structure

Every `.sdf` file starts with a standards-compliant 48-byte **HDF5 Superblock Version 2** (`\x89HDF\r\n\x1a\n` + Jenkins `lookup3` checksum) and organizes datasets into two top-level groups:

```text
/ (Root Metadata: sdf_version="1.0", start_time_utc, part_number, project_name, module_name)
├── /Events
│   ├── /Event_10ms
│   │   ├── time_utc_s        (float64 [N] — Unix UTC epoch seconds)
│   │   └── time_utc_ns       (int64   [N] — Unix UTC epoch nanoseconds)
│   └── /Event_50ms
│       ├── time_utc_s        (float64 [M] — Unix UTC epoch seconds)
│       └── time_utc_ns       (int64   [M] — Unix UTC epoch nanoseconds)
└── /Measurements
    ├── /Perception
    │   └── /RadarFront
    │       ├── frame_counter            (float64 [N], unit="frames", time_dataset="/Events/Event_10ms/time_utc_s")
    │       └── /targets_0
    │           ├── distance_m           (float64 [N], unit="m")
    │           ├── velocity_mps         (float64 [N], unit="m/s")
    │           ├── lat_offset_m         (float64 [N], unit="m")
    │           └── snr_db               (float64 [N], unit="m/s")
    └── /EgoMotion
        ├── /Kinematics
        │   ├── velocity_x               (float64 [N], unit="m/s")
        │   └── yaw_rate_deg_s           (float64 [N], unit="deg/s")
        └── /Pose
            └── covariance_diag          (float64 [M, 3], unit="m^2", time_dataset="/Events/Event_50ms/time_utc_s")
```

> [!IMPORTANT]
> **Cloud-Ready Purity**: As designed, `.sdf` files contain **only** measurement time-series and structural metadata. Local tool settings and calibration watch-lists are stored exclusively in `./config/settings.json`.

---

## Reading `.sdf` Files in Rust (`SdfFileReader`)

[`SdfFileReader`](../src/recording/sdf_writer.rs) provides zero-dependency verification and dataset extraction:

```rust
use open_xcp_client::recording::SdfFileReader;

let reader = SdfFileReader::open("./logfile/Measurement_2026-10-23_01-30-45Z_PART001.sdf")?;
let timestamps_s = reader.read_f64_dataset("/Events/Event_10ms/time_utc_s").unwrap();
let radar_dist = reader
    .read_f64_dataset("/Measurements/Perception/RadarFront/targets_0/distance_m")
    .unwrap();
```

---

## Next Steps
- [Calibration Guide & Data-Type Validation →](./calibration_guide.md)
- [UI / UX Operator Manual →](./ui_ux_manual.md)
