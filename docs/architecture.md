# System Architecture & Performance Optimization

[← Back to Documentation Index](./index.md)

## Overview

**Open_XCP_Client** is architected to achieve minimal CPU and RAM usage on developer laptops while sustaining high-frequency multi-rate XCP DAQ recording, offscreen 2D `.mp4` video encoding, `.sdf` replay, and concurrent interactive calibration.

```mermaid
flowchart TB
    subgraph UIThread["Main Thread — UI (src/ui/)"]
        TopBar["Top Bar + Live/Replay Switch + ⚙ Settings\n(src/ui/top_bar.rs)"]
        SettingsModal["Settings Menu Modal\n(src/ui/settings_menu.rs)"]
        PlaneTab["2D Live Plot Tab\n(src/ui/plane_plot_tab.rs)"]
        NumTab["Numerical Tab\n(src/ui/numerical_tab.rs)"]
        TimeTab["Time Plot Tab\n(src/ui/time_plot_tab.rs)"]
        CalTab["Calibration Tab\n(src/ui/calibration_tab.rs)"]
        InfoTab["XCP Info Tab\n(src/ui/xcp_info_tab.rs)"]
        Replay["Multi-Part .sdf Replay Engine\n(src/replay/mod.rs)"]
    end

    subgraph WorkerThread["Background Worker Thread (src/app_state.rs)"]
        XcpClient["XCP Master Client\n(src/xcp/client.rs)"]
        Transport["Async UDP/TCP Demux\n(src/xcp/transport.rs)"]
        DaqPlanner["DAQ ODT Unpacker\n(src/xcp/daq_planner.rs)"]
        SdfRecorder["HDF5 .sdf Session Recorder\n(src/recording/sdf_writer.rs)"]
        Mp4Recorder["Offscreen 2D .mp4 Video Recorder\n(src/recording/video_recorder.rs)"]
    end

    UIThread -->|"WorkerCommand\n(crossbeam_channel)"| WorkerThread
    WorkerThread -->|"WorkerEvent\n(crossbeam_channel)"| UIThread
    Transport -->|"CTO Responses (0xFF/0xFE)"| XcpClient
    Transport -->|"DTO DAQ Packets (<=0xFB)"| DaqPlanner
    DaqPlanner -->|"DaqEventSample"| SdfRecorder
    DaqPlanner -->|"Signal State"| Mp4Recorder
```

---

## Strictly Scoped Module Responsibilities

1. **Configuration Layer ([`src/config_store.rs`](../src/config_store.rs))**:
   - Reads and writes `./config/settings.json` (falling back to `./config/settings.default.json`).
   - Validates IP, port, folder, prefix, `max_logfile_size_mb` (default `500 MB`), `video_fps` (`1..30 Hz`), and `EgoVehicleSettings` (`length_m`, `width_m`, `rear_axle_to_center_m`).
2. **A2L Layer ([`src/a2l/`](../src/a2l/mod.rs))**:
   - Zero-copy lexical scanner ([`lexer.rs`](../src/a2l/lexer.rs)) avoids unnecessary heap allocations when tokenizing multi-megabyte A2L files.
   - Recursive structure expander ([`struct_resolver.rs`](../src/a2l/struct_resolver.rs)) flattens `INSTANCE` / `TYPEDEF_STRUCTURE` trees into concrete memory addresses while preserving the nested hierarchy in `StructTreeNode`.
3. **XCP Layer ([`src/xcp/`](../src/xcp/mod.rs))**:
   - Single-threaded `tokio` runtime handles non-blocking UDP/TCP I/O ([`transport.rs`](../src/xcp/transport.rs)).
   - Incoming packets are immediately split into two dedicated queues:
     - `cto_rx` for command responses (`0xFF` / `0xFE`), enabling sub-millisecond calibration reads/writes.
     - `daq_rx` for streaming DTO DAQ packets (`0x00..=0xFB`), processed continuously for both live monitoring and `.sdf` + `.mp4` recording.
4. **Recording & Replay Layer ([`src/recording/`](../src/recording/mod.rs) & [`src/replay/`](../src/replay/mod.rs))**:
   - Accumulates time-correlated samples per XCP Event channel into HDF5 v2 `.sdf` files while rendering the 2D XY Plot offscreen into `.mp4` (H.264 / ISO BMFF) video files, rotating both in lockstep before exceeding `max_logfile_size_mb`.
   - [`SdfReplayPlayer`](../src/replay/mod.rs) loads one or multiple `.sdf` part files for offline playback and time-slider seeking.
5. **UI Layer ([`src/ui/`](../src/ui/mod.rs))**:
   - Uses `egui`/`eframe` across 5 focused tabs (`2D Live Plot`, `Numerical`, `Time Plot`, `Calibration`, `XCP Info`), capping background repaints to ~30 Hz during active streaming/recording to keep laptop CPU and GPU usage minimal.

---

## Next Steps
- [XCP Protocol & Vector `xcplite` 2.2.2 Integration →](./xcp_protocol_and_xcplite.md)
- [HDF5-Based `.sdf` & `.mp4` Video Recording →](./sdf_file_format.md)
