// SPDX-License-Identifier: GPL-2.0-only
//! Tests for named, reusable plot profiles (`./config/profiles/<Name>.json`):
//! save/list/load round-trips, atomic writes, name sanitization, schema validation,
//! version gating, unresolved-signal detection against an A2L model, and the
//! [`AppController`] integration (apply, startup auto-load, default-seeding suppression).

use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::app_state::AppController;
use open_xcp_client::config_store::{
    AppSettings, ConfigStore, EgoVehicleSettings, XyDrawStyle, XyPlotLayerConfig,
};
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use open_xcp_client::plot_profile::{
    PlotProfile, ProfileError, ProfileStore, ProfileSummary, MAX_TIME_PLOT_WINDOW_S,
    PROFILE_FORMAT_VERSION,
};
use open_xcp_client::recording::video_recorder::PlaneViewport;
use open_xcp_client::ui::profile_panel::{
    apply_profile_action, render_profile_collapsing, render_profile_section,
    render_profile_selector_items, render_unresolved_warning, ProfileAction,
};
use open_xcp_client::ui::time_plot_tab::MAX_TIME_PLOT_SIGNALS;
use open_xcp_client::ui::OpenXcpClientApp;
use std::fs;
use tempfile::{tempdir, TempDir};

/// Builds a profile referencing real signal names taken from the mock ADAS A2L.
fn sample_profile(name: &str, signals: &[String]) -> PlotProfile {
    PlotProfile {
        format_version: PROFILE_FORMAT_VERSION,
        name: name.to_string(),
        description: "Round-trip fixture".to_string(),
        created_utc: "2026-10-24T00:00:00+00:00".to_string(),
        a2l_project: "MockProject".to_string(),
        a2l_module: "MockModule".to_string(),
        ego: EgoVehicleSettings {
            length_m: 5.10,
            width_m: 2.05,
            rear_axle_to_center_m: 1.60,
            show_ego: false,
            iso8855_axes: false,
        },
        xy_layers: vec![XyPlotLayerConfig {
            name: "Pair".to_string(),
            enabled: true,
            x_signal: signals[0].clone(),
            y_signal: signals[1].clone(),
            valid_count_signal: String::new(),
            corner_signals: Default::default(),
            style: XyDrawStyle::Line,
            color_rgb: [10, 20, 30],
            point_radius: 4.25,
        }],
        time_plot_signals: vec![signals[0].clone(), signals[1].clone()],
        time_plot_window_s: 60.0,
        calibration_watch_list: vec!["Cal.Controller.kp_gain".to_string()],
        viewport: Some(PlaneViewport {
            center_x_m: 3.5,
            center_y_m: -2.25,
            pixels_per_meter: 42.0,
        }),
    }
}

/// Returns the first `count` expanded leaf measurement names of the mock ADAS A2L.
fn mock_signal_names(count: usize) -> Vec<String> {
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    assert!(
        model.measurements.len() >= count,
        "mock A2L should expand to at least {} measurements",
        count
    );
    model
        .measurements
        .iter()
        .take(count)
        .map(|m| m.name.clone())
        .collect()
}

/// Creates a controller whose settings and profiles both live in a fresh temp directory.
fn controller_in(dir: &TempDir) -> AppController {
    let store = ConfigStore::new(dir.path().join("settings.json"));
    let profiles = ProfileStore::new(dir.path().join("profiles"));
    AppController::new_with_profile_store(store, profiles)
}

#[test]
fn test_profile_save_list_load_round_trip() {
    let dir = tempdir().unwrap();
    let store = ProfileStore::new(dir.path().join("profiles"));
    let signals = mock_signal_names(2);

    // An empty / missing profile directory lists cleanly.
    assert!(store.list().unwrap().is_empty());

    let profile = sample_profile("Round Trip", &signals);
    let path = store.save(&profile).unwrap();
    assert!(path.exists());
    assert_eq!(path.file_name().unwrap(), "Round_Trip.json");

    // The file is pretty-printed JSON, not a single line.
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\n  \"name\": \"Round Trip\""));

    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Round Trip");
    assert_eq!(listed[0].layer_count, 1);
    assert_eq!(listed[0].time_plot_signal_count, 2);
    assert!(!listed[0].is_invalid());
    assert!(listed[0].label().contains("1 layer(s) / 2 signal(s)"));

    let loaded = store.load("Round Trip").unwrap();
    assert_eq!(loaded, profile);
    assert_eq!(loaded.viewport.unwrap().pixels_per_meter, 42.0);
    assert!(!loaded.ego.iso8855_axes);

    // Overwriting in place keeps exactly one file and leaves no .tmp behind.
    let mut updated = profile.clone();
    updated.description = "Second write".to_string();
    store.save(&updated).unwrap();
    let entries: Vec<_> = fs::read_dir(dir.path().join("profiles"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(entries, vec!["Round_Trip.json".to_string()]);
    assert_eq!(
        store.load("Round Trip").unwrap().description,
        "Second write"
    );

    store.delete("Round Trip").unwrap();
    assert!(store.list().unwrap().is_empty());
    assert!(matches!(
        store.delete("Round Trip"),
        Err(ProfileError::NotFound(_, _))
    ));
    assert!(matches!(
        store.load("Round Trip"),
        Err(ProfileError::NotFound(_, _))
    ));
}

#[test]
fn test_profile_name_sanitization_and_export_import() {
    let dir = tempdir().unwrap();
    let store = ProfileStore::new(dir.path().join("profiles"));

    assert_eq!(
        ProfileStore::file_name_for("Park In / Test #2").unwrap(),
        "Park_In_Test_2.json"
    );
    assert!(matches!(
        ProfileStore::file_name_for("   "),
        Err(ProfileError::Validation(_))
    ));
    assert!(matches!(
        ProfileStore::file_name_for("///"),
        Err(ProfileError::Validation(_))
    ));
    assert!(store
        .path_for("Valid-Name_1")
        .unwrap()
        .ends_with("Valid-Name_1.json"));

    // Export to an arbitrary path, then import it back through load_from_path.
    let signals = mock_signal_names(2);
    let profile = sample_profile("Exported", &signals);
    let export_path = dir.path().join("share").join("exported_view.json");
    store.save_to_path(&profile, &export_path).unwrap();
    assert!(export_path.exists());
    assert!(!export_path.with_extension("tmp").exists());
    assert_eq!(store.load_from_path(&export_path).unwrap(), profile);

    assert!(matches!(
        store.load_from_path(&dir.path().join("missing.json")),
        Err(ProfileError::Io { .. })
    ));
}

#[test]
fn test_profile_validation_and_version_gating() {
    let dir = tempdir().unwrap();
    let store = ProfileStore::new(dir.path().join("profiles"));
    let signals = mock_signal_names(2);
    let base = sample_profile("Valid", &signals);

    // Empty name.
    let mut invalid = base.clone();
    invalid.name = "   ".to_string();
    assert!(matches!(
        invalid.validate(),
        Err(ProfileError::Validation(_))
    ));

    // Out-of-range time window, both directions.
    let mut invalid = base.clone();
    invalid.time_plot_window_s = 0.5;
    assert!(invalid.validate().is_err());
    invalid.time_plot_window_s = MAX_TIME_PLOT_WINDOW_S + 1.0;
    assert!(invalid.validate().is_err());

    // Too many Time Plot signals.
    let mut invalid = base.clone();
    invalid.time_plot_signals = (0..=MAX_TIME_PLOT_SIGNALS)
        .map(|i| format!("Signal_{}", i))
        .collect();
    assert!(invalid.validate().is_err());

    // A non-polygon layer missing its Y signal is rejected; a polygon layer may omit X/Y.
    let mut invalid = base.clone();
    invalid.xy_layers[0].y_signal = String::new();
    assert!(invalid.validate().is_err());
    let mut polygon = invalid.clone();
    polygon.xy_layers[0].x_signal = String::new();
    polygon.xy_layers[0].style = XyDrawStyle::Polygon4Corner;
    assert!(polygon.validate().is_ok());

    // Saving an invalid profile fails and writes nothing.
    assert!(store.save(&invalid).is_err());
    assert!(store.list().unwrap().is_empty());

    // A file from a newer schema version is rejected by name.
    let newer = dir.path().join("newer.json");
    fs::create_dir_all(dir.path()).unwrap();
    fs::write(
        &newer,
        format!(
            r#"{{"format_version": {}, "name": "FromTheFuture"}}"#,
            PROFILE_FORMAT_VERSION + 1
        ),
    )
    .unwrap();
    match store.load_from_path(&newer) {
        Err(ProfileError::UnsupportedVersion { name, found, .. }) => {
            assert_eq!(name, "FromTheFuture");
            assert_eq!(found, PROFILE_FORMAT_VERSION + 1);
        }
        other => panic!(
            "expected UnsupportedVersion, got {:?}",
            other.map(|p| p.name)
        ),
    }

    // Unknown fields are tolerated and omitted optional fields take their defaults.
    let minimal = dir.path().join("minimal.json");
    fs::write(
        &minimal,
        r#"{"name": "Minimal", "unknown_future_field": 42}"#,
    )
    .unwrap();
    let loaded = store.load_from_path(&minimal).unwrap();
    assert_eq!(loaded.name, "Minimal");
    assert_eq!(loaded.format_version, PROFILE_FORMAT_VERSION);
    assert_eq!(loaded.time_plot_window_s, 20.0);
    assert!(loaded.viewport.is_none());
    assert!(loaded.xy_layers.is_empty());
    assert_eq!(
        loaded,
        PlotProfile {
            name: "Minimal".to_string(),
            ..PlotProfile::default()
        }
    );

    // Malformed JSON surfaces as a Json error.
    let broken = dir.path().join("broken.json");
    fs::write(&broken, "{ not json").unwrap();
    assert!(matches!(
        store.load_from_path(&broken),
        Err(ProfileError::Json { .. })
    ));
}

#[test]
fn test_list_tolerates_unreadable_profile_files() {
    let dir = tempdir().unwrap();
    let profiles_dir = dir.path().join("profiles");
    let store = ProfileStore::new(&profiles_dir);
    let signals = mock_signal_names(2);

    store.save(&sample_profile("Good", &signals)).unwrap();
    fs::write(profiles_dir.join("Broken.json"), "{ not json").unwrap();
    // Non-JSON files and subdirectories are skipped entirely.
    fs::write(profiles_dir.join("notes.txt"), "ignore me").unwrap();
    fs::create_dir_all(profiles_dir.join("nested.json")).unwrap();

    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 2);
    let broken = listed.iter().find(|s| s.name == "Broken").unwrap();
    assert!(broken.is_invalid());
    assert!(broken.label().starts_with("⚠ Broken (unreadable:"));
    let good = listed.iter().find(|s| s.name == "Good").unwrap();
    assert!(!good.is_invalid());
}

#[test]
fn test_unresolved_signals_against_a2l_model() {
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let signals = mock_signal_names(2);
    let mut profile = sample_profile("Mixed", &signals);

    // Every referenced name exists in the mock A2L.
    assert!(profile.unresolved_signals(&model).is_empty());
    assert_eq!(profile.referenced_signals().len(), 2);

    // Add bogus measurement, corner, valid-count and calibration references.
    profile.xy_layers.push(XyPlotLayerConfig {
        name: "Ghost".to_string(),
        enabled: true,
        x_signal: "Not.A.Signal".to_string(),
        y_signal: signals[1].clone(),
        valid_count_signal: "Not.A.Count".to_string(),
        corner_signals: [
            "Not.A.Corner".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ],
        style: XyDrawStyle::Polygon4Corner,
        color_rgb: [1, 2, 3],
        point_radius: 3.0,
    });
    profile
        .time_plot_signals
        .push("Not.A.TimeSignal".to_string());
    profile
        .calibration_watch_list
        .push("Cal.Not.A.Parameter".to_string());

    let unresolved = profile.unresolved_signals(&model);
    assert_eq!(unresolved.len(), 5);
    assert!(unresolved.contains("Not.A.Signal"));
    assert!(unresolved.contains("Not.A.Count"));
    assert!(unresolved.contains("Not.A.Corner"));
    assert!(unresolved.contains("Not.A.TimeSignal"));
    assert!(unresolved.contains("Cal.Not.A.Parameter"));
    // Real signals and the real calibration parameter are not reported.
    assert!(!unresolved.contains(signals[1].as_str()));
    assert!(!unresolved.contains("Cal.Controller.kp_gain"));
}

#[test]
fn test_apply_profile_updates_controller_and_suppresses_default_seeding() {
    let dir = tempdir().unwrap();
    let mut controller = controller_in(&dir);
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let signals = mock_signal_names(2);
    controller.a2l_model = Some(model.clone());

    // Stale trail history must not survive a profile switch.
    controller
        .trails_by_layer
        .insert("stale".to_string(), vec![(1.0, 2.0)]);

    let profile = sample_profile("Applied", &signals);
    controller.apply_profile(&profile);

    assert_eq!(
        controller.settings.visualization.xy_layers,
        profile.xy_layers
    );
    assert_eq!(
        controller.settings.visualization.time_plot_signals,
        profile.time_plot_signals
    );
    assert_eq!(controller.settings.visualization.time_plot_window_s, 60.0);
    assert_eq!(controller.settings.visualization.ego, profile.ego);
    assert_eq!(controller.plane_viewport, profile.viewport.unwrap());
    assert!(controller.trails_by_layer.is_empty());
    assert_eq!(controller.settings.active_profile, "Applied");
    assert_eq!(controller.selected_profile, "Applied");
    assert!(controller.unresolved_profile_signals.is_empty());
    // The watch-list was rebuilt from the profile.
    assert_eq!(
        controller.calibration_manager.watch_list_names(),
        vec!["Cal.Controller.kp_gain".to_string()]
    );

    // The default-seeding heuristic must not overwrite an applied profile.
    let before_layers = controller.settings.visualization.xy_layers.clone();
    let before_signals = controller.settings.visualization.time_plot_signals.clone();
    controller.init_default_layers_and_plots_for_model(&model);
    assert_eq!(controller.settings.visualization.xy_layers, before_layers);
    assert_eq!(
        controller.settings.visualization.time_plot_signals,
        before_signals
    );

    // An intentionally empty profile stays empty across a model (re)load.
    let empty = PlotProfile {
        name: "Empty".to_string(),
        ..PlotProfile::default()
    };
    controller.apply_profile(&empty);
    controller.init_default_layers_and_plots_for_model(&model);
    assert!(controller.settings.visualization.xy_layers.is_empty());
    assert!(controller
        .settings
        .visualization
        .time_plot_signals
        .is_empty());

    // Applying a profile whose signals are absent flags them without dropping them.
    let mut ghost = sample_profile("Ghost", &signals);
    ghost.time_plot_signals = vec!["Nope.One".to_string(), "Nope.Two".to_string()];
    controller.apply_profile(&ghost);
    assert_eq!(controller.unresolved_profile_signals.len(), 2);
    assert!(controller.is_signal_unresolved("Nope.One"));
    assert!(!controller.is_signal_unresolved(signals[0].as_str()));
    assert_eq!(
        controller.settings.visualization.time_plot_signals, ghost.time_plot_signals,
        "unresolved signals stay in the applied view"
    );
    assert!(controller.status_message.contains("not in the current A2L"));

    // With no A2L loaded, nothing can be judged unresolved.
    controller.a2l_model = None;
    controller.apply_profile(&ghost);
    assert!(controller.unresolved_profile_signals.is_empty());
}

#[test]
fn test_controller_save_load_delete_import_export_round_trip() {
    let dir = tempdir().unwrap();
    let mut controller = controller_in(&dir);
    let signals = mock_signal_names(2);
    controller.a2l_model = Some(parse_a2l(sample_adas_a2l_text()).unwrap());

    // Build a view by hand, then capture it as a profile.
    controller.settings.visualization.time_plot_signals = signals.clone();
    controller.settings.visualization.time_plot_window_s = 10.0;
    controller.plane_viewport = PlaneViewport {
        center_x_m: 1.0,
        center_y_m: 2.0,
        pixels_per_meter: 33.0,
    };
    let path = controller
        .save_current_as_profile("Session A", "first view")
        .unwrap();
    assert!(path.exists());
    assert_eq!(controller.settings.active_profile, "Session A");
    assert_eq!(controller.profile_list.len(), 1);
    assert!(controller.status_message.contains("Session A"));

    let captured = controller.capture_current_profile("Probe".to_string(), String::new());
    assert_eq!(captured.time_plot_signals, signals);
    assert_eq!(captured.viewport.unwrap().pixels_per_meter, 33.0);
    assert_eq!(captured.a2l_project, "OpenXcpAdasProject");

    // Mutate the view, then load the profile back to restore it.
    controller.settings.visualization.time_plot_signals.clear();
    controller.settings.visualization.time_plot_window_s = 120.0;
    controller.load_profile_by_name("Session A").unwrap();
    assert_eq!(controller.settings.visualization.time_plot_signals, signals);
    assert_eq!(controller.settings.visualization.time_plot_window_s, 10.0);
    assert_eq!(controller.plane_viewport.pixels_per_meter, 33.0);

    // Export, then import from an explicit path.
    let export_path = dir.path().join("shared.json");
    controller.profile_io_path = export_path.display().to_string();
    controller
        .export_current_profile_to_path(&controller.profile_io_path.clone())
        .unwrap();
    assert!(export_path.exists());
    controller
        .import_profile_from_path(&export_path.display().to_string())
        .unwrap();
    assert_eq!(controller.settings.active_profile, "Session A");

    // Error paths.
    assert!(controller.load_profile_by_name("Does Not Exist").is_err());
    assert!(controller.import_profile_from_path("   ").is_err());
    assert!(controller.export_current_profile_to_path("").is_err());
    assert!(controller
        .import_profile_from_path(&dir.path().join("nope.json").display().to_string())
        .is_err());
    assert!(controller.save_current_as_profile("///", "").is_err());

    // Exporting with no active profile derives the name from the destination file stem.
    controller.settings.active_profile.clear();
    let derived = dir.path().join("Derived_Name.json");
    controller
        .export_current_profile_to_path(&derived.display().to_string())
        .unwrap();
    let store = ProfileStore::new(dir.path());
    assert_eq!(store.load_from_path(&derived).unwrap().name, "Derived_Name");

    // Deleting the active profile clears the pointer.
    controller.settings.active_profile = "Session A".to_string();
    controller.selected_profile = "Session A".to_string();
    controller.delete_profile("Session A").unwrap();
    assert!(controller.settings.active_profile.is_empty());
    assert!(controller.selected_profile.is_empty());
    assert!(controller.profile_list.is_empty());
    assert!(controller.delete_profile("Session A").is_err());
}

#[test]
fn test_startup_auto_loads_active_profile() {
    let dir = tempdir().unwrap();
    let signals = mock_signal_names(2);
    let profiles_dir = dir.path().join("profiles");
    let settings_path = dir.path().join("settings.json");

    // Write a profile plus a settings file naming it as active.
    let store = ProfileStore::new(&profiles_dir);
    let profile = sample_profile("Restored", &signals);
    store.save(&profile).unwrap();

    let mut settings = AppSettings {
        active_profile: "Restored".to_string(),
        ..AppSettings::default()
    };
    settings.recording.default_file_prefix = "AutoLoad".to_string();
    ConfigStore::new(&settings_path).save(&settings).unwrap();

    let controller = AppController::new_with_profile_store(
        ConfigStore::new(&settings_path),
        ProfileStore::new(&profiles_dir),
    );
    assert_eq!(controller.settings.active_profile, "Restored");
    assert_eq!(controller.selected_profile, "Restored");
    assert_eq!(
        controller.settings.visualization.xy_layers,
        profile.xy_layers
    );
    assert_eq!(controller.settings.visualization.time_plot_window_s, 60.0);
    assert_eq!(controller.plane_viewport, profile.viewport.unwrap());
    assert_eq!(controller.profile_list.len(), 1);

    // A dangling active_profile pointer degrades to a status message, never a panic.
    let mut settings = AppSettings {
        active_profile: "Vanished".to_string(),
        ..AppSettings::default()
    };
    settings.recording.default_file_prefix = "Dangling".to_string();
    ConfigStore::new(&settings_path).save(&settings).unwrap();
    let controller = AppController::new_with_profile_store(
        ConfigStore::new(&settings_path),
        ProfileStore::new(&profiles_dir),
    );
    assert!(controller
        .status_message
        .contains("Could not restore plot profile 'Vanished'"));
}

#[test]
fn test_profile_panel_renders_headlessly() {
    let dir = tempdir().unwrap();
    let mut controller = controller_in(&dir);
    let signals = mock_signal_names(2);
    controller.a2l_model = Some(parse_a2l(sample_adas_a2l_text()).unwrap());

    // One valid profile and one unreadable file, so both selector branches render.
    controller
        .save_current_as_profile("Rendered", "panel fixture")
        .unwrap();
    fs::write(dir.path().join("profiles").join("Broken.json"), "{ nope").unwrap();
    controller.refresh_profile_list();
    assert_eq!(controller.profile_list.len(), 2);

    // Enough unresolved signals to exercise the preview list and the "show all" collapsible.
    let mut ghost = sample_profile("Ghosts", &signals);
    ghost.time_plot_signals = (0..8).map(|i| format!("Missing_{}", i)).collect();
    controller.apply_profile(&ghost);
    assert_eq!(controller.unresolved_profile_signals.len(), 8);

    let ctx = egui::Context::default();
    for _ in 0..2 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_profile_section(ui, &mut controller);
                // The drop-down body and the collapsed header body only render when open, so
                // drive them directly to cover both the valid and the unreadable entry.
                render_profile_selector_items(ui, &mut controller);
                render_unresolved_warning(ui, &mut controller);
                render_profile_collapsing(ui, &mut controller);
            });
        });
    }

    // The empty-directory branch of the selector.
    controller.profile_list.clear();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            render_profile_selector_items(ui, &mut controller);
        });
    });
    controller.refresh_profile_list();

    // With no active profile and nothing unresolved, the collapsed header falls back to its
    // generic title and the warning renders nothing.
    let saved_unresolved = controller.unresolved_profile_signals.clone();
    controller.settings.active_profile.clear();
    controller.unresolved_profile_signals.clear();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            render_profile_collapsing(ui, &mut controller);
            render_unresolved_warning(ui, &mut controller);
        });
    });

    // A short unresolved list renders inline instead of scrolling.
    controller.unresolved_profile_signals = ["Only_One".to_string()].into_iter().collect();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            render_unresolved_warning(ui, &mut controller);
        });
    });

    // Restore the unresolved set and give the 2D plot a layer whose X signal is missing, so the
    // ⚠ markers in both plot sidebars render.
    controller.unresolved_profile_signals = saved_unresolved;
    controller.settings.active_profile = "Ghosts".to_string();
    controller.settings.visualization.xy_layers = vec![XyPlotLayerConfig {
        name: "Ghost Layer".to_string(),
        enabled: true,
        x_signal: "Missing_0".to_string(),
        y_signal: signals[1].clone(),
        valid_count_signal: "Missing_1".to_string(),
        corner_signals: Default::default(),
        style: XyDrawStyle::Scatter,
        color_rgb: [9, 9, 9],
        point_radius: 5.0,
    }];
    controller
        .unresolved_profile_signals
        .insert("Missing_0".to_string());

    // The profile section also renders inside the Settings modal and both plot sidebars.
    let mut app = OpenXcpClientApp::new(controller);
    app.controller.settings_menu_open = true;
    for tab in [
        open_xcp_client::app_state::ActiveTab::PlanePlot,
        open_xcp_client::app_state::ActiveTab::TimePlot,
    ] {
        app.controller.active_tab = tab;
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                app.render_frame(ctx);
            });
        }
    }
    assert!(!app.controller.profile_list.is_empty());
    // The Time Plot picker lists the profile's unresolved selections, which are still selected.
    assert!(app
        .controller
        .settings
        .visualization
        .time_plot_signals
        .contains(&"Missing_0".to_string()));
}

#[test]
fn test_profile_store_io_error_paths() {
    let dir = tempdir().unwrap();
    let signals = mock_signal_names(2);
    let profile = sample_profile("Io", &signals);

    // `dir()` reports the directory the store manages.
    let store = ProfileStore::new(dir.path().join("profiles"));
    assert_eq!(store.dir(), dir.path().join("profiles").as_path());
    // The default store points at ./config/profiles.
    assert_eq!(ProfileStore::default().dir(), ProfileStore::default_dir());

    // Listing a path that is a regular file, not a directory, surfaces an I/O error.
    let file_as_dir = dir.path().join("not_a_dir");
    fs::write(&file_as_dir, "x").unwrap();
    let broken_store = ProfileStore::new(&file_as_dir);
    assert!(matches!(broken_store.list(), Err(ProfileError::Io { .. })));

    // A parent path that is a regular file makes `create_dir_all` fail.
    let under_file = file_as_dir.join("child").join("p.json");
    assert!(matches!(
        store.save_to_path(&profile, &under_file),
        Err(ProfileError::Io { .. })
    ));

    // Renaming the temp file over a non-empty directory fails, and the temp file is cleaned up.
    let busy = dir.path().join("Busy.json");
    fs::create_dir_all(&busy).unwrap();
    fs::write(busy.join("occupant"), "x").unwrap();
    assert!(matches!(
        store.save_to_path(&profile, &busy),
        Err(ProfileError::Io { .. })
    ));
    assert!(!busy.with_extension("tmp").exists());

    // Deleting a "profile" that is really a directory fails on remove_file.
    let dir_store = ProfileStore::new(dir.path());
    assert!(matches!(
        dir_store.delete("Busy"),
        Err(ProfileError::Io { .. })
    ));

    // A read-only profile directory makes the temp-file write fail.
    let locked = dir.path().join("locked");
    fs::create_dir_all(&locked).unwrap();
    let mut perms = fs::metadata(&locked).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o555);
        fs::set_permissions(&locked, perms.clone()).unwrap();
        let locked_store = ProfileStore::new(&locked);
        assert!(matches!(
            locked_store.save(&profile),
            Err(ProfileError::Io { .. })
        ));
        perms.set_mode(0o755);
        fs::set_permissions(&locked, perms).unwrap();
    }
    #[cfg(not(unix))]
    {
        let _ = perms;
    }
}

#[test]
fn test_profile_panel_actions() {
    let dir = tempdir().unwrap();
    let mut controller = controller_in(&dir);
    let signals = mock_signal_names(2);
    controller.a2l_model = Some(parse_a2l(sample_adas_a2l_text()).unwrap());
    controller.settings.visualization.time_plot_signals = signals.clone();

    // SaveAs clears the name field on success.
    controller.new_profile_name = "Panel A".to_string();
    controller.new_profile_description = "from the panel".to_string();
    apply_profile_action(&mut controller, ProfileAction::SaveAs);
    assert!(controller.new_profile_name.is_empty());
    assert_eq!(controller.settings.active_profile, "Panel A");

    // SaveAs with an unusable name reports an error and keeps the field.
    controller.new_profile_name = "///".to_string();
    apply_profile_action(&mut controller, ProfileAction::SaveAs);
    assert_eq!(controller.new_profile_name, "///");
    assert!(controller.status_message.contains("Could not save profile"));

    // Refresh re-scans the directory.
    apply_profile_action(&mut controller, ProfileAction::Refresh);
    assert_eq!(controller.profile_list.len(), 1);

    // Load restores the saved view.
    controller.settings.visualization.time_plot_signals.clear();
    controller.selected_profile = "Panel A".to_string();
    apply_profile_action(&mut controller, ProfileAction::Load);
    assert_eq!(controller.settings.visualization.time_plot_signals, signals);

    // Load of a missing profile reports an error.
    controller.selected_profile = "Nope".to_string();
    apply_profile_action(&mut controller, ProfileAction::Load);
    assert!(controller
        .status_message
        .contains("Could not load profile 'Nope'"));

    // Update overwrites the selected profile, picking up its stored description.
    controller.selected_profile = "Panel A".to_string();
    controller.settings.visualization.time_plot_window_s = 120.0;
    apply_profile_action(&mut controller, ProfileAction::Update);
    assert_eq!(
        controller
            .profile_store
            .load("Panel A")
            .unwrap()
            .time_plot_window_s,
        120.0
    );
    assert_eq!(
        controller
            .profile_store
            .load("Panel A")
            .unwrap()
            .description,
        "from the panel"
    );

    // Update of an unusable name reports an error.
    controller.selected_profile = "///".to_string();
    apply_profile_action(&mut controller, ProfileAction::Update);
    assert!(controller
        .status_message
        .contains("Could not update profile"));

    // Export then Import through the shared path field.
    controller.profile_io_path = dir.path().join("panel.json").display().to_string();
    apply_profile_action(&mut controller, ProfileAction::Export);
    assert!(dir.path().join("panel.json").exists());
    apply_profile_action(&mut controller, ProfileAction::Import);
    assert!(!controller.status_message.contains("Could not import"));

    // Both report errors on an empty path.
    controller.profile_io_path.clear();
    apply_profile_action(&mut controller, ProfileAction::Export);
    assert!(controller
        .status_message
        .contains("Could not export profile"));
    apply_profile_action(&mut controller, ProfileAction::Import);
    assert!(controller
        .status_message
        .contains("Could not import profile"));

    // Delete removes the file; deleting again reports an error.
    controller.selected_profile = "Panel A".to_string();
    apply_profile_action(&mut controller, ProfileAction::Delete);
    apply_profile_action(&mut controller, ProfileAction::Delete);
    assert!(controller
        .status_message
        .contains("Could not delete profile"));
}

#[test]
fn test_profile_error_reporting_paths() {
    let dir = tempdir().unwrap();
    let signals = mock_signal_names(2);

    // A settings path that cannot be written (its parent is a regular file) makes every
    // persist attempt report into the status bar instead of failing silently.
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "x").unwrap();
    let mut controller = AppController::new_with_profile_store(
        ConfigStore::new(blocker.join("nested").join("settings.json")),
        ProfileStore::new(dir.path().join("profiles")),
    );
    controller.a2l_model = Some(parse_a2l(sample_adas_a2l_text()).unwrap());

    controller.persist_visualization();
    assert!(
        controller
            .status_message
            .contains("Could not persist plot settings"),
        "unexpected: {}",
        controller.status_message
    );

    // apply_profile reports the same failure and leaves the status bar on the error.
    controller.apply_profile(&sample_profile("Unwritable", &signals));
    assert!(
        controller
            .status_message
            .contains("Could not persist active profile"),
        "unexpected: {}",
        controller.status_message
    );

    // A profile directory that is really a file makes listing report an error and clear the list.
    let mut controller = AppController::new_with_profile_store(
        ConfigStore::new(dir.path().join("settings.json")),
        ProfileStore::new(&blocker),
    );
    controller.profile_list = vec![ProfileSummary {
        name: "stale".to_string(),
        path: blocker.clone(),
        description: String::new(),
        layer_count: 0,
        time_plot_signal_count: 0,
        invalid_reason: None,
    }];
    controller.refresh_profile_list();
    assert!(controller.profile_list.is_empty());
    assert!(
        controller
            .status_message
            .contains("Could not list plot profiles"),
        "unexpected: {}",
        controller.status_message
    );

    // Deleting a profile that is not the active one leaves the active pointer alone.
    let mut controller = controller_in(&dir);
    controller.a2l_model = Some(parse_a2l(sample_adas_a2l_text()).unwrap());
    controller
        .save_current_as_profile("Keeper", "stays active")
        .unwrap();
    controller
        .profile_store
        .save(&sample_profile("Doomed", &signals))
        .unwrap();
    controller.refresh_profile_list();
    controller.selected_profile = "Keeper".to_string();
    controller.delete_profile("Doomed").unwrap();
    assert_eq!(controller.settings.active_profile, "Keeper");
    assert_eq!(controller.selected_profile, "Keeper");
    assert_eq!(controller.profile_list.len(), 1);

    // Exporting while a profile is active reuses its name rather than the file stem.
    let out = dir.path().join("renamed.json");
    controller
        .export_current_profile_to_path(&out.display().to_string())
        .unwrap();
    assert_eq!(
        controller.profile_store.load_from_path(&out).unwrap().name,
        "Keeper"
    );
}
