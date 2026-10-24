// SPDX-License-Identifier: GPL-2.0-only
//! Comprehensive tests for 2D XY Plot (`plane_plot_tab`), `(X, Y)` scalar/vector/polygon
//! signal pairing, `Mp4VideoRecorder` and synchronized `.sdf` + `.mp4` size splitting,
//! `SdfReplayPlayer` multi-part `.sdf` playback & seeking, `TimePlot`, `Numerical`,
//! and `XcpInfo` tabs.

use chrono::{TimeZone, Utc};
use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::app_state::{
    record_synchronized_daq_and_video_sample, update_trails_for_layers, ActiveTab, AppController,
    AppMode, EventRuntimeStats, WorkerEvent,
};
use open_xcp_client::config_store::{
    AppSettings, ConfigStore, EgoVehicleSettings, XyDrawStyle, XyPlotLayerConfig,
};
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use open_xcp_client::recording::{
    extract_polygons_for_layer, extract_xy_points_for_layer, render_plane_plot_rgb24,
    Mp4VideoRecorder, PlaneViewport, SdfSessionRecorder, MP4_FTYP_MAGIC,
};
use open_xcp_client::replay::SdfReplayPlayer;
use open_xcp_client::ui::axis_ticks::{grid_steps, is_major, nice_step, tick_text, ticks};
use open_xcp_client::ui::plane_plot_tab::{
    add_xy_layer_from_inputs, apply_canvas_zoom_and_pan, format_canvas_hover_tooltip,
};
use open_xcp_client::ui::time_plot_tab::{
    reset_time_plot_signals_to_defaults, set_time_plot_signal_selected,
};
use open_xcp_client::ui::top_bar::{resolve_replay_input_paths, trigger_replay_load_from_input};
use open_xcp_client::ui::OpenXcpClientApp;
use open_xcp_client::xcp::daq_planner::{DaqEventSample, DecodedValue};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_axis_ticks_1_2_5_ladder_and_formatting() {
    assert_eq!(nice_step(-1.0), 1.0);
    assert_eq!(nice_step(0.0), 1.0);
    assert!((nice_step(0.08) - 0.1).abs() < 1e-9);
    assert!((nice_step(0.15) - 0.2).abs() < 1e-9);
    assert!((nice_step(0.35) - 0.5).abs() < 1e-9);
    assert!((nice_step(0.75) - 1.0).abs() < 1e-9);
    assert!((nice_step(1.5) - 2.0).abs() < 1e-9);
    assert!((nice_step(3.2) - 5.0).abs() < 1e-9);
    assert!((nice_step(7.5) - 10.0).abs() < 1e-9);

    let (minor, major) = grid_steps(40.0, 800.0);
    assert!(minor > 0.0 && major >= minor);
    assert!(is_major(0.0, major));
    assert!(is_major(major * 2.0, major));
    assert!(!is_major(major * 0.5, major));
    assert!(!is_major(1.0, 0.0));

    let t = ticks(-5.0, 5.0, 2.5);
    assert_eq!(t, vec![-5.0, -2.5, 0.0, 2.5, 5.0]);
    let t_rev = ticks(5.0, -5.0, 5.0);
    assert_eq!(t_rev, vec![-5.0, 0.0, 5.0]);
    assert!(ticks(0.0, 10.0, 0.0).is_empty());

    assert_eq!(tick_text(10.0, 5.0), "10");
    assert_eq!(tick_text(1.5, 0.5), "1.5");
    assert_eq!(tick_text(0.25, 0.05), "0.25");
}

#[test]
fn test_ego_vehicle_settings_and_legacy_config_compat() {
    let mut ego = EgoVehicleSettings::default();
    assert!((ego.front_extent_m() - (1.425 + 4.85 * 0.5)).abs() < 1e-6);
    assert!((ego.rear_extent_m() - (1.425 - 4.85 * 0.5)).abs() < 1e-6);
    assert!((ego.half_width_m() - 0.95).abs() < 1e-6);

    let dir = tempdir().unwrap();
    let vcfg_path = dir.path().join("VehicleConfig.json");
    std::fs::write(
        &vcfg_path,
        r#"{"Length": 5.0, "Width": 2.0, "RearAxleToRearBumper": 1.1}"#,
    )
    .unwrap();
    ego.load_from_vehicle_config_json(&vcfg_path).unwrap();
    assert!((ego.length_m - 5.0).abs() < 1e-6);
    assert!((ego.width_m - 2.0).abs() < 1e-6);
    assert!((ego.rear_extent_m() - (-1.1)).abs() < 1e-6);
    assert!((ego.front_extent_m() - 3.9).abs() < 1e-6);

    let vcfg2_path = dir.path().join("VehicleConfig2.json");
    std::fs::write(
        &vcfg2_path,
        r#"{"length_m": 4.6, "width_m": 1.88, "RearAxleToCenter": 1.35}"#,
    )
    .unwrap();
    ego.load_from_vehicle_config_json(&vcfg2_path).unwrap();
    assert!((ego.rear_axle_to_center_m - 1.35).abs() < 1e-6);

    // Invalid configs return error
    let bad_path = dir.path().join("bad.json");
    std::fs::write(&bad_path, r#"{"Length": 0.05, "Width": 2.0}"#).unwrap();
    assert!(ego.load_from_vehicle_config_json(&bad_path).is_err());
    std::fs::write(&bad_path, r#"{"Width": 2.0}"#).unwrap();
    assert!(ego.load_from_vehicle_config_json(&bad_path).is_err());
    std::fs::write(&bad_path, r#"{"Length": 4.5}"#).unwrap();
    assert!(ego.load_from_vehicle_config_json(&bad_path).is_err());
    assert!(ego
        .load_from_vehicle_config_json(&dir.path().join("missing.json"))
        .is_err());
    std::fs::write(&bad_path, "{ bad json ").unwrap();
    assert!(ego.load_from_vehicle_config_json(&bad_path).is_err());

    // Legacy v0.1 JSON without visualization or video fields loads with defaults
    let legacy_json = r#"{
      "connection": {
        "target_ip": "127.0.0.1",
        "target_port": 5555,
        "transport": "Udp",
        "connect_timeout_ms": 2000,
        "local_a2l_fallback_path": ""
      },
      "recording": {
        "default_log_folder": "./logfile",
        "default_file_prefix": "Measurement",
        "max_logfile_size_mb": 500,
        "prefer_target_utc_clock": true
      },
      "calibration": {
        "watch_list": [],
        "verify_readback": true,
        "active_cal_page": 0
      },
      "ui": {
        "dark_mode": true,
        "enable_mock_server": false
      }
    }"#;
    let legacy_path = dir.path().join("legacy_settings.json");
    std::fs::write(&legacy_path, legacy_json).unwrap();
    let store = ConfigStore::new(&legacy_path);
    let loaded = store.load_or_default().unwrap();
    assert!(loaded.recording.record_video_mp4);
    assert_eq!(loaded.recording.video_fps, 10);
    assert!((loaded.visualization.time_plot_window_s - 20.0).abs() < 1e-6);

    // Validation errors for video_fps and ego dimensions
    let mut bad_fps = AppSettings::default();
    bad_fps.recording.video_fps = 0;
    assert!(bad_fps.validate().is_err());
    bad_fps.recording.video_fps = 60;
    assert!(bad_fps.validate().is_err());

    let mut bad_ego = AppSettings::default();
    bad_ego.visualization.ego.length_m = 0.05;
    assert!(bad_ego.validate().is_err());

    let layer_deser: XyPlotLayerConfig = serde_json::from_str(
        r#"{"name":"L","enabled":true,"x_signal":"x","y_signal":"y","style":"Scatter","color_rgb":[1,2,3]}"#,
    )
    .unwrap();
    assert!((layer_deser.point_radius - 5.0).abs() < 1e-5);
}

#[test]
fn test_xy_signal_pairing_scalar_vector_broadcast_and_polygons() {
    let mut signals = BTreeMap::new();
    signals.insert("scalar_x".to_string(), DecodedValue::Scalar(12.5));
    signals.insert("scalar_y".to_string(), DecodedValue::Scalar(-1.75));
    signals.insert(
        "vec_x".to_string(),
        DecodedValue::Array(vec![10.0, 20.0, 30.0, 40.0]),
    );
    signals.insert(
        "vec_y".to_string(),
        DecodedValue::Array(vec![1.0, 2.0, 3.0, 4.0]),
    );
    signals.insert("valid_n".to_string(), DecodedValue::Scalar(2.0));

    // Missing signal returns empty
    let mut layer = XyPlotLayerConfig {
        name: "S2S".to_string(),
        enabled: true,
        x_signal: "missing".to_string(),
        y_signal: "scalar_y".to_string(),
        valid_count_signal: String::new(),
        corner_signals: Default::default(),
        style: XyDrawStyle::Scatter,
        color_rgb: [255, 0, 0],
        point_radius: 5.0,
    };
    assert!(extract_xy_points_for_layer(&layer, &signals).is_empty());

    // 1. Scalar + Scalar
    layer.x_signal = "scalar_x".to_string();
    assert_eq!(
        extract_xy_points_for_layer(&layer, &signals),
        vec![(12.5, -1.75)]
    );

    // 2. Vector + Vector with valid_count truncation
    layer.x_signal = "vec_x".to_string();
    layer.y_signal = "vec_y".to_string();
    layer.valid_count_signal = "valid_n".to_string();
    assert_eq!(
        extract_xy_points_for_layer(&layer, &signals),
        vec![(10.0, 1.0), (20.0, 2.0)]
    );

    // 3. Scalar + Vector broadcast
    layer.valid_count_signal.clear();
    layer.x_signal = "scalar_x".to_string();
    layer.y_signal = "vec_y".to_string();
    assert_eq!(extract_xy_points_for_layer(&layer, &signals).len(), 4);

    // 4. Vector + Scalar broadcast
    layer.x_signal = "vec_x".to_string();
    layer.y_signal = "scalar_y".to_string();
    assert_eq!(extract_xy_points_for_layer(&layer, &signals).len(), 4);

    // 5. 4-Corner Polygon from chunked 4-element vectors
    layer.x_signal = "vec_x".to_string();
    layer.y_signal = "vec_y".to_string();
    layer.style = XyDrawStyle::Polygon4Corner;
    let polys = extract_polygons_for_layer(&layer, &signals);
    assert_eq!(polys.len(), 1);

    // 6. 4-Corner Polygon from explicit corner_signals (both scalar and array)
    for i in 0..4 {
        signals.insert(
            format!("p{}_x", i),
            DecodedValue::Array(vec![i as f64, (i + 10) as f64]),
        );
        signals.insert(
            format!("p{}_y", i),
            DecodedValue::Array(vec![-(i as f64), -((i + 10) as f64)]),
        );
    }
    layer.corner_signals = [
        "p0_x".to_string(),
        "p0_y".to_string(),
        "p1_x".to_string(),
        "p1_y".to_string(),
        "p2_x".to_string(),
        "p2_y".to_string(),
        "p3_x".to_string(),
        "p3_y".to_string(),
    ];
    layer.valid_count_signal = "valid_n".to_string();
    let explicit_polys = extract_polygons_for_layer(&layer, &signals);
    assert_eq!(explicit_polys.len(), 2);

    // Scalar corner signals
    for i in 0..4 {
        signals.insert(format!("s{}_x", i), DecodedValue::Scalar(i as f64));
        signals.insert(format!("s{}_y", i), DecodedValue::Scalar(-(i as f64)));
    }
    layer.corner_signals = [
        "s0_x".to_string(),
        "s0_y".to_string(),
        "s1_x".to_string(),
        "s1_y".to_string(),
        "s2_x".to_string(),
        "s2_y".to_string(),
        "s3_x".to_string(),
        "s3_y".to_string(),
    ];
    assert_eq!(extract_polygons_for_layer(&layer, &signals).len(), 1);

    // Missing corner signal returns empty
    layer.corner_signals[0] = "nonexistent_corner".to_string();
    assert!(extract_polygons_for_layer(&layer, &signals).is_empty());
}

#[test]
fn test_plane_viewport_transforms_and_offscreen_mp4_recording_with_splitting() {
    let mut vp = PlaneViewport::default();
    for iso in [true, false] {
        let (sx, sy) = vp.world_to_screen(20.0, -3.5, 640.0, 360.0, iso);
        let (wx, wy) = vp.screen_to_world(sx, sy, 640.0, 360.0, iso);
        assert!((wx - 20.0).abs() < 1e-6);
        assert!((wy - (-3.5)).abs() < 1e-6);
        assert!(apply_canvas_zoom_and_pan(
            &mut vp,
            egui::Vec2::new(12.0, -8.0),
            10.0,
            320.0,
            180.0,
            640.0,
            360.0,
            iso
        ));
        assert!(apply_canvas_zoom_and_pan(
            &mut vp,
            egui::Vec2::ZERO,
            -10.0,
            320.0,
            180.0,
            640.0,
            360.0,
            iso
        ));
    }

    let pts_screen = vec![(
        egui::Pos2::new(100.0, 100.0),
        "Target[0]".to_string(),
        15.0,
        -1.2,
    )];
    let near_tip =
        format_canvas_hover_tooltip(egui::Pos2::new(104.0, 103.0), 15.1, -1.1, &pts_screen);
    assert!(near_tip.contains("Target[0]"));
    let far_tip = format_canvas_hover_tooltip(egui::Pos2::new(300.0, 300.0), 5.0, 2.0, &pts_screen);
    assert!(!far_tip.contains("Target[0]"));

    let mut signals = BTreeMap::new();
    signals.insert(
        "vec_x".to_string(),
        DecodedValue::Array(vec![5.0, 10.0, 15.0, 20.0]),
    );
    signals.insert(
        "vec_y".to_string(),
        DecodedValue::Array(vec![-2.0, -1.0, 1.0, 2.0]),
    );
    let mut trails = BTreeMap::new();
    trails.insert(
        "TrailLayer".to_string(),
        vec![(2.0, 0.0), (4.0, 0.5), (6.0, 1.0)],
    );

    let layers = vec![
        XyPlotLayerConfig {
            name: "ScatterLayer".to_string(),
            enabled: true,
            x_signal: "vec_x".to_string(),
            y_signal: "vec_y".to_string(),
            style: XyDrawStyle::Scatter,
            color_rgb: [56, 189, 248],
            ..Default::default()
        },
        XyPlotLayerConfig {
            name: "LineLayer".to_string(),
            enabled: true,
            x_signal: "vec_x".to_string(),
            y_signal: "vec_y".to_string(),
            style: XyDrawStyle::Line,
            color_rgb: [52, 211, 153],
            ..Default::default()
        },
        XyPlotLayerConfig {
            name: "TrailLayer".to_string(),
            enabled: true,
            x_signal: "vec_x".to_string(),
            y_signal: "vec_y".to_string(),
            style: XyDrawStyle::TrajectoryTrail,
            color_rgb: [251, 191, 36],
            ..Default::default()
        },
        XyPlotLayerConfig {
            name: "PolyLayer".to_string(),
            enabled: true,
            x_signal: "vec_x".to_string(),
            y_signal: "vec_y".to_string(),
            style: XyDrawStyle::Polygon4Corner,
            color_rgb: [248, 113, 113],
            ..Default::default()
        },
    ];

    let ego = EgoVehicleSettings::default();
    let rgb = render_plane_plot_rgb24(320, 240, &vp, &ego, &layers, &signals, &trails);
    assert_eq!(rgb.len(), 320 * 240 * 3);

    // 1. Test Mp4VideoRecorder using deterministic native ISO BMFF backend
    let dir = tempdir().unwrap();
    let start_utc = Utc.with_ymd_and_hms(2026, 10, 24, 1, 0, 0).unwrap();
    let mut vid =
        Mp4VideoRecorder::new(dir.path(), "Measurement", start_utc, 320, 240, 10, "native")
            .unwrap();
    assert!(vid.write_rgb24_frame(&[1, 2, 3]).is_err());
    vid.write_rgb24_frame(&rgb).unwrap();
    assert_eq!(vid.total_frames_recorded(), 1);
    assert!(vid.current_part_bytes() > 24);
    assert!(vid.should_split(4096));

    let part1_mp4 = vid.rotate_part().unwrap();
    assert!(part1_mp4.exists());
    let raw_mp4 = std::fs::read(&part1_mp4).unwrap();
    assert_eq!(&raw_mp4[4..8], MP4_FTYP_MAGIC);
    assert_eq!(vid.current_part(), 2);

    vid.write_rgb24_frame(&rgb).unwrap();
    let completed = vid.finish().unwrap();
    assert_eq!(completed.len(), 2);

    // 2. Also test Mp4VideoRecorder with libx264 (exercises ffmpeg backend when ffmpeg is present)
    let mut vid_ff =
        Mp4VideoRecorder::new(dir.path(), "FfmpegTest", start_utc, 320, 240, 10, "libx264")
            .unwrap();
    assert_eq!(vid_ff.encoder(), "libx264");
    assert_eq!(vid_ff.fps(), 10);
    vid_ff.write_rgb24_frame(&rgb).unwrap();
    let _ = vid_ff.rotate_part().unwrap();
    vid_ff.write_rgb24_frame(&rgb).unwrap();
    let ff_completed = vid_ff.finish().unwrap();
    assert_eq!(ff_completed.len(), 2);
    assert_eq!(vid_ff.completed_files().len(), 2);
}

#[tokio::test]
async fn test_multi_part_sdf_replay_player_and_all_5_tabs_and_worker_video_split() {
    let dir = tempdir().unwrap();
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let start_utc = Utc.with_ymd_and_hms(2026, 10, 24, 2, 0, 0).unwrap();

    // Empty load_files returns error
    let empty_paths: Vec<PathBuf> = Vec::new();
    assert!(SdfReplayPlayer::load_files(&empty_paths).is_err());

    // Record a 2-part .sdf session
    let mut rec =
        SdfSessionRecorder::new(&model, dir.path(), "DriveLog", start_utc, 1_000_000).unwrap();
    for step in 0..20 {
        let mut values = BTreeMap::new();
        for (idx, m) in model.measurements.iter().enumerate() {
            if m.element_count() > 1 {
                values.insert(
                    idx,
                    DecodedValue::Array(vec![step as f64 + 1.0; m.element_count()]),
                );
            } else {
                values.insert(idx, DecodedValue::Scalar(step as f64 * 0.5));
            }
        }
        rec.record_event_sample(&DaqEventSample {
            daq_list_num: 0,
            event_channel_id: 0,
            event_name: "Event_10ms".to_string(),
            time_utc_s: 1_792_800_000.0 + (step as f64) * 0.1,
            time_utc_ns: 1_792_800_000_000_000_000 + (step as i64) * 100_000_000,
            values,
        })
        .unwrap();
        if step == 9 {
            rec.force_rotate_part().unwrap();
        }
    }
    let sdf_files = rec.finish().unwrap();
    assert_eq!(sdf_files.len(), 2);

    // Resolve paths from directory and from comma-separated file list
    let resolved_dir = resolve_replay_input_paths(&dir.path().display().to_string());
    assert_eq!(resolved_dir.len(), 2);
    let comma_input = format!("{}, {}", sdf_files[0].display(), sdf_files[1].display());
    assert_eq!(resolve_replay_input_paths(&comma_input).len(), 2);

    // Load multi-part .sdf into SdfReplayPlayer
    let mut player = SdfReplayPlayer::load_files(&sdf_files).unwrap();
    assert_eq!(player.loaded_files().len(), 2);
    assert_eq!(player.total_samples(), 20);
    assert!((player.duration_s() - 1.9).abs() < 1e-4);
    assert!(player.end_utc_s() > player.start_utc_s());

    player.toggle_play_pause();
    assert!(player.is_playing());
    player.toggle_play_pause();
    assert!(!player.is_playing());
    player.play();
    player.set_speed(2.0);
    let step_samples = player.advance_by_wall_dt(0.25);
    assert!(!step_samples.is_empty());

    let preroll = player.seek_to_seconds(1.5);
    assert!(!preroll.is_empty());
    assert!((player.position_s() - 1.5).abs() < 1e-6);

    player.set_speed(0.0);
    let max_samples = player.advance_by_wall_dt(0.1);
    assert!(!max_samples.is_empty());
    assert!(!player.is_playing());
    // Calling play() when at the end restarts from 0.0
    player.play();
    assert_eq!(player.position_s(), 0.0);
    player.stop();

    // Exercise AppController + OpenXcpClientApp across all 5 tabs in Live & Replay modes
    let store = ConfigStore::new(dir.path().join("settings.json"));
    let mut controller = AppController::new(store);
    controller.replay_file_input = dir.path().display().to_string();
    controller.load_replay_files(&sdf_files).unwrap();
    assert_eq!(controller.mode, AppMode::Replay);
    controller.replay_seek(0.8);
    if let Some(p) = &mut controller.replay_player {
        p.play();
    }

    // Add all 4 XyDrawStyle variants to controller.settings.visualization.xy_layers with 4-element vector signals
    controller.latest_signals_by_name.insert(
        "vec4_x".to_string(),
        DecodedValue::Array(vec![5.0, 12.0, 12.0, 5.0]),
    );
    controller.latest_signals_by_name.insert(
        "vec4_y".to_string(),
        DecodedValue::Array(vec![-2.0, -2.0, 2.0, 2.0]),
    );
    controller.trails_by_layer.insert(
        "Layer_TrajectoryTrail".to_string(),
        vec![(2.0, -1.0), (6.0, 0.0), (10.0, 1.0)],
    );
    for style in [
        XyDrawStyle::Scatter,
        XyDrawStyle::Line,
        XyDrawStyle::TrajectoryTrail,
        XyDrawStyle::Polygon4Corner,
    ] {
        controller.new_layer_name = format!("Layer_{:?}", style);
        controller.new_layer_x_signal = "vec4_x".to_string();
        controller.new_layer_y_signal = "vec4_y".to_string();
        controller.new_layer_valid_count_signal.clear();
        controller.new_layer_style = style;
        assert!(add_xy_layer_from_inputs(&mut controller));
    }
    controller.new_layer_name.clear();
    controller.new_layer_valid_count_signal = "vec4_x".to_string();
    assert!(add_xy_layer_from_inputs(&mut controller));
    controller.new_layer_x_signal.clear();
    assert!(!add_xy_layer_from_inputs(&mut controller));

    let mut app = OpenXcpClientApp::new(controller);
    let ctx = egui::Context::default();

    for tab in [
        ActiveTab::PlanePlot,
        ActiveTab::Numerical,
        ActiveTab::TimePlot,
        ActiveTab::Calibration,
        ActiveTab::XcpInfo,
    ] {
        app.controller.active_tab = tab;
        let mut raw = egui::RawInput::default();
        raw.events
            .push(egui::Event::PointerMoved(egui::Pos2::new(500.0, 260.0)));
        raw.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::Vec2::new(0.0, 12.0),
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(raw, |ctx| {
            app.render_frame(ctx);
        });
    }

    // Exercise Numerical expand/collapse and Cartesian 2D mode and single-signal TimePlot
    app.controller.numerical_expand_all = Some(false);
    app.controller.active_tab = ActiveTab::Numerical;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    app.controller.settings.visualization.ego.iso8855_axes = false;
    app.controller.active_tab = ActiveTab::PlanePlot;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    app.controller
        .settings
        .visualization
        .time_plot_signals
        .truncate(1);
    app.controller.active_tab = ActiveTab::TimePlot;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    // Also render TimePlot & Replay top bar when time_plot_signals is empty and replay_player is None
    app.controller
        .settings
        .visualization
        .time_plot_signals
        .clear();
    let saved_player = app.controller.replay_player.take();
    let saved_model = app.controller.a2l_model.take();
    app.controller.mode = AppMode::Replay;
    app.controller.active_tab = ActiveTab::TimePlot;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });
    app.controller.replay_player = saved_player;
    app.controller.a2l_model = saved_model;

    // Exercise XCP Info tab while recording is active with completed .sdf and .mp4 files
    app.controller.set_mode(AppMode::Live);
    app.controller.is_recording = true;
    app.controller.active_recording_file = Some(PathBuf::from("./logfile/Test_PART002.sdf"));
    app.controller.active_video_file = Some(PathBuf::from("./logfile/Test_PART002.mp4"));
    app.controller
        .handle_worker_event(WorkerEvent::VideoPartSplit {
            completed_video_file: PathBuf::from("./logfile/Test_PART001.mp4"),
            new_active_video_file: PathBuf::from("./logfile/Test_PART002.mp4"),
            new_part_number: 2,
        });
    app.controller
        .completed_files
        .push(PathBuf::from("./logfile/Test_PART001.sdf"));
    app.controller.active_tab = ActiveTab::XcpInfo;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    // Exercise keyboard shortcuts (Ctrl+1..Ctrl+5, Ctrl+R), helpers, and EventRuntimeStats
    let mut ev_stat = EventRuntimeStats::new(1, "Event_10ms".to_string(), 10.0);
    assert!(ev_stat.mean_cycle_ms().is_none());
    assert!(ev_stat.max_cycle_ms().is_none());
    assert!(ev_stat.age_ms().is_none());
    ev_stat
        .recent_intervals_ms
        .push_back((std::time::Instant::now(), 9.8));
    ev_stat
        .recent_intervals_ms
        .push_back((std::time::Instant::now(), 10.4));
    ev_stat.last_arrival = Some(std::time::Instant::now());
    assert!(ev_stat.mean_cycle_ms().unwrap() > 9.0);
    assert!(ev_stat.max_cycle_ms().unwrap() >= 10.4);
    assert!(ev_stat.age_ms().is_some());

    reset_time_plot_signals_to_defaults(&mut app.controller);
    assert!(!app
        .controller
        .settings
        .visualization
        .time_plot_signals
        .is_empty());
    set_time_plot_signal_selected(&mut app.controller, "CustomSig", true);
    assert!(app
        .controller
        .settings
        .visualization
        .time_plot_signals
        .contains(&"CustomSig".to_string()));
    set_time_plot_signal_selected(&mut app.controller, "CustomSig", false);

    app.controller.replay_file_input = "/nonexistent/folder_or_file.sdf".to_string();
    trigger_replay_load_from_input(&mut app.controller);
    assert!(app.controller.status_message.contains("No .sdf files"));
    let bad_sdf = dir.path().join("corrupt.sdf");
    std::fs::write(&bad_sdf, b"not_an_hdf5_file").unwrap();
    app.controller.replay_file_input = bad_sdf.display().to_string();
    trigger_replay_load_from_input(&mut app.controller);
    assert!(app.controller.status_message.contains("Replay load error"));

    for key in [
        egui::Key::Num1,
        egui::Key::Num2,
        egui::Key::Num3,
        egui::Key::Num4,
        egui::Key::Num5,
        egui::Key::R,
    ] {
        let mut raw = egui::RawInput::default();
        raw.modifiers.ctrl = true;
        raw.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
        });
        let _ = ctx.run(raw, |ctx| {
            app.render_frame(ctx);
        });
    }

    // Exercise direct record_synchronized_daq_and_video_sample (both SDF-triggered and Video-triggered splits)
    let sync_dir = dir.path().join("direct_sync_split");
    let mut direct_rec =
        SdfSessionRecorder::new(&model, &sync_dir, "DirectSync", start_utc, 4096).unwrap();
    let mut direct_vid = Some(
        Mp4VideoRecorder::new(&sync_dir, "DirectSync", start_utc, 320, 240, 10, "native").unwrap(),
    );
    let init_rgb = render_plane_plot_rgb24(
        320,
        240,
        &PlaneViewport::default(),
        &EgoVehicleSettings::default(),
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    direct_vid
        .as_mut()
        .unwrap()
        .write_rgb24_frame(&init_rgb)
        .unwrap();
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded();
    let mut sig_map = BTreeMap::new();
    sig_map.insert("veh_speed_mps".to_string(), DecodedValue::Scalar(15.0));
    let mut trail_map = BTreeMap::new();
    update_trails_for_layers(
        &app.controller.settings.visualization.xy_layers,
        &sig_map,
        &mut trail_map,
    );
    let sample0 = DaqEventSample {
        daq_list_num: 0,
        event_channel_id: 0,
        event_name: "Event_10ms".to_string(),
        time_utc_s: 1_792_800_100.0,
        time_utc_ns: 1_792_800_100_000_000_000,
        values: BTreeMap::from([(0, DecodedValue::Scalar(15.0))]),
    };
    record_synchronized_daq_and_video_sample(
        &mut direct_rec,
        &mut direct_vid,
        true,
        &PlaneViewport::default(),
        &EgoVehicleSettings::default(),
        &app.controller.settings.visualization.xy_layers,
        &sig_map,
        &trail_map,
        sample0,
        &evt_tx,
    );
    let emitted: Vec<WorkerEvent> = evt_rx.try_iter().collect();
    assert!(emitted
        .iter()
        .any(|e| matches!(e, WorkerEvent::VideoPartSplit { .. })));

    // Also trigger SDF-led split inside record_synchronized_daq_and_video_sample (emit_video_frame = false)
    for step in 1..25 {
        let mut vals = BTreeMap::new();
        for (idx, m) in model.measurements.iter().enumerate() {
            if m.element_count() > 1 {
                vals.insert(
                    idx,
                    DecodedValue::Array(vec![step as f64; m.element_count()]),
                );
            } else {
                vals.insert(idx, DecodedValue::Scalar(step as f64));
            }
        }
        record_synchronized_daq_and_video_sample(
            &mut direct_rec,
            &mut direct_vid,
            false,
            &PlaneViewport::default(),
            &EgoVehicleSettings::default(),
            &app.controller.settings.visualization.xy_layers,
            &sig_map,
            &trail_map,
            DaqEventSample {
                daq_list_num: 0,
                event_channel_id: 0,
                event_name: "Event_10ms".to_string(),
                time_utc_s: 1_792_800_100.0 + (step as f64) * 0.01,
                time_utc_ns: 1_792_800_100_000_000_000 + (step as i64) * 10_000_000,
                values: vals,
            },
            &evt_tx,
        );
    }
    let emitted_sdf_split: Vec<WorkerEvent> = evt_rx.try_iter().collect();
    assert!(emitted_sdf_split
        .iter()
        .any(|e| matches!(e, WorkerEvent::RecordingPartSplit { .. })));

    // Exercise background worker live DAQ streaming + synchronized .sdf and .mp4 recording with small split threshold
    let rec_dir = dir.path().join("live_rec");
    app.controller.settings.ui.enable_mock_server = true;
    app.controller.settings.recording.record_video_mp4 = true;
    app.controller.settings.recording.video_encoder = "native".to_string();
    app.controller.settings.recording.video_fps = 30;
    app.controller.settings.recording.video_width = 160;
    app.controller.settings.recording.video_height = 120;
    app.controller.log_folder_input = rec_dir.display().to_string();
    app.controller.is_recording = false;
    app.controller.request_connect();

    let wait = std::time::Instant::now();
    while !app.controller.connected && wait.elapsed() < Duration::from_secs(2) {
        app.controller.poll_worker_events();
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    assert!(app.controller.connected);

    // Wait briefly so LiveSample events arrive before starting recording
    tokio::time::sleep(Duration::from_millis(45)).await;
    app.controller.poll_worker_events();
    assert!(!app.controller.latest_measurement_values.is_empty());

    // Start recording with a tiny max_size_bytes so both .sdf and .mp4 split in the worker
    app.controller.sync_video_scene_to_worker();
    app.controller
        .send_worker_command(open_xcp_client::app_state::WorkerCommand::StartRecording {
            folder: rec_dir.display().to_string(),
            prefix: "SyncSplit".to_string(),
            max_size_bytes: 20_000,
        });
    tokio::time::sleep(Duration::from_millis(160)).await;
    app.controller.poll_worker_events();
    app.controller.toggle_recording(); // Stop recording
    tokio::time::sleep(Duration::from_millis(50)).await;
    app.controller.poll_worker_events();
    assert!(!app.controller.completed_files.is_empty());
    assert!(!app.controller.completed_video_files.is_empty());

    // Start recording again and disconnect while recording is active so Disconnect finalizes both .sdf and .mp4
    app.controller.toggle_recording();
    tokio::time::sleep(Duration::from_millis(40)).await;
    app.controller.request_disconnect();
    tokio::time::sleep(Duration::from_millis(40)).await;
    app.controller.poll_worker_events();
}
