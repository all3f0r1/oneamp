pub mod components;
pub mod cursor;
pub mod equalizer_window;
pub mod main_window;
pub mod playlist_window;
pub mod renderer;
pub mod shade_window;
pub mod skin_theme;
pub mod texture_cache;

pub use equalizer_window::EqualizerWindow;
pub use main_window::WszMainWindow;
pub use playlist_window::PlaylistWindow;
pub use shade_window::ShadeWindow;

/// Hover hint for a skinned control: the sprites only say "EJECT", "CROP",
/// "O A I D V", so each window registers one per clickable area.
/// `Sense::hover()` and registered after the window's widgets, so it shows
/// over them without taking clicks. Delay handled here: egui's
/// `on_hover_text` never opened in these windows.
pub(crate) fn tip(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    // Position in the id: two areas may share a text (logo and `O`).
    let id = ui
        .id()
        .with(("tip", text, rect.min.x as i32, rect.min.y as i32));
    let r = ui.interact(rect, id, egui::Sense::hover());
    if !r.hovered() {
        return;
    }
    let (still, since_click, down) = ui.ctx().input(|i| {
        (
            i.pointer.time_since_last_movement(),
            i.pointer.time_since_last_click(),
            i.pointer.any_down(),
        )
    });
    // Hidden while pressing, and after a click until the pointer moves.
    if down || since_click < still {
        return;
    }
    let delay = ui.style().interaction.tooltip_delay;
    if still < delay {
        ui.ctx().request_repaint_after_secs(delay - still);
        return;
    }
    egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), r.id, |ui| {
        ui.label(text);
    });
}
