// SPDX-License-Identifier: GPL-2.0-only
//! HDF5-based Scientific Data Format (`.sdf`) multi-rate time-series recorder,
//! human-readable UTC filename generator, and configurable file-size splitter.

pub mod file_namer;
pub mod sdf_writer;
pub mod size_splitter;
pub mod video_recorder;

pub use file_namer::{format_utc_for_filename, RecordingFileNamer};
pub use sdf_writer::{
    jenkins_lookup3_checksum, SdfDatasetSummary, SdfError, SdfFileMetadata, SdfFileReader,
    SdfGapRecord, SdfIntegritySummary, SdfProvenance, SdfSessionRecorder, HDF5_SIGNATURE,
};
pub use size_splitter::SizeSplitter;
pub use video_recorder::{
    build_ffmpeg_args, extract_polygons_for_layer, extract_xy_points_for_layer,
    render_plane_plot_rgb24, Mp4VideoRecorder, PlaneViewport, MP4_FTYP_MAGIC,
};
