//! Modal "Save EQ preset as…" dialog.
//!
//! Triggered from the EQ window's PRESETS dropdown when the user
//! picks the "Save as preset…" row. Asks for a free-form name and
//! returns it to the app, which then writes the current band curve +
//! preamp into the `PresetManager` and flushes the JSON store.
//!
//! Why a dedicated dialog and not the file-save path that "Save as
//! .eqf…" already uses: .eqf is a single-preset binary format meant
//! for cross-application exchange (Winamp / Audacious / DeaDBeeF).
//! User presets live alongside the built-ins in one JSON store so
//! they appear in the dropdown without a per-file pick — quicker
//! recall, fewer files to manage.

use crate::dialog_util::{ConfirmChoice, DialogOutcome, DialogView, confirm_buttons};
use egui::Key;

pub type Outcome = DialogOutcome<String>;

pub struct PresetNameDialog {
    pub name: String,
    pub error: Option<String>,
    last_size_sent: Option<egui::Vec2>,
}

impl PresetNameDialog {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            error: None,
            last_size_sent: None,
        }
    }
}

impl DialogView for PresetNameDialog {
    type Payload = String;

    fn viewport_hash(&self) -> &'static str {
        "oneamp_preset_name_dialog"
    }

    fn title(&self) -> String {
        "OneAmp — Save EQ preset".to_string()
    }

    fn target_width(&self) -> f32 {
        420.0
    }

    fn initial_height(&self) -> f32 {
        140.0
    }

    fn last_size_sent(&mut self) -> &mut Option<egui::Vec2> {
        &mut self.last_size_sent
    }

    fn render_body(&mut self, ui: &mut egui::Ui, outcome: &mut Outcome) {
        ui.label("Preset name:");
        let avail = ui.available_width();
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.name)
                .desired_width(avail)
                .hint_text("e.g. Bedroom Bass"),
        );
        let enter_pressed = edit.lost_focus() && ui.ctx().input(|i| i.key_pressed(Key::Enter));

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            match confirm_buttons(ui, "Save") {
                ConfirmChoice::Accept if !self.name.trim().is_empty() => {
                    *outcome = DialogOutcome::Accepted(self.name.trim().to_string());
                }
                ConfirmChoice::Cancel => *outcome = DialogOutcome::Cancelled,
                _ => {}
            }
            if enter_pressed && !self.name.trim().is_empty() {
                *outcome = DialogOutcome::Accepted(self.name.trim().to_string());
            }
        });
    }
}
