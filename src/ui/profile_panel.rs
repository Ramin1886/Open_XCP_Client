// SPDX-License-Identifier: GPL-2.0-only
//! Plot profile selector and editor (`src/ui/profile_panel.rs`).
//!
//! Renders the reusable `Plot Profiles` section — a selector over the JSON files in
//! `./config/profiles/`, plus `Load`, `Update`, `Delete`, `Save As…`, `Import` and `Export`
//! actions and the unresolved-signal warning. The same renderer is embedded in the
//! `⚙ Settings` modal and, as a collapsed header, in the `2D Live Plot` and `Time Plot`
//! sidebars so switching views never requires opening the modal.
//!
//! The widgets only translate clicks into a [`ProfileAction`]; every state change goes through
//! [`apply_profile_action`], which keeps the behavior independent of the egui layout.

use crate::app_state::AppController;
use egui::{Color32, RichText, Ui};

/// Number of unresolved signal names listed before the rest are collapsed.
const UNRESOLVED_PREVIEW_COUNT: usize = 5;

/// Amber used to mark signals a loaded profile references that the current A2L does not define.
pub const UNRESOLVED_COLOR: Color32 = Color32::from_rgb(251, 191, 36);

/// Buttons acting on the profile chosen in the selector, disabled while nothing is selected.
const SELECTION_ACTIONS: [(&str, &str, ProfileAction); 3] = [
    (
        "📂 Load",
        "Apply the selected profile to all plot tabs",
        ProfileAction::Load,
    ),
    (
        "🔁 Update",
        "Overwrite the selected profile with the current view",
        ProfileAction::Update,
    ),
    (
        "🗑 Delete",
        "Delete the selected profile file",
        ProfileAction::Delete,
    ),
];

/// Buttons acting on the shared Import/Export path field.
const IO_ACTIONS: [(&str, &str, ProfileAction); 2] = [
    (
        "⤵ Import",
        "Load a profile file from this path and copy it into ./config/profiles/",
        ProfileAction::Import,
    ),
    (
        "⤴ Export",
        "Write the current view to this path so it can be shared",
        ProfileAction::Export,
    ),
];

/// One user action available in the `Plot Profiles` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAction {
    /// Re-scan the profile directory.
    Refresh,
    /// Apply the profile currently chosen in the selector.
    Load,
    /// Overwrite the chosen profile with the current view.
    Update,
    /// Delete the chosen profile file.
    Delete,
    /// Save the current view under the name typed into the `New Profile Name` field.
    SaveAs,
    /// Read a profile from the Import/Export path and apply it.
    Import,
    /// Write the current view to the Import/Export path.
    Export,
}

/// Executes `action` against `controller`, reporting any failure in the status bar.
pub fn apply_profile_action(controller: &mut AppController, action: ProfileAction) {
    match action {
        ProfileAction::Refresh => controller.refresh_profile_list(),
        ProfileAction::Load => {
            let name = controller.selected_profile.clone();
            if let Err(err) = controller.load_profile_by_name(&name) {
                controller.status_message = format!("Could not load profile '{}': {}", name, err);
            }
        }
        ProfileAction::Update => {
            let name = controller.selected_profile.clone();
            let description = controller
                .profile_list
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.description.clone())
                .unwrap_or_default();
            if let Err(err) = controller.save_current_as_profile(&name, &description) {
                controller.status_message = format!("Could not update profile '{}': {}", name, err);
            }
        }
        ProfileAction::Delete => {
            let name = controller.selected_profile.clone();
            if let Err(err) = controller.delete_profile(&name) {
                controller.status_message = format!("Could not delete profile '{}': {}", name, err);
            }
        }
        ProfileAction::SaveAs => {
            let name = controller.new_profile_name.clone();
            let description = controller.new_profile_description.clone();
            match controller.save_current_as_profile(&name, &description) {
                Ok(_) => controller.new_profile_name.clear(),
                Err(err) => {
                    controller.status_message =
                        format!("Could not save profile '{}': {}", name, err);
                }
            }
        }
        ProfileAction::Import => {
            let path = controller.profile_io_path.clone();
            if let Err(err) = controller.import_profile_from_path(&path) {
                controller.status_message = format!("Could not import profile: {}", err);
            }
        }
        ProfileAction::Export => {
            let path = controller.profile_io_path.clone();
            if let Err(err) = controller.export_current_profile_to_path(&path) {
                controller.status_message = format!("Could not export profile: {}", err);
            }
        }
    }
}

/// Renders the entries of the profile selector drop-down.
///
/// Unreadable files appear disabled, so a corrupt profile is visible without being selectable.
pub fn render_profile_selector_items(ui: &mut Ui, controller: &mut AppController) {
    if controller.profile_list.is_empty() {
        ui.weak("No profiles in ./config/profiles/ yet");
        return;
    }
    for summary in controller.profile_list.clone() {
        if summary.is_invalid() {
            ui.add_enabled(false, egui::Button::new(summary.label()));
            continue;
        }
        ui.selectable_value(
            &mut controller.selected_profile,
            summary.name.clone(),
            summary.label(),
        );
    }
}

/// Renders the `⚠` warning listing profile signals absent from the currently loaded A2L.
pub fn render_unresolved_warning(ui: &mut Ui, controller: &mut AppController) {
    let unresolved: Vec<String> = controller
        .unresolved_profile_signals
        .iter()
        .cloned()
        .collect();
    if unresolved.is_empty() {
        return;
    }

    ui.separator();
    ui.label(
        RichText::new(format!(
            "⚠ {} signal(s) in this profile are not in the current A2L (kept in the profile)",
            unresolved.len()
        ))
        .color(UNRESOLVED_COLOR)
        .strong(),
    );
    if unresolved.len() <= UNRESOLVED_PREVIEW_COUNT {
        render_signal_name_list(ui, &unresolved);
        return;
    }
    // Longer lists scroll in place rather than hiding behind a click, so the whole set is
    // always visible to the operator.
    egui::ScrollArea::vertical()
        .id_salt("unresolved_profile_signals_list")
        .max_height(120.0)
        .show(ui, |ui| render_signal_name_list(ui, &unresolved));
}

/// Renders `names` as a monospace list.
pub fn render_signal_name_list(ui: &mut Ui, names: &[String]) {
    for name in names {
        ui.monospace(name);
    }
}

/// Renders the full `Plot Profiles` section into `ui`.
pub fn render_profile_section(ui: &mut Ui, controller: &mut AppController) {
    let mut action: Option<ProfileAction> = None;

    ui.horizontal(|ui| {
        ui.label("Profile:");
        let selected_label = if controller.selected_profile.is_empty() {
            "<none>".to_string()
        } else {
            controller.selected_profile.clone()
        };
        egui::ComboBox::from_id_salt("plot_profile_selector")
            .selected_text(selected_label)
            .width(240.0)
            .show_ui(ui, |ui| render_profile_selector_items(ui, controller));
        if ui
            .button("🔄")
            .on_hover_text("Re-scan ./config/profiles/")
            .clicked()
        {
            action = Some(ProfileAction::Refresh);
        }
    });

    let has_selection = !controller.selected_profile.trim().is_empty();
    ui.horizontal(|ui| {
        for (label, hover, act) in SELECTION_ACTIONS {
            if ui
                .add_enabled(has_selection, egui::Button::new(label))
                .on_hover_text(hover)
                .clicked()
            {
                action = Some(act);
            }
        }
    });

    ui.separator();
    egui::Grid::new("plot_profile_save_as_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("New Profile Name:");
            ui.text_edit_singleline(&mut controller.new_profile_name);
            ui.end_row();

            ui.label("Description:");
            ui.text_edit_singleline(&mut controller.new_profile_description);
            ui.end_row();
        });
    let can_save = !controller.new_profile_name.trim().is_empty();
    if ui
        .add_enabled(
            can_save,
            egui::Button::new(RichText::new("💾 Save Current View As Profile").strong()),
        )
        .on_hover_text(
            "Write the current 2D layers, Time Plot selection, Ego geometry, viewport and \
             calibration watch-list to ./config/profiles/",
        )
        .clicked()
    {
        action = Some(ProfileAction::SaveAs);
    }

    ui.separator();
    ui.label("Import / Export Path (.json):");
    ui.text_edit_singleline(&mut controller.profile_io_path);
    ui.horizontal(|ui| {
        for (label, hover, act) in IO_ACTIONS {
            if ui.button(label).on_hover_text(hover).clicked() {
                action = Some(act);
            }
        }
    });

    render_unresolved_warning(ui, controller);

    if let Some(action) = action {
        apply_profile_action(controller, action);
    }
}

/// Renders the `Plot Profiles` section as a collapsed header, for embedding in a plot sidebar.
pub fn render_profile_collapsing(ui: &mut Ui, controller: &mut AppController) {
    let title = if controller.settings.active_profile.trim().is_empty() {
        "📁 Plot Profiles".to_string()
    } else {
        format!("📁 Plot Profile: {}", controller.settings.active_profile)
    };
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt("plot_profile_collapsing")
        .default_open(false)
        .show(ui, |ui| render_profile_section(ui, controller));
}
