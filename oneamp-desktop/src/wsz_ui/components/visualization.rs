use egui::{Color32, Pos2, Vec2};

use super::super::renderer::WszRenderer;
use super::bitmap_font;
use oneamp_core::wsz::skin::SkinComponent;
use oneamp_core::wsz::viscolor::VisColors;

/// Convert a `[u8; 3]` from `viscolor.txt` to an opaque egui color.
fn vis_color32(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Oscilloscope render style (Winamp parity — its scope cycled through
/// these three looks). `Solid` is OneAmp's historical min/max fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OscilloscopeStyle {
    /// Connect successive samples with vertical line segments.
    Lines,
    /// One lit cell at each column's center sample.
    Dots,
    /// Filled min/max envelope strip.
    #[default]
    Solid,
}

/// Spectrum peak-fall speed (Winamp's configurable falloff). The value
/// is the normalized fraction of the 16-cell height the peak drops per
/// second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FalloffSpeed {
    Slow,
    #[default]
    Medium,
    Fast,
}

impl FalloffSpeed {
    /// Cells-per-second the peak indicator falls (out of the 16-cell
    /// column). Medium ≈ the classic ~1 s full-height drop.
    pub fn rate(self) -> f32 {
        match self {
            FalloffSpeed::Slow => 8.0,
            FalloffSpeed::Medium => 16.0,
            FalloffSpeed::Fast => 32.0,
        }
    }
}

/// User-tunable visualizer options (Audio/View menu). Mirrors the knobs
/// Winamp exposed for its analyzer and oscilloscope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VisualizerOptions {
    /// Whether the spectrum analyzer floats a peak-hold marker.
    pub spectrum_peak_hold: bool,
    /// Spectrum peak-fall speed.
    pub spectrum_falloff: FalloffSpeed,
    /// Oscilloscope render style.
    pub oscilloscope_style: OscilloscopeStyle,
}

impl Default for VisualizerOptions {
    fn default() -> Self {
        Self {
            spectrum_peak_hold: true,
            spectrum_falloff: FalloffSpeed::default(),
            oscilloscope_style: OscilloscopeStyle::default(),
        }
    }
}

pub struct SpectrumAnalyzer {
    position: (u32, u32),
    bins: [f32; 16],
    /// Per-bar peak hold: the highest amplitude reached recently. Falls
    /// gradually so it appears to "float" above the bar like Winamp.
    peaks: [f32; 16],
    /// Last instant `update` ran — used to time peak decay independently of
    /// audio sample rate.
    last_update: Option<std::time::Instant>,
    /// Whether the floating peak-hold marker is drawn (View → Visualizer).
    peak_hold_enabled: bool,
    /// Peak-fall speed in cells-per-second (from `FalloffSpeed::rate`).
    falloff_rate: f32,
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        Self {
            position: (24, 43),
            bins: [0.0; 16],
            peaks: [0.0; 16],
            last_update: None,
            peak_hold_enabled: true,
            falloff_rate: FalloffSpeed::Medium.rate(),
        }
    }

    /// Push the user's analyzer options (peak-hold on/off + fall speed).
    pub fn set_options(&mut self, peak_hold: bool, falloff: FalloffSpeed) {
        self.peak_hold_enabled = peak_hold;
        self.falloff_rate = falloff.rate();
    }

    fn size(&self) -> (u32, u32) {
        (76, 16)
    }

    pub fn update(&mut self, spectrum: &[f32]) {
        let now = std::time::Instant::now();
        let dt = self
            .last_update
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0);
        self.last_update = Some(now);

        // 1:1 copy — the audio thread now emits exactly 16 dB-scaled
        // bins. v1 sent 64 and we re-averaged here; that double-binning
        // is gone (O11 in AUDIO_OBJECTIVES.md). If the engine ever
        // sends a different count (e.g. a future "wide" mode), we copy
        // what we can and leave the rest at zero rather than re-binning.
        let n = spectrum.len().min(self.bins.len());
        for (slot, &v) in self.bins[..n].iter_mut().zip(&spectrum[..n]) {
            *slot = v.clamp(0.0, 1.0);
        }
        for slot in self.bins[n..].iter_mut() {
            *slot = 0.0;
        }

        // Peak hold: snap up to current bar, then decay over time.
        let fall = (self.falloff_rate / 16.0) * dt; // normalized 0..1 per second × dt
        for (peak, &bar) in self.peaks.iter_mut().zip(self.bins.iter()) {
            if bar >= *peak {
                *peak = bar;
            } else {
                *peak = (*peak - fall).max(0.0);
            }
        }
    }

    /// Render the 16 spectrum bars using `viscolor.txt`:
    /// - background: line 0
    /// - bar gradient: lines 2-17 (top → bottom of each bar)
    /// - peak marker: line 23 (a single 1×1 cell that "floats" at the top
    ///   of each bar for ~1 s before falling, à la classic Winamp)
    ///
    /// Each bar spans 16 vertical "cells" in skin space; cell k from the
    /// bottom uses `vis_colors[17 - k]`. Only as many cells as the bar's
    /// amplitude fills are drawn.
    pub fn render(&self, ui: &mut egui::Ui, offset: Pos2, scale: f32, vis_colors: &VisColors) {
        let (x, y) = self.position;
        let (w, h) = self.size();

        let analyzer_pos = offset + Vec2::new(x as f32 * scale, y as f32 * scale);
        let bar_width = (w as f32 / 16.0) * scale;
        let max_height = h as f32 * scale;

        let bg = vis_color32(vis_colors[0]);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(analyzer_pos, Vec2::new(w as f32 * scale, max_height)),
            0.0,
            bg,
        );

        let cell_height = scale; // 1 skin pixel per color cell.
        let peak_color = vis_color32(vis_colors[23]);

        for (i, &amplitude) in self.bins.iter().enumerate() {
            let bar_x = analyzer_pos.x + (i as f32 * bar_width);
            let cells_filled = (amplitude * 16.0).round().clamp(0.0, 16.0) as i32;

            // Bar body
            for cell in 0..cells_filled {
                // cell=0 is the bottom-most pixel → vis_colors[17].
                let color_idx = 17_i32 - cell;
                let color = vis_color32(vis_colors[color_idx as usize]);
                let cell_top = analyzer_pos.y + max_height - (cell as f32 + 1.0) * cell_height;
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        Pos2::new(bar_x + scale, cell_top),
                        Vec2::new(bar_width - 2.0 * scale, cell_height),
                    ),
                    0.0,
                    color,
                );
            }

            // Peak hold marker — only when the peak is above the current bar
            // (otherwise it overlaps the top cell and is invisible).
            let peak = self.peaks[i];
            if self.peak_hold_enabled && peak > amplitude + 1e-3 {
                let peak_cell = (peak * 16.0).round().clamp(1.0, 16.0) as i32;
                let cell_top = analyzer_pos.y + max_height - (peak_cell as f32) * cell_height;
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        Pos2::new(bar_x + scale, cell_top),
                        Vec2::new(bar_width - 2.0 * scale, cell_height),
                    ),
                    0.0,
                    peak_color,
                );
            }
        }
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Time-domain oscilloscope drawn in the main window's 76×16 visualization
/// zone. Holds the latest `WaveformSnapshot` (one min/max pair per source
/// column) from the audio thread and renders it as a filled waveform using
/// the skin's `viscolor.txt` oscilloscope band (entries 18..=22), classic
/// Winamp style — but with proper min/max bars instead of v1's
/// nearest-neighbor dotted line.
pub struct Oscilloscope {
    position: (u32, u32),
    /// Latest (min, max) pairs from the audio thread, in chronological
    /// order. Same length as `maxs`; empty until the first frame.
    mins: Vec<f32>,
    maxs: Vec<f32>,
    /// User-selected render style (Lines / Dots / Solid).
    style: OscilloscopeStyle,
}

impl Oscilloscope {
    pub fn new() -> Self {
        Self {
            position: (24, 43),
            mins: Vec::new(),
            maxs: Vec::new(),
            style: OscilloscopeStyle::default(),
        }
    }

    /// Set the render style (View → Visualizer → Oscilloscope style).
    pub fn set_style(&mut self, style: OscilloscopeStyle) {
        self.style = style;
    }

    fn size(&self) -> (u32, u32) {
        (76, 16)
    }

    /// Take a new pre-decimated snapshot from the audio thread. The
    /// renderer resamples to the display width on each frame.
    pub fn update(&mut self, snapshot: &oneamp_core::WaveformSnapshot) {
        self.mins.clear();
        self.mins.extend_from_slice(&snapshot.mins);
        self.maxs.clear();
        self.maxs.extend_from_slice(&snapshot.maxs);
    }

    /// Render the waveform as a min/max filled strip. Background =
    /// `vis_colors[0]`. The 16-cell-tall area is split into 5 horizontal
    /// bands by distance from center: cells 0..2 → vis_colors[18], 3..4
    /// → 19, 5..6 → 20, 7..8 → 21, deeper → 22. For each pixel column
    /// we pick the source (min, max) pair, map them to row indices,
    /// and fill all cells between them — gives a solid vertical strip
    /// whose height tracks the signal envelope per column. v1 painted
    /// one cell per column from a single nearest-neighbor sample, which
    /// aliased high-frequency content into phantom slow waves.
    pub fn render(&self, ui: &mut egui::Ui, offset: Pos2, scale: f32, vis_colors: &VisColors) {
        let (x, y) = self.position;
        let (w, h) = self.size();

        let pos = offset + Vec2::new(x as f32 * scale, y as f32 * scale);
        let area_w = w as f32 * scale;
        let area_h = h as f32 * scale;

        // Background
        ui.painter().rect_filled(
            egui::Rect::from_min_size(pos, Vec2::new(area_w, area_h)),
            0.0,
            vis_color32(vis_colors[0]),
        );

        if self.mins.is_empty() {
            return;
        }

        let cell = scale; // 1 skin pixel per cell
        let center_row = (h / 2) as f32; // 8 — both halves are 8 cells tall
        let n = self.mins.len().min(self.maxs.len());

        let sample_to_row = |sample: f32| -> i32 {
            let row_f = center_row - sample.clamp(-1.0, 1.0) * center_row;
            (row_f.round() as i32).clamp(0, h as i32 - 1)
        };

        // Band color by vertical distance from the center line — the
        // classic green-near-center → warmer-toward-edges oscilloscope
        // palette (`viscolor.txt` entries 18..=22).
        let band_color = |row: i32| -> Color32 {
            let dist = (row - center_row as i32).unsigned_abs();
            let color_idx = match dist {
                0..=2 => 18,
                3..=4 => 19,
                5..=6 => 20,
                7..=8 => 21,
                _ => 22,
            };
            vis_color32(vis_colors[color_idx])
        };
        let fill_cell = |ui: &mut egui::Ui, px: u32, row: i32, color: Color32| {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    Pos2::new(pos.x + px as f32 * cell, pos.y + row as f32 * cell),
                    Vec2::new(cell, cell),
                ),
                0.0,
                color,
            );
        };
        // For dots/lines a column is represented by its positive-peak
        // sample row — what a classic scope traces.
        let col_row = |px: u32| -> i32 {
            let s_idx = (px as usize * n) / w as usize;
            sample_to_row(self.maxs[s_idx])
        };

        match self.style {
            OscilloscopeStyle::Solid => {
                for px in 0..w {
                    let s_idx = (px as usize * n) / w as usize;
                    // The producer guarantees min ≤ max, but sample_to_row
                    // inverts the sign axis — `max` (positive amplitude)
                    // maps to a lower row index, `min` to higher.
                    let row_a = sample_to_row(self.maxs[s_idx]);
                    let row_b = sample_to_row(self.mins[s_idx]);
                    let (row_top, row_bot) = (row_a.min(row_b), row_a.max(row_b));
                    for row in row_top..=row_bot {
                        fill_cell(ui, px, row, band_color(row));
                    }
                }
            }
            OscilloscopeStyle::Dots => {
                for px in 0..w {
                    let row = col_row(px);
                    fill_cell(ui, px, row, band_color(row));
                }
            }
            OscilloscopeStyle::Lines => {
                // Connect successive column samples with vertical fills so
                // a continuous trace appears even across steep slopes.
                let mut prev = col_row(0);
                for px in 0..w {
                    let row = col_row(px);
                    let (a, b) = (prev.min(row), prev.max(row));
                    for r in a..=b {
                        fill_cell(ui, px, r, band_color(r));
                    }
                    prev = row;
                }
            }
        }
    }
}

impl Default for Oscilloscope {
    fn default() -> Self {
        Self::new()
    }
}

/// Stereo peak / RMS meter drawn in the 76×16 visualization zone.
///
/// Layout: top 8 cells = left channel, bottom 8 cells = right channel.
/// Each row is 76 cells wide. The bar fill tracks the RMS value
/// mapped log-style onto a `[-METER_FLOOR_DB, 0] dB → [0, 76]` cell
/// span; the peak (short-term, reset each refresh) is overlaid as a
/// single bright cell at its mapped position, with a slow client-side
/// fall so the user can see a moment ago's peak before it disappears.
///
/// Color buckets reuse the oscilloscope band (vis_colors[18..=22])
/// for green→red gradient feel; the peak hold cap uses
/// `vis_colors[23]` like the spectrum analyzer.
pub struct PeakMeter {
    position: (u32, u32),
    /// Latest published RMS (linear amplitude in [0,1]).
    rms_l: f32,
    rms_r: f32,
    /// Client-side held peak, in linear amplitude. Snaps up to the
    /// engine's per-refresh peak then falls slowly over time so the
    /// last transient stays visible briefly.
    peak_hold_l: f32,
    peak_hold_r: f32,
    last_update: Option<std::time::Instant>,
}

/// Display dB floor for the meter bar mapping. -40 dB to 0 dBFS
/// covers the useful range of typical music; finer resolution would
/// just compress the visible bar at low levels into noise.
const METER_FLOOR_DB: f32 = -40.0;
/// Linear amplitude per second the peak-hold cap falls. With a
/// `[0, 1]` range, 0.6/s means a full-scale peak takes ~1.7 s to
/// decay to zero — matches the classic "VU peak hold" feel.
const PEAK_FALL_LINEAR_PER_SEC: f32 = 0.6;

impl PeakMeter {
    pub fn new() -> Self {
        Self {
            position: (24, 43),
            rms_l: 0.0,
            rms_r: 0.0,
            peak_hold_l: 0.0,
            peak_hold_r: 0.0,
            last_update: None,
        }
    }

    fn size(&self) -> (u32, u32) {
        (76, 16)
    }

    /// Pull a fresh `MeterSnapshot` from the audio engine. Updates
    /// RMS straight, runs the peak hold (snap-up + slow fall).
    pub fn update(&mut self, snapshot: &oneamp_core::MeterSnapshot) {
        let now = std::time::Instant::now();
        let dt = self
            .last_update
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0);
        self.last_update = Some(now);

        self.rms_l = snapshot.rms_l.clamp(0.0, 1.0);
        self.rms_r = snapshot.rms_r.clamp(0.0, 1.0);

        let fall = PEAK_FALL_LINEAR_PER_SEC * dt;
        let new_l = snapshot.peak_l.clamp(0.0, 1.0);
        let new_r = snapshot.peak_r.clamp(0.0, 1.0);
        if new_l >= self.peak_hold_l {
            self.peak_hold_l = new_l;
        } else {
            self.peak_hold_l = (self.peak_hold_l - fall).max(0.0);
        }
        if new_r >= self.peak_hold_r {
            self.peak_hold_r = new_r;
        } else {
            self.peak_hold_r = (self.peak_hold_r - fall).max(0.0);
        }
    }

    /// Convert a linear amplitude in `[0, 1]` to a bar-cell count in
    /// `[0, width]` via a `-40..0 dB` mapping. Below the floor the
    /// bar is empty; at 0 dBFS the bar fills the full row.
    fn amplitude_to_cells(amp: f32, width: u32) -> u32 {
        if amp <= 0.0 {
            return 0;
        }
        let db = 20.0 * amp.max(1e-6).log10();
        if db <= METER_FLOOR_DB {
            return 0;
        }
        let ratio = (db - METER_FLOOR_DB) / -METER_FLOOR_DB;
        (ratio.clamp(0.0, 1.0) * width as f32).round() as u32
    }

    /// Pick a color for the cell at `cell` (0-indexed from the left)
    /// in a `width`-wide bar. 5 buckets like the oscilloscope band:
    /// the rightmost cells (hot levels) glow the brightest red.
    fn cell_color(cell: u32, width: u32) -> usize {
        let ratio = cell as f32 / width.max(1) as f32;
        match ratio {
            r if r < 0.20 => 18,
            r if r < 0.40 => 19,
            r if r < 0.60 => 20,
            r if r < 0.80 => 21,
            _ => 22,
        }
    }

    /// Render a single horizontal bar within the meter zone.
    #[allow(clippy::too_many_arguments)]
    fn draw_bar(
        ui: &mut egui::Ui,
        origin: Pos2,
        row_count: u32,
        scale: f32,
        rms: f32,
        peak: f32,
        vis_colors: &VisColors,
        bar_width_cells: u32,
        bar_height_rows: u32,
    ) {
        let cell = scale;
        let rms_cells = Self::amplitude_to_cells(rms, bar_width_cells);
        let peak_cell = Self::amplitude_to_cells(peak, bar_width_cells);
        let peak_color = vis_color32(vis_colors[23]);

        for i in 0..rms_cells {
            let color = vis_color32(vis_colors[Self::cell_color(i, bar_width_cells)]);
            let x = origin.x + i as f32 * cell;
            let rect = egui::Rect::from_min_size(
                Pos2::new(x, origin.y),
                Vec2::new(cell, row_count as f32 * cell),
            );
            ui.painter().rect_filled(rect, 0.0, color);
        }

        // Peak hold marker: a single column drawn the full bar height
        // at the peak position, but only when it's ahead of the RMS
        // fill — otherwise it'd be invisible under the bar.
        if peak_cell > rms_cells && peak_cell > 0 {
            let x = origin.x + (peak_cell - 1) as f32 * cell;
            let rect = egui::Rect::from_min_size(
                Pos2::new(x, origin.y),
                Vec2::new(cell, bar_height_rows as f32 * cell),
            );
            ui.painter().rect_filled(rect, 0.0, peak_color);
        }
    }

    pub fn render(&self, ui: &mut egui::Ui, offset: Pos2, scale: f32, vis_colors: &VisColors) {
        let (x, y) = self.position;
        let (w, h) = self.size();
        let pos = offset + Vec2::new(x as f32 * scale, y as f32 * scale);

        // Background
        ui.painter().rect_filled(
            egui::Rect::from_min_size(pos, Vec2::new(w as f32 * scale, h as f32 * scale)),
            0.0,
            vis_color32(vis_colors[0]),
        );

        // 8-cell-tall bars; reserve a 0-cell gap between them so the
        // L/R bars stay visually distinct even when full-scale.
        let bar_height = h / 2;

        // Left channel: top half, y = 0..(h/2 - 1) (one-cell gap on
        // bottom so the eye separates L from R).
        let top_origin = pos;
        let top_height = bar_height - 1;
        Self::draw_bar(
            ui,
            top_origin,
            top_height,
            scale,
            self.rms_l,
            self.peak_hold_l,
            vis_colors,
            w,
            top_height,
        );

        // Right channel: bottom half, starts at row h/2.
        let bot_origin = pos + Vec2::new(0.0, bar_height as f32 * scale);
        let bot_height = bar_height - 1;
        Self::draw_bar(
            ui,
            bot_origin,
            bot_height,
            scale,
            self.rms_r,
            self.peak_hold_r,
            vis_colors,
            w,
            bot_height,
        );
    }
}

impl Default for PeakMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// Channel state for the mono/stereo indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// No track loaded — both labels dim ("STENO" idle look).
    None,
    Mono,
    Stereo,
}

pub struct MonoStereoDisplay {
    pub channels: ChannelState,
}

impl MonoStereoDisplay {
    /// Skin-space position of the mono indicator (27×12). Spec WSZ §main.bmp.
    const MONO_POS: (u32, u32) = (212, 41);
    /// Skin-space position of the stereo indicator (29×12).
    const STEREO_POS: (u32, u32) = (239, 41);

    pub fn new() -> Self {
        Self {
            channels: ChannelState::None,
        }
    }

    /// Render BOTH mono and stereo indicators side-by-side. Active state
    /// lights one sprite, inactive shows dim — same behaviour as classic
    /// Winamp where you always see "STENO" with the relevant half lit.
    ///
    /// Atlas layout (`monoster.bmp` 58×24):
    /// - (0,0) 29×12 stereo active
    /// - (29,0) 27×12 mono active (overlaps stereo's last 2 cols by design)
    /// - (0,12) 29×12 stereo inactive
    /// - (29,12) 27×12 mono inactive
    pub fn render(&self, renderer: &mut WszRenderer, ui: &mut egui::Ui, offset: Pos2) {
        let scale = renderer.get_scale();
        let atlas = match renderer
            .get_skin()
            .get_bitmap(&SkinComponent::MonoSter)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // Sprite half-height — defensive in case the skin ships a non-standard
        // monoster.bmp.
        let row_h = (atlas.height / 2).min(12);
        if row_h == 0 {
            return;
        }

        let stereo_lit = matches!(self.channels, ChannelState::Stereo);
        let mono_lit = matches!(self.channels, ChannelState::Mono);
        let stereo_y = if stereo_lit { 0 } else { row_h };
        let mono_y = if mono_lit { 0 } else { row_h };

        // Stereo sprite: (0, y) 29×12, drawn at skin (239, 41).
        let stereo_w = atlas.width.min(29);
        if stereo_w > 0
            && let Some(region) = atlas.extract_region(0, stereo_y, stereo_w, row_h)
        {
            let (sx, sy) = Self::STEREO_POS;
            let pos = offset + Vec2::new(sx as f32 * scale, sy as f32 * scale);
            let key = if stereo_lit {
                "monoster_stereo_on"
            } else {
                "monoster_stereo_off"
            };
            renderer.render_region(ui, &region, pos, key);
        }

        // Mono sprite: (29, y) 27×12, drawn at skin (212, 41).
        if atlas.width > 29 {
            let mono_w = (atlas.width - 29).min(27);
            if let Some(region) = atlas.extract_region(29, mono_y, mono_w, row_h) {
                let (mx, my) = Self::MONO_POS;
                let pos = offset + Vec2::new(mx as f32 * scale, my as f32 * scale);
                let key = if mono_lit {
                    "monoster_mono_on"
                } else {
                    "monoster_mono_off"
                };
                renderer.render_region(ui, &region, pos, key);
            }
        }
    }

    pub fn set_state(&mut self, state: ChannelState) {
        self.channels = state;
    }
}

impl Default for MonoStereoDisplay {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BitrateDisplay {
    /// Bitrate in kbps. 0 means unknown.
    pub bitrate_kbps: u32,
    /// Sample rate in Hz. 0 means unknown.
    pub sample_rate_hz: u32,
}

impl BitrateDisplay {
    /// Skin-space position of the bitrate digits — 3-char zone per WSZ spec.
    const BITRATE_POS: (u32, u32) = (111, 43);
    /// Skin-space position of the sample-rate digits — 2-char zone.
    const SAMPLE_RATE_POS: (u32, u32) = (156, 43);

    pub fn new() -> Self {
        Self {
            bitrate_kbps: 0,
            sample_rate_hz: 0,
        }
    }

    /// Render the bitrate (3 digits) and sample-rate (2 digits) overlays at
    /// their fixed Winamp positions. Per WSZ_FORMAT §main.bmp, these zones are
    /// digits-only — the "KBPS" / "KHZ" labels (when present) are baked into
    /// main.bmp by the skin author, never drawn by us. Missing values render
    /// as "0" so the user always sees a number instead of a blank slot.
    pub fn render(&self, renderer: &mut WszRenderer, ui: &mut egui::Ui, offset: Pos2) {
        let scale = renderer.get_scale();

        let text = format_right_aligned(self.bitrate_kbps, 3);
        let (x, y) = Self::BITRATE_POS;
        let pos = offset + Vec2::new(x as f32 * scale, y as f32 * scale);
        self.draw_field(renderer, ui, &text, pos, scale);

        let khz = self.sample_rate_hz / 1000;
        let text = format_right_aligned(khz, 2);
        let (x, y) = Self::SAMPLE_RATE_POS;
        let pos = offset + Vec2::new(x as f32 * scale, y as f32 * scale);
        self.draw_field(renderer, ui, &text, pos, scale);
    }

    fn draw_field(
        &self,
        renderer: &mut WszRenderer,
        ui: &mut egui::Ui,
        text: &str,
        pos: Pos2,
        scale: f32,
    ) {
        if bitmap_font::render_text(renderer, ui, text, pos).is_none() {
            // Skin's text atlas is too small / missing — fall back to egui
            // proportional font so the user still sees the info.
            ui.painter().text(
                pos,
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(7.0 * scale),
                Color32::from_rgb(0, 255, 0),
            );
        }
    }

    pub fn set_info(&mut self, bitrate_kbps: u32, sample_rate_hz: u32) {
        self.bitrate_kbps = bitrate_kbps;
        self.sample_rate_hz = sample_rate_hz;
    }
}

/// Format `value` as a `width`-char string, right-aligned with leading spaces.
/// Values that overflow the width are truncated to their leading digits — the
/// 3-char bitrate field can't represent a FLAC's 1411 kbps so we show "141".
fn format_right_aligned(value: u32, width: usize) -> String {
    let s = value.to_string();
    if s.len() >= width {
        s.chars().take(width).collect()
    } else {
        format!("{:>width$}", s, width = width)
    }
}

impl Default for BitrateDisplay {
    fn default() -> Self {
        Self::new()
    }
}
