# HDF5-Based `.sdf` & `.mp4` Video Recording with Configurable Size Splitting

[← Back to Documentation Index](./index.md)

## Overview

Implemented in [`src/recording/`](../src/recording/mod.rs), **Open_XCP_Client** records 100% of A2L measurement variables at their native XCP event refresh rates into an **HDF5-based Scientific Data Format (`.sdf`)** file while simultaneously recording the **2D XY Live Plot** offscreen into a companion **`.mp4` (H.264 / ISO BMFF)** video file.

---

## Synchronized `.sdf` + `.mp4` Naming & Lockstep Part Splitting

Managed by [`RecordingFileNamer`](../src/recording/file_namer.rs), [`SizeSplitter`](../src/recording/size_splitter.rs), [`SdfSessionRecorder`](../src/recording/sdf_writer.rs), and [`Mp4VideoRecorder`](../src/recording/video_recorder.rs):

- **Default Storage Folder**: `./logfile` (configurable in the Top Recording Bar and in **`⚙ Settings`**).
- **Default Prefix**: `Measurement` (configurable in the Top Recording Bar and in **`⚙ Settings`**).
- **Matched Filename Patterns**:
  ```text
  <Prefix>_<YYYY-MM-DD_HH-MM-SSZ>_PART001.sdf
  <Prefix>_<YYYY-MM-DD_HH-MM-SSZ>_PART001.mp4
  ```
- **Constant Start Timestamp Across Parts**: The `YYYY-MM-DD_HH-MM-SSZ` UTC timestamp is captured at the exact instant the user clicks **Start Recording** and remains identical across all split parts (`PART001`, `PART002`, ...) belonging to that recording session.
- **Lockstep Size Splitting (Default 500 MB)**:
  - The maximum part file size is stored in `./config/settings.json` under `recording.max_logfile_size_mb` (default: **`500` MB**, i.e., `500,000,000` bytes).
  - Whenever either the active `.sdf` part or the active `.mp4` video part reaches the configured size limit, **both files finalize and rotate in lockstep** to `PART{N+1}.sdf` and `PART{N+1}.mp4` so corresponding part numbers always cover the exact same time window.
- **Offscreen 2D Video Rasterization**:
  - Because [`render_plane_plot_rgb24`](../src/recording/video_recorder.rs) renders the 2D XY Plot scene offscreen in the background worker at `recording.video_fps` (`1..30 Hz`, default `10 Hz`), `.mp4` video recording continues uninterrupted even when the operator switches to the `Numerical`, `Time Plot`, `Calibration`, or `XCP Info` tabs.

---

## Internal Hierarchical `.sdf` Structure

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
    │           └── snr_db               (float64 [N], unit="dB")
    └── /EgoMotion
        ├── /Kinematics
        │   ├── velocity_x               (float64 [N], unit="m/s")
        │   └── yaw_rate_deg_s           (float64 [N], unit="deg/s")
        └── /Pose
            └── covariance_diag          (float64 [M, 3], unit="m^2", time_dataset="/Events/Event_50ms/time_utc_s")
```

---

## Reading & Replaying `.sdf` Files (`SdfFileReader` & `SdfReplayPlayer`)

[`SdfFileReader`](../src/recording/sdf_writer.rs) and [`SdfReplayPlayer`](../src/replay/mod.rs) provide zero-dependency dataset extraction and multi-part chronological playback:

```rust
use open_xcp_client::replay::SdfReplayPlayer;

let mut player = SdfReplayPlayer::load_files(&[
    "./logfile/Measurement_2026-10-23_01-30-45Z_PART001.sdf",
    "./logfile/Measurement_2026-10-23_01-30-45Z_PART002.sdf",
])?;
let samples_at_5s = player.seek_to_seconds(5.0);
```

---

## Next Steps
- [Calibration Guide & Data-Type Validation →](./calibration_guide.md)
- [UI / UX Operator Manual →](./ui_ux_manual.md)
