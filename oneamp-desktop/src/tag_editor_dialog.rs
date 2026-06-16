//! Modal egui dialog for editing tags on a single playlist entry.
//!
//! Opened from the playlist's right-click context menu. The dialog
//! lazily reads the file's tags on construction (via
//! `oneamp_core::tag_editor::EditableTags::read`) and writes them back
//! when the user clicks Save. Errors during read/write surface as a
//! red message inside the dialog so the user keeps the rest of the
//! player usable — no global error banner.

use crate::dialog_util::{ConfirmChoice, DialogOutcome, DialogView, confirm_buttons};
use oneamp_core::tag_editor::EditableTags;
use std::path::PathBuf;

/// Concrete outcome alias — `Accepted(usize)` returns the playlist
/// row index so the app can refresh the cached display name for
/// that exact entry without restarting.
pub type Outcome = DialogOutcome<usize>;

/// In-flight tag editor state. The dialog is opened with a path, all
/// the displayed strings are kept locally so the user can experiment
/// without touching the file until Save.
pub struct TagEditorDialog {
    /// File being edited. Echoed in the dialog header so the user can
    /// confirm they're working on the right track when they have
    /// duplicates of the same title.
    pub path: PathBuf,
    /// Playlist row index — bubbled back through `Outcome::Accepted(idx)`
    /// so the app can refresh that exact entry's cached display name.
    pub playlist_index: usize,

    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: String,
    pub tracknumber: String,
    pub comment: String,

    /// Last-error message rendered in red at the bottom of the dialog.
    /// Set when reading the file fails (the dialog opens with empty
    /// fields plus the error) or when saving fails (the dialog stays
    /// open so the user can adjust their input and retry).
    pub error: Option<String>,

    /// Last viewport size we asked the OS for via `InnerSize`. See
    /// `dialog_util::fit_viewport_height` — caches the request so we
    /// don't re-send it every frame.
    last_size_sent: Option<egui::Vec2>,
}

impl TagEditorDialog {
    /// Open the dialog for `path`, pre-filling fields from the file's
    /// current tags. Read failures don't abort — we open with empty
    /// fields plus an error banner so the user can still tag a file
    /// from scratch.
    pub fn open(playlist_index: usize, path: PathBuf) -> Self {
        let (tags, err) = match EditableTags::read(&path) {
            Ok(t) => (t, None),
            Err(e) => (EditableTags::default(), Some(format!("Read failed: {}", e))),
        };
        Self {
            path,
            playlist_index,
            title: tags.title.unwrap_or_default(),
            artist: tags.artist.unwrap_or_default(),
            album: tags.album.unwrap_or_default(),
            album_artist: tags.album_artist.unwrap_or_default(),
            genre: tags.genre.unwrap_or_default(),
            year: tags.year.map(|y| y.to_string()).unwrap_or_default(),
            tracknumber: tags.tracknumber.map(|n| n.to_string()).unwrap_or_default(),
            comment: tags.comment.unwrap_or_default(),
            error: err,
            last_size_sent: None,
        }
    }

    /// Convert the form back into an `EditableTags`. Empty strings map
    /// to `None` — the editor's "clear this field" gesture is just
    /// blanking the text box. Year and track number parse leniently:
    /// non-numeric garbage clears the field rather than rejecting the
    /// save.
    fn snapshot(&self) -> EditableTags {
        fn opt(s: &str) -> Option<String> {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        EditableTags {
            title: opt(&self.title),
            artist: opt(&self.artist),
            album: opt(&self.album),
            album_artist: opt(&self.album_artist),
            genre: opt(&self.genre),
            year: self.year.trim().parse().ok(),
            tracknumber: self.tracknumber.trim().parse().ok(),
            comment: opt(&self.comment),
        }
    }
}

impl DialogView for TagEditorDialog {
    type Payload = usize;

    fn viewport_hash(&self) -> &'static str {
        "oneamp_tag_editor"
    }

    fn title(&self) -> String {
        let filename = self.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        format!("OneAmp — Edit tags — {}", filename)
    }

    // Width: comfortable for the longest single-line field (~300 px
    // text edit + label column + spacing + 2 × margin).
    fn target_width(&self) -> f32 {
        440.0
    }

    // Rough first guess — `fit_viewport_height` re-sizes on the
    // first frame to the actual content extent.
    fn initial_height(&self) -> f32 {
        300.0
    }

    fn last_size_sent(&mut self) -> &mut Option<egui::Vec2> {
        &mut self.last_size_sent
    }

    fn render_body(&mut self, ui: &mut egui::Ui, outcome: &mut Outcome) {
        egui::Grid::new("tag_editor_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Title");
                ui.add(egui::TextEdit::singleline(&mut self.title).desired_width(300.0));
                ui.end_row();

                ui.label("Artist");
                ui.add(egui::TextEdit::singleline(&mut self.artist).desired_width(300.0));
                ui.end_row();

                ui.label("Album");
                ui.add(egui::TextEdit::singleline(&mut self.album).desired_width(300.0));
                ui.end_row();

                ui.label("Album artist");
                ui.add(egui::TextEdit::singleline(&mut self.album_artist).desired_width(300.0));
                ui.end_row();

                ui.label("Genre");
                ui.add(egui::TextEdit::singleline(&mut self.genre).desired_width(300.0));
                ui.end_row();

                ui.label("Year");
                ui.add(egui::TextEdit::singleline(&mut self.year).desired_width(80.0));
                ui.end_row();

                ui.label("Track #");
                ui.add(egui::TextEdit::singleline(&mut self.tracknumber).desired_width(80.0));
                ui.end_row();

                ui.label("Comment");
                ui.add(
                    egui::TextEdit::multiline(&mut self.comment)
                        .desired_width(300.0)
                        .desired_rows(2),
                );
                ui.end_row();
            });

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| match confirm_buttons(ui, "Save") {
            ConfirmChoice::Accept => {
                let snap = self.snapshot();
                match snap.write(&self.path) {
                    Ok(()) => {
                        *outcome = DialogOutcome::Accepted(self.playlist_index);
                    }
                    Err(e) => {
                        self.error = Some(format!("Write failed: {}", e));
                    }
                }
            }
            ConfirmChoice::Cancel => *outcome = DialogOutcome::Cancelled,
            ConfirmChoice::None => {}
        });
    }
}
