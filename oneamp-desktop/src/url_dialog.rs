//! Modal "Open URL…" dialog (`Ctrl+L` / playlist context menu).
//!
//! Asks for an HTTP(S) URL and reports it back to the app, which then
//! pushes a `Play(url-path)`-equivalent into the audio engine and
//! appends the stream to the playlist as a `file:` -less entry.
//!
//! Why a dedicated dialog and not `rfd::FileDialog`: `rfd` is built
//! for picking *local* paths. We need a free-form text field, not a
//! filesystem browser.

use crate::dialog_util::{ConfirmChoice, DialogOutcome, DialogView, confirm_buttons};
use egui::Key;

/// Concrete outcome alias — `Accepted(String)` carries the trimmed
/// URL the user submitted (scheme validation happens at the call
/// site so paste-and-go still works for whitespace input here).
pub type Outcome = DialogOutcome<String>;

pub struct UrlDialog {
    pub url: String,
    pub error: Option<String>,
    /// Last viewport size we requested via `InnerSize`. Used by the
    /// auto-shrink pass at the end of every frame to avoid re-sending
    /// the same resize command and avoid a tug-of-war with the OS
    /// when the content size hasn't actually changed.
    last_size_sent: Option<egui::Vec2>,
}

impl UrlDialog {
    pub fn new() -> Self {
        Self {
            url: String::new(),
            error: None,
            last_size_sent: None,
        }
    }
}

impl DialogView for UrlDialog {
    type Payload = String;

    fn viewport_hash(&self) -> &'static str {
        "oneamp_url_dialog"
    }

    fn title(&self) -> String {
        "OneAmp — Open URL".to_string()
    }

    // Width: roomy enough for a real radio URL without horizontal scrolling.
    fn target_width(&self) -> f32 {
        480.0
    }

    // Just a starting guess — fit_viewport_height re-sizes on the
    // first frame to the actual content extent.
    fn initial_height(&self) -> f32 {
        130.0
    }

    fn last_size_sent(&mut self) -> &mut Option<egui::Vec2> {
        &mut self.last_size_sent
    }

    fn render_body(&mut self, ui: &mut egui::Ui, outcome: &mut Outcome) {
        ui.label("Internet radio / podcast URL:");
        let avail = ui.available_width();
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.url)
                .desired_width(avail)
                .hint_text("https://example.com/stream.mp3"),
        );
        // Enter on the text field is the same as clicking Open.
        let enter_pressed = edit.lost_focus() && ui.ctx().input(|i| i.key_pressed(Key::Enter));

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            match confirm_buttons(ui, "Open") {
                ConfirmChoice::Accept if !self.url.trim().is_empty() => {
                    *outcome = DialogOutcome::Accepted(self.url.trim().to_string());
                }
                ConfirmChoice::Cancel => *outcome = DialogOutcome::Cancelled,
                _ => {}
            }
            // Pressing Enter inside the URL text field is the same as
            // hitting Open. Not handled by `confirm_buttons` because
            // text fields legitimately consume Enter, so we gate this
            // on the field having had focus this frame.
            if enter_pressed && !self.url.trim().is_empty() {
                *outcome = DialogOutcome::Accepted(self.url.trim().to_string());
            }
        });
    }
}
