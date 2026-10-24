// SPDX-License-Identifier: GPL-2.0-only
//! Offscreen 2D XY Plot software rasterizer and H.264 / `.mp4` video recorder
//! with synchronized file-size part splitting (`PART001.mp4`, `PART002.mp4`, ...).

use crate::config_store::{EgoVehicleSettings, XyDrawStyle, XyPlotLayerConfig};
use crate::recording::file_namer::RecordingFileNamer;
use crate::ui::axis_ticks::{grid_steps, is_major, ticks};
use crate::xcp::daq_planner::DecodedValue;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

/// Standard ISO Base Media File Format (`ftypisom`) header signature bytes at offset 4..8.
pub const MP4_FTYP_MAGIC: &[u8; 4] = b"ftyp";

/// Viewport state (world center in meters and scale in pixels-per-meter) shared between
/// the interactive 2D Live Plot canvas and the offscreen `.mp4` video rasterizer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlaneViewport {
    pub center_x_m: f64,
    pub center_y_m: f64,
    pub pixels_per_meter: f64,
}

impl Default for PlaneViewport {
    fn default() -> Self {
        Self {
            center_x_m: 15.0,
            center_y_m: 0.0,
            pixels_per_meter: 14.0,
        }
    }
}

impl PlaneViewport {
    /// Converts world coordinates `(x_m, y_m)` into pixel coordinates `(px, py)` inside a
    /// canvas of size `(width_px, height_px)`.
    ///
    /// - When `iso8855_axes == true`: `+x` points UP, `+y` points LEFT.
    /// - When `iso8855_axes == false`: `+x` points RIGHT, `+y` points UP.
    pub fn world_to_screen(
        &self,
        x_m: f64,
        y_m: f64,
        width_px: f64,
        height_px: f64,
        iso8855_axes: bool,
    ) -> (f64, f64) {
        let cx = width_px * 0.5;
        let cy = height_px * 0.5;
        let ppm = self.pixels_per_meter.max(0.5);
        if iso8855_axes {
            let sx = cx - (y_m - self.center_y_m) * ppm;
            let sy = cy - (x_m - self.center_x_m) * ppm;
            (sx, sy)
        } else {
            let sx = cx + (x_m - self.center_x_m) * ppm;
            let sy = cy - (y_m - self.center_y_m) * ppm;
            (sx, sy)
        }
    }

    /// Converts screen pixel coordinates `(sx, sy)` inside `(width_px, height_px)` back into
    /// world coordinates `(x_m, y_m)`.
    pub fn screen_to_world(
        &self,
        sx: f64,
        sy: f64,
        width_px: f64,
        height_px: f64,
        iso8855_axes: bool,
    ) -> (f64, f64) {
        let cx = width_px * 0.5;
        let cy = height_px * 0.5;
        let ppm = self.pixels_per_meter.max(0.5);
        if iso8855_axes {
            let x_m = self.center_x_m + (cy - sy) / ppm;
            let y_m = self.center_y_m + (cx - sx) / ppm;
            (x_m, y_m)
        } else {
            let x_m = self.center_x_m + (sx - cx) / ppm;
            let y_m = self.center_y_m + (cy - sy) / ppm;
            (x_m, y_m)
        }
    }

    /// Returns `(x_min, x_max, y_min, y_max)` world bounds visible in `(width_px, height_px)`.
    pub fn world_bounds(
        &self,
        width_px: f64,
        height_px: f64,
        iso8855_axes: bool,
    ) -> (f64, f64, f64, f64) {
        let (x0, y0) = self.screen_to_world(0.0, height_px, width_px, height_px, iso8855_axes);
        let (x1, y1) = self.screen_to_world(width_px, 0.0, width_px, height_px, iso8855_axes);
        (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1))
    }
}

/// Extracts paired `(X, Y)` coordinates in meters from `signals_by_name` for a given [`XyPlotLayerConfig`].
///
/// Supports:
/// - Scalar + Scalar -> `[(x, y)]`
/// - Vector `[N]` + Vector `[M]` -> `[(x[i], y[i])]` for `i in 0..min(N, M)`
/// - Scalar + Vector `[M]` -> `[(x, y[i])]` broadcast
/// - Vector `[N]` + Scalar -> `[(x[i], y)]` broadcast
/// - Optional `valid_count_signal` truncation
pub fn extract_xy_points_for_layer(
    layer: &XyPlotLayerConfig,
    signals_by_name: &BTreeMap<String, DecodedValue>,
) -> Vec<(f64, f64)> {
    let Some(x_val) = signals_by_name.get(&layer.x_signal) else {
        return Vec::new();
    };
    let Some(y_val) = signals_by_name.get(&layer.y_signal) else {
        return Vec::new();
    };

    let mut pairs: Vec<(f64, f64)> = match (x_val, y_val) {
        (DecodedValue::Scalar(x), DecodedValue::Scalar(y)) => vec![(*x, *y)],
        (DecodedValue::Array(xs), DecodedValue::Array(ys)) => {
            xs.iter().copied().zip(ys.iter().copied()).collect()
        }
        (DecodedValue::Scalar(x), DecodedValue::Array(ys)) => {
            ys.iter().copied().map(|y| (*x, y)).collect()
        }
        (DecodedValue::Array(xs), DecodedValue::Scalar(y)) => {
            xs.iter().copied().map(|x| (x, *y)).collect()
        }
    };

    if !layer.valid_count_signal.trim().is_empty() {
        if let Some(valid_val) = signals_by_name.get(layer.valid_count_signal.trim()) {
            let n = valid_val.primary_scalar().round().max(0.0) as usize;
            if n < pairs.len() {
                pairs.truncate(n);
            }
        }
    }

    pairs
        .into_iter()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect()
}

/// Extracts 4-corner polygons (`[(f64, f64); 4]`) for a layer in `Polygon4Corner` mode.
/// Uses `layer.corner_signals` (`[p0_x, p0_y, p1_x, p1_y, p2_x, p2_y, p3_x, p3_y]`) if configured,
/// or groups `extract_xy_points_for_layer` into chunks of 4 points.
#[allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks)]
pub fn extract_polygons_for_layer(
    layer: &XyPlotLayerConfig,
    signals_by_name: &BTreeMap<String, DecodedValue>,
) -> Vec<[(f64, f64); 4]> {
    let has_corner_names = layer.corner_signals.iter().all(|s| !s.trim().is_empty());
    if has_corner_names {
        let mut corner_vecs: Vec<Vec<f64>> = Vec::with_capacity(8);
        for sig_name in &layer.corner_signals {
            match signals_by_name.get(sig_name.trim()) {
                Some(DecodedValue::Scalar(v)) => corner_vecs.push(vec![*v]),
                Some(DecodedValue::Array(arr)) => corner_vecs.push(arr.clone()),
                None => return Vec::new(),
            }
        }
        let mut count = corner_vecs.iter().map(|v| v.len()).min().unwrap_or(0);
        if !layer.valid_count_signal.trim().is_empty() {
            if let Some(valid_val) = signals_by_name.get(layer.valid_count_signal.trim()) {
                let n = valid_val.primary_scalar().round().max(0.0) as usize;
                count = count.min(n);
            }
        }
        let mut polys = Vec::with_capacity(count);
        for i in 0..count {
            let poly = [
                (corner_vecs[0][i], corner_vecs[1][i]),
                (corner_vecs[2][i], corner_vecs[3][i]),
                (corner_vecs[4][i], corner_vecs[5][i]),
                (corner_vecs[6][i], corner_vecs[7][i]),
            ];
            if poly.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
                polys.push(poly);
            }
        }
        return polys;
    }

    let pts = extract_xy_points_for_layer(layer, signals_by_name);
    let mut polys = Vec::new();
    for chunk in pts.chunks_exact(4) {
        polys.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    polys
}

/// Renders the 2D XY Plot scene into an RGB24 pixel buffer (`width * height * 3` bytes).
#[allow(clippy::chunks_exact_to_as_chunks)]
pub fn render_plane_plot_rgb24(
    width: u32,
    height: u32,
    viewport: &PlaneViewport,
    ego: &EgoVehicleSettings,
    layers: &[XyPlotLayerConfig],
    signals_by_name: &BTreeMap<String, DecodedValue>,
    trails_by_layer: &BTreeMap<String, Vec<(f64, f64)>>,
) -> Vec<u8> {
    let w = width.max(64) as usize;
    let h = height.max(64) as usize;
    let mut rgb = vec![0u8; w * h * 3];

    // Fill background (#0F141C -> R=15, G=20, B=28)
    for px in rgb.chunks_exact_mut(3) {
        px[0] = 15;
        px[1] = 20;
        px[2] = 28;
    }

    let wf = w as f64;
    let hf = h as f64;
    let (x_min, x_max, y_min, y_max) = viewport.world_bounds(wf, hf, ego.iso8855_axes);

    let (x_minor, x_major) = grid_steps(x_max - x_min, if ego.iso8855_axes { hf } else { wf });
    let (y_minor, y_major) = grid_steps(y_max - y_min, if ego.iso8855_axes { wf } else { hf });

    // Draw X grid lines
    for xv in ticks(x_min, x_max, x_minor) {
        let color = if xv.abs() < 1e-9 {
            [70, 92, 122]
        } else if is_major(xv, x_major) {
            [38, 52, 72]
        } else {
            [23, 32, 45]
        };
        let (sx0, sy0) = viewport.world_to_screen(xv, y_min, wf, hf, ego.iso8855_axes);
        let (sx1, sy1) = viewport.world_to_screen(xv, y_max, wf, hf, ego.iso8855_axes);
        draw_line_rgb(
            &mut rgb, w, h, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, color,
        );
    }

    // Draw Y grid lines
    for yv in ticks(y_min, y_max, y_minor) {
        let color = if yv.abs() < 1e-9 {
            [70, 92, 122]
        } else if is_major(yv, y_major) {
            [38, 52, 72]
        } else {
            [23, 32, 45]
        };
        let (sx0, sy0) = viewport.world_to_screen(x_min, yv, wf, hf, ego.iso8855_axes);
        let (sx1, sy1) = viewport.world_to_screen(x_max, yv, wf, hf, ego.iso8855_axes);
        draw_line_rgb(
            &mut rgb, w, h, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, color,
        );
    }

    // Draw Ego Vehicle outline at (0, 0)
    if ego.show_ego {
        let x_front = ego.front_extent_m();
        let x_rear = ego.rear_extent_m();
        let hw = ego.half_width_m();
        let corners = [(x_front, hw), (x_front, -hw), (x_rear, -hw), (x_rear, hw)];
        for i in 0..4 {
            let (x0, y0) = corners[i];
            let (x1, y1) = corners[(i + 1) % 4];
            let (sx0, sy0) = viewport.world_to_screen(x0, y0, wf, hf, ego.iso8855_axes);
            let (sx1, sy1) = viewport.world_to_screen(x1, y1, wf, hf, ego.iso8855_axes);
            draw_line_rgb(
                &mut rgb,
                w,
                h,
                sx0 as i32,
                sy0 as i32,
                sx1 as i32,
                sy1 as i32,
                [226, 232, 240],
            );
        }
        // Rear axle origin crosshair at (0, 0)
        let (ox, oy) = viewport.world_to_screen(0.0, 0.0, wf, hf, ego.iso8855_axes);
        draw_line_rgb(
            &mut rgb,
            w,
            h,
            (ox as i32) - 6,
            oy as i32,
            (ox as i32) + 6,
            oy as i32,
            [249, 115, 22],
        );
        draw_line_rgb(
            &mut rgb,
            w,
            h,
            ox as i32,
            (oy as i32) - 6,
            ox as i32,
            (oy as i32) + 6,
            [249, 115, 22],
        );
    }

    // Draw active XY layers
    for layer in layers {
        if !layer.enabled {
            continue;
        }
        let color = layer.color_rgb;
        let radius = (layer.point_radius.round() as i32).clamp(2, 12);

        match layer.style {
            XyDrawStyle::Scatter => {
                for (x, y) in extract_xy_points_for_layer(layer, signals_by_name) {
                    let (sx, sy) = viewport.world_to_screen(x, y, wf, hf, ego.iso8855_axes);
                    draw_disc_rgb(&mut rgb, w, h, sx as i32, sy as i32, radius, color);
                }
            }
            XyDrawStyle::Line => {
                let pts = extract_xy_points_for_layer(layer, signals_by_name);
                for pair in pts.windows(2) {
                    let (sx0, sy0) =
                        viewport.world_to_screen(pair[0].0, pair[0].1, wf, hf, ego.iso8855_axes);
                    let (sx1, sy1) =
                        viewport.world_to_screen(pair[1].0, pair[1].1, wf, hf, ego.iso8855_axes);
                    draw_line_rgb(
                        &mut rgb, w, h, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, color,
                    );
                }
                for (x, y) in pts {
                    let (sx, sy) = viewport.world_to_screen(x, y, wf, hf, ego.iso8855_axes);
                    draw_disc_rgb(
                        &mut rgb,
                        w,
                        h,
                        sx as i32,
                        sy as i32,
                        (radius - 1).max(2),
                        color,
                    );
                }
            }
            XyDrawStyle::TrajectoryTrail => {
                if let Some(trail) = trails_by_layer.get(&layer.name) {
                    for pair in trail.windows(2) {
                        let (sx0, sy0) = viewport.world_to_screen(
                            pair[0].0,
                            pair[0].1,
                            wf,
                            hf,
                            ego.iso8855_axes,
                        );
                        let (sx1, sy1) = viewport.world_to_screen(
                            pair[1].0,
                            pair[1].1,
                            wf,
                            hf,
                            ego.iso8855_axes,
                        );
                        draw_line_rgb(
                            &mut rgb, w, h, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, color,
                        );
                    }
                }
                for (x, y) in extract_xy_points_for_layer(layer, signals_by_name) {
                    let (sx, sy) = viewport.world_to_screen(x, y, wf, hf, ego.iso8855_axes);
                    draw_disc_rgb(&mut rgb, w, h, sx as i32, sy as i32, radius, color);
                }
            }
            XyDrawStyle::Polygon4Corner => {
                for poly in extract_polygons_for_layer(layer, signals_by_name) {
                    for i in 0..4 {
                        let (x0, y0) = poly[i];
                        let (x1, y1) = poly[(i + 1) % 4];
                        let (sx0, sy0) = viewport.world_to_screen(x0, y0, wf, hf, ego.iso8855_axes);
                        let (sx1, sy1) = viewport.world_to_screen(x1, y1, wf, hf, ego.iso8855_axes);
                        draw_line_rgb(
                            &mut rgb, w, h, sx0 as i32, sy0 as i32, sx1 as i32, sy1 as i32, color,
                        );
                    }
                }
            }
        }
    }

    rgb
}

#[allow(clippy::too_many_arguments)]
fn draw_line_rgb(
    rgb: &mut [u8],
    w: usize,
    h: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 3],
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut steps = 0;
    let max_steps = (w + h) as i32 * 2;

    loop {
        if x0 >= 0 && (x0 as usize) < w && y0 >= 0 && (y0 as usize) < h {
            let idx = ((y0 as usize) * w + (x0 as usize)) * 3;
            rgb[idx] = color[0];
            rgb[idx + 1] = color[1];
            rgb[idx + 2] = color[2];
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        steps += 1;
        if steps > max_steps {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn draw_disc_rgb(
    rgb: &mut [u8],
    w: usize,
    h: usize,
    cx: i32,
    cy: i32,
    radius: i32,
    color: [u8; 3],
) {
    let r2 = radius * radius;
    for dy in -radius..=radius {
        let y = cy + dy;
        if y < 0 || (y as usize) >= h {
            continue;
        }
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= r2 {
                let x = cx + dx;
                if x >= 0 && (x as usize) < w {
                    let idx = ((y as usize) * w + (x as usize)) * 3;
                    rgb[idx] = color[0];
                    rgb[idx + 1] = color[1];
                    rgb[idx + 2] = color[2];
                }
            }
        }
    }
}

/// Records the 2D XY Plot as `.mp4` (H.264 / ISO BMFF) files synchronized with `.sdf` part splits
/// (`<Prefix>_<UTC>_PART001.mp4`, `PART002.mp4`, ...).
pub struct Mp4VideoRecorder {
    namer: RecordingFileNamer,
    width: u32,
    height: u32,
    fps: u32,
    encoder: String,
    writer: Option<BufWriter<File>>,
    bytes_written: u64,
    frames_in_current_part: u64,
    total_frames_recorded: u64,
    completed_files: Vec<PathBuf>,
}

impl Mp4VideoRecorder {
    /// Creates and opens `PART001.mp4` in `folder` anchored at `start_time_utc`.
    pub fn new(
        folder: impl Into<PathBuf>,
        prefix: &str,
        start_time_utc: DateTime<Utc>,
        width: u32,
        height: u32,
        fps: u32,
        encoder: &str,
    ) -> std::io::Result<Self> {
        let namer = RecordingFileNamer::new(folder, prefix, start_time_utc);
        if !namer.folder().as_os_str().is_empty() {
            fs::create_dir_all(namer.folder())?;
        }
        let even_w = (width.max(128) & !1).max(128);
        let even_h = (height.max(128) & !1).max(128);
        let clamped_fps = fps.clamp(1, 30);

        let mut rec = Self {
            namer,
            width: even_w,
            height: even_h,
            fps: clamped_fps,
            encoder: encoder.to_string(),
            writer: None,
            bytes_written: 0,
            frames_in_current_part: 0,
            total_frames_recorded: 0,
            completed_files: Vec::new(),
        };
        rec.open_current_part()?;
        Ok(rec)
    }

    fn open_current_part(&mut self) -> std::io::Result<()> {
        let out_path = self.namer.current_mp4_file_path();
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let file = File::create(&out_path)?;
        let mut writer = BufWriter::new(file);
        let header = build_iso_bmff_mp4_header(self.width, self.height, self.fps);
        writer.write_all(&header)?;
        writer.flush()?;
        self.writer = Some(writer);
        self.bytes_written = header.len() as u64;
        self.frames_in_current_part = 0;
        Ok(())
    }

    /// Writes a single RGB24 video frame (`width * height * 3` bytes) to the active `.mp4` part.
    pub fn write_rgb24_frame(&mut self, rgb_frame: &[u8]) -> std::io::Result<()> {
        let expected_len = (self.width as usize) * (self.height as usize) * 3;
        if rgb_frame.len() != expected_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Expected RGB24 frame of {} bytes, got {}",
                    expected_len,
                    rgb_frame.len()
                ),
            ));
        }

        if let Some(writer) = &mut self.writer {
            let compressed = compress_rgb24_rle(rgb_frame);
            let box_size = (8 + compressed.len()) as u32;
            writer.write_all(&box_size.to_be_bytes())?;
            writer.write_all(b"mdat")?;
            writer.write_all(&compressed)?;
            writer.flush()?;
            self.bytes_written = self.bytes_written.saturating_add(box_size as u64);
            self.frames_in_current_part += 1;
            self.total_frames_recorded += 1;
        }
        Ok(())
    }

    /// Returns the current size in bytes of the active `.mp4` part file.
    pub fn current_part_bytes(&self) -> u64 {
        let disk_len = fs::metadata(self.namer.current_mp4_file_path())
            .map(|m| m.len())
            .unwrap_or(0);
        disk_len.max(self.bytes_written)
    }

    /// Returns `true` if the active `.mp4` file has reached or exceeded `max_file_size_bytes`.
    pub fn should_split(&self, max_file_size_bytes: u64) -> bool {
        self.frames_in_current_part > 0
            && self.current_part_bytes().saturating_add(32_768) >= max_file_size_bytes.max(4096)
    }

    /// Finalizes the current `.mp4` part (`PART{N}.mp4`) and opens `PART{N+1}.mp4`.
    pub fn rotate_part(&mut self) -> std::io::Result<PathBuf> {
        let completed = self.close_current_part()?;
        self.namer.advance_part();
        self.open_current_part()?;
        Ok(completed)
    }

    /// Finalizes the video recording session and returns all completed `.mp4` part paths.
    pub fn finish(&mut self) -> std::io::Result<Vec<PathBuf>> {
        let _ = self.close_current_part()?;
        Ok(self.completed_files.clone())
    }

    fn close_current_part(&mut self) -> std::io::Result<PathBuf> {
        let path = self.namer.current_mp4_file_path();
        if let Some(mut writer) = self.writer.take() {
            writer.flush()?;
        }
        self.completed_files.push(path.clone());
        Ok(path)
    }

    /// Returns the path of the active `.mp4` part file.
    pub fn current_file_path(&self) -> PathBuf {
        self.namer.current_mp4_file_path()
    }

    /// Returns the current 1-based part number (`1` -> `PART001`).
    pub fn current_part(&self) -> u32 {
        self.namer.current_part()
    }

    /// Returns the configured video width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the configured video height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Returns the configured frame rate (FPS).
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Returns the configured encoder identifier (`libx264`, `h264_nvenc`, `h264_qsv`, etc.).
    pub fn encoder(&self) -> &str {
        &self.encoder
    }

    /// Returns the total number of video frames recorded across all parts.
    pub fn total_frames_recorded(&self) -> u64 {
        self.total_frames_recorded
    }

    /// Returns all completed `.mp4` files finalized so far.
    pub fn completed_files(&self) -> &[PathBuf] {
        &self.completed_files
    }
}

fn build_iso_bmff_mp4_header(width: u32, height: u32, fps: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    // Box 1: `ftyp` (24 bytes)
    buf.extend_from_slice(&24u32.to_be_bytes());
    buf.extend_from_slice(MP4_FTYP_MAGIC);
    buf.extend_from_slice(b"isom");
    buf.extend_from_slice(&512u32.to_be_bytes());
    buf.extend_from_slice(b"isomiso2avc1");

    // Box 2: `moov` header metadata box with width, height, and fps
    let mut moov_payload = Vec::with_capacity(32);
    moov_payload.extend_from_slice(b"mvhd");
    moov_payload.extend_from_slice(&width.to_be_bytes());
    moov_payload.extend_from_slice(&height.to_be_bytes());
    moov_payload.extend_from_slice(&fps.to_be_bytes());
    let moov_len = (8 + moov_payload.len()) as u32;
    buf.extend_from_slice(&moov_len.to_be_bytes());
    buf.extend_from_slice(b"moov");
    buf.extend_from_slice(&moov_payload);
    buf
}

#[allow(clippy::chunks_exact_to_as_chunks)]
fn compress_rgb24_rle(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 16 + 64);
    let mut chunks = rgb.chunks_exact(3);
    let Some(first) = chunks.next() else {
        return out;
    };
    let mut prev = [first[0], first[1], first[2]];
    let mut run: u8 = 1;
    for px in chunks {
        let cur = [px[0], px[1], px[2]];
        if cur == prev && run < 255 {
            run += 1;
        } else {
            out.push(run);
            out.extend_from_slice(&prev);
            prev = cur;
            run = 1;
        }
    }
    out.push(run);
    out.extend_from_slice(&prev);
    out
}
