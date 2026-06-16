//! Modal dialog for editing `AppConfig::playlist_display_format`.
//!
//! Surface the same template tokens the formatter understands in a
//! help block at the bottom of the dialog, so the user can experiment
//! without consulting docs.

use crate::dialog_util::{ConfirmChoice, DialogOutcome, DialogView, confirm_buttons};
use egui::Key;

/// Concrete outcome alias so call-sites can stay readable —
/// `Accepted(String)` carries the new template the app overwrites
/// `config.playlist_display_format` with.
pub type Outcome = DialogOutcome<String>;

pub struct FormatDialog {
    pub template: String,
    /// Last viewport size we asked the OS for via `InnerSize`. See
    /// `dialog_util::fit_viewport_height` — caches the request so we
    /// don't re-send it every frame.
    last_size_sent: Option<egui::Vec2>,
}

impl FormatDialog {
    pub fn new(current: &str) -> Self {
        Self {
            template: current.to_string(),
            last_size_sent: None,
        }
    }
}

impl DialogView for FormatDialog {
    type Payload = String;

    fn viewport_hash(&self) -> &'static str {
        "oneamp_format_dialog"
    }

    fn title(&self) -> String {
        "OneAmp — Playlist display format".to_string()
    }

    // Width: wide enough for the monospace "{artist} {title} …"
    // cheat sheet to fit on one row.
    fn target_width(&self) -> f32 {
        580.0
    }

    // Rough starting guess — fit_viewport_height re-sizes on first frame.
    fn initial_height(&self) -> f32 {
        180.0
    }

    fn last_size_sent(&mut self) -> &mut Option<egui::Vec2> {
        &mut self.last_size_sent
    }

    fn render_body(&mut self, ui: &mut egui::Ui, outcome: &mut Outcome) {
        ui.label("Template:");
        let avail = ui.available_width();
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.template)
                .desired_width(avail)
                .hint_text("{artist} - {title}"),
        );
        let enter_pressed = edit.lost_focus() && ui.ctx().input(|i| i.key_pressed(Key::Enter));

        ui.add_space(6.0);
        ui.label("Available tokens:");
        ui.monospace("{artist} {title} {album} {genre} {tracknumber} {year} {duration} {filename}");
        ui.add_space(4.0);
        ui.weak(
            "Missing tags collapse separators automatically — e.g. \"{artist} - {title}\" \
             against an untagged file falls back to the filename.",
        );

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            // "Reset to default" is a side action — render it first so
            // it sits at the leading edge while the OS-conventional
            // Cancel/Save pair stays pinned to the trailing edge.
            if ui.button("Reset to default").clicked() {
                self.template = crate::config::default_playlist_format();
            }
            match confirm_buttons(ui, "Save") {
                ConfirmChoice::Accept => {
                    *outcome = DialogOutcome::Accepted(self.template.clone());
                }
                ConfirmChoice::Cancel => *outcome = DialogOutcome::Cancelled,
                _ => {}
            }
            if enter_pressed {
                *outcome = DialogOutcome::Accepted(self.template.clone());
            }
        });
    }
}
