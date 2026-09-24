//! Winamp's "Jump to file" box (`J` / `F3`): type a few words, pick a
//! match with the arrow keys, then play it (`Enter`) or queue it
//! (`Shift+Enter`). Unlike the Ctrl+F filter it never changes what the
//! playlist window shows.

use crate::dialog_util::{DialogOutcome, DialogView};
use egui::Key;

/// What the user picked: a real playlist index, and whether to queue it
/// instead of playing it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpChoice {
    pub index: usize,
    pub queue: bool,
}

pub type Outcome = DialogOutcome<JumpChoice>;

pub struct JumpDialog {
    query: String,
    /// Snapshot of `(playlist index, row label)` taken when opened.
    rows: Vec<(usize, String)>,
    /// Lowercased labels, parallel to `rows`, for matching.
    haystacks: Vec<String>,
    /// Position of the highlighted row inside the current matches.
    highlighted: usize,
    focus_pending: bool,
    last_size_sent: Option<egui::Vec2>,
}

/// Rows whose label contains every whitespace-separated word of `query`,
/// case-insensitively. Empty query = every row.
pub fn matching(haystacks: &[String], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    haystacks
        .iter()
        .enumerate()
        .filter(|(_, h)| words.iter().all(|w| h.contains(w.as_str())))
        .map(|(i, _)| i)
        .collect()
}

impl JumpDialog {
    pub fn new(rows: Vec<(usize, String)>, current_row: Option<usize>) -> Self {
        let haystacks = rows.iter().map(|(_, s)| s.to_lowercase()).collect();
        let highlighted = current_row
            .and_then(|c| rows.iter().position(|(i, _)| *i == c))
            .unwrap_or(0);
        Self {
            query: String::new(),
            rows,
            haystacks,
            highlighted,
            focus_pending: true,
            last_size_sent: None,
        }
    }
}

impl DialogView for JumpDialog {
    type Payload = JumpChoice;

    fn viewport_hash(&self) -> &'static str {
        "oneamp_jump_dialog"
    }

    fn title(&self) -> String {
        "OneAmp — Jump to file".to_string()
    }

    fn target_width(&self) -> f32 {
        420.0
    }

    fn initial_height(&self) -> f32 {
        360.0
    }

    fn last_size_sent(&mut self) -> &mut Option<egui::Vec2> {
        &mut self.last_size_sent
    }

    fn render_body(&mut self, ui: &mut egui::Ui, outcome: &mut Outcome) {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .desired_width(ui.available_width())
                .hint_text("Type part of a title, artist or file name"),
        );
        if self.focus_pending {
            edit.request_focus();
            self.focus_pending = false;
        }
        if edit.changed() {
            self.highlighted = 0;
        }

        let matches = matching(&self.haystacks, &self.query);
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            *outcome = DialogOutcome::Cancelled;
            return;
        }
        let (up, down, page_up, page_down, enter, shift) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::PageUp),
                i.key_pressed(Key::PageDown),
                i.key_pressed(Key::Enter),
                i.modifiers.shift,
            )
        });
        let last = matches.len().saturating_sub(1);
        if up {
            self.highlighted = self.highlighted.saturating_sub(1);
        }
        if down {
            self.highlighted = (self.highlighted + 1).min(last);
        }
        if page_up {
            self.highlighted = self.highlighted.saturating_sub(10);
        }
        if page_down {
            self.highlighted = (self.highlighted + 10).min(last);
        }
        self.highlighted = self.highlighted.min(last);

        let mut choice: Option<(usize, bool)> = None;

        ui.add_space(4.0);
        let mut clicked: Option<(usize, bool)> = None;
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (pos, &row) in matches.iter().enumerate() {
                    let (index, label) = &self.rows[row];
                    let on = pos == self.highlighted;
                    let r = ui.selectable_label(on, format!("{}. {}", index + 1, label));
                    if on && (up || down || page_up || page_down) {
                        r.scroll_to_me(None);
                    }
                    if r.double_clicked() {
                        clicked = Some((row, false));
                    } else if r.clicked() {
                        self.highlighted = pos;
                    }
                }
            });
        if matches.is_empty() {
            ui.label("No match.");
        }

        let selected_row = matches.get(self.highlighted).copied();
        if clicked.is_some() {
            choice = clicked;
        }
        if enter && let Some(row) = selected_row {
            choice = Some((row, shift));
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let has = selected_row.is_some();
            if ui.add_enabled(has, egui::Button::new("Play")).clicked()
                && let Some(row) = selected_row
            {
                choice = Some((row, false));
            }
            if ui
                .add_enabled(has, egui::Button::new("Queue (Shift+Enter)"))
                .clicked()
                && let Some(row) = selected_row
            {
                choice = Some((row, true));
            }
            if ui.button("Close").clicked() {
                *outcome = DialogOutcome::Cancelled;
            }
        });
        if let Some((row, queue)) = choice
            && let Some(&(index, _)) = self.rows.get(row)
        {
            *outcome = DialogOutcome::Accepted(JumpChoice { index, queue });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matching;

    #[test]
    fn every_word_must_match_case_insensitively() {
        let rows: Vec<String> = ["Daft Punk - One More Time", "Punk Rock", "Air - La Femme"]
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        assert_eq!(matching(&rows, "punk"), vec![0, 1]);
        assert_eq!(matching(&rows, "PUNK time"), vec![0]);
        assert_eq!(matching(&rows, ""), vec![0, 1, 2]);
        assert!(matching(&rows, "zzz").is_empty());
    }
}
