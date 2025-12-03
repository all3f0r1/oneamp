# OneAmp v0.15 - UI/UX Improvement Specifications

## Overview

This document outlines all improvements for v0.15 focused on UI/UX enhancements. The goal is to transform OneAmp from a basic music player into a polished, modern application with proper theming, visual feedback, and professional appearance.

## Current Issues (v0.14.8)

1. ❌ Skin not properly applied to all UI elements
2. ❌ Poor layout and spacing (too compact, hard to read)
3. ❌ No hover effects or visual feedback on buttons
4. ❌ Audio format detection incorrect (shows 24kHz Mono instead of actual format)
5. ❌ Buttons and icons are hard to see (low contrast)
6. ❌ No visual hierarchy or emphasis
7. ❌ ALSA warnings cluttering console output

## Phase 1: Skin Application (Priority: HIGH)

### 1.1 Theme Integration

**Current State:**
- Theme::default() is applied at startup
- Skin is loaded and applied separately
- Theme colors may override skin colors

**Changes Required:**

```rust
// In main.rs - new() function
// Change from:
let theme = Theme::default();
theme.apply_to_egui(&cc.egui_ctx);

// To:
let mut theme = Theme::default();
// Don't apply theme yet - let skin override it
// Apply skin first, then theme as fallback
```

**Implementation:**
1. Load skin BEFORE theme
2. Extract skin colors and apply to theme
3. Use skin colors as primary, theme as fallback
4. Ensure consistency across all UI elements

### 1.2 Skin Color Application

**Required Changes:**

- **Player Section**: Use skin.colors.text_normal for track title
- **Progress Bar**: Use skin.colors.accent for progress indicator
- **Control Buttons**: Use skin.colors.accent for active state
- **Equalizer**: Use skin.colors.accent for frequency bars
- **Playlist**: Use skin.colors.text_normal for items
- **Visualizer**: Use skin.colors.spectrum_color

**Files to Modify:**
- `ui_components.rs` - Apply skin colors to all components
- `control_buttons.rs` - Use skin accent color
- `equalizer_display.rs` - Use skin colors for bars
- `custom_widgets.rs` - Apply skin styling

## Phase 2: Layout and Spacing (Priority: HIGH)

### 2.1 Main Layout Improvements

**Current Issues:**
- Elements too close together
- Hard to distinguish sections
- Poor visual hierarchy

**Changes Required:**

```
BEFORE:
[Player Section]
[8px space]
[Progress Bar]
[8px space]
[Control Buttons]

AFTER:
[Player Section - Larger, more padding]
[16px space]
[Progress Bar - More visible]
[16px space]
[Control Buttons - Larger, better spacing]
[24px space]
[Visualizer/Skins Section]
[24px space]
[Equalizer Section]
[24px space]
[Playlist Section]
```

**Implementation:**
1. Increase spacing between major sections (8px → 16-24px)
2. Add padding inside sections (4px → 8-12px)
3. Increase button sizes (current: ~40px → target: 50-60px)
4. Better use of available space

**Files to Modify:**
- `main.rs` - Update spacing in update() function
- `ui_components.rs` - Add padding to components
- `control_buttons.rs` - Increase button size

### 2.2 Player Section Improvements

**Current:**
- Time display too small
- Track info cramped
- No visual separation

**Target:**
- Time display: 48pt font (instead of current ~24pt)
- Track title: 20pt font with proper padding
- Sample rate/channels: 14pt font, secondary color
- Clear visual separation with background panel

**Implementation:**
```rust
// Increase font sizes
ui.heading(format!("{:02}:{:02}", minutes, seconds)); // 48pt
ui.label(RichText::new(track_title).size(20.0)); // 20pt
ui.label(RichText::new(format!("{}Hz • {}", sample_rate, channels)).size(14.0).color(Color32::GRAY));
```

### 2.3 Control Buttons Layout

**Current:**
- 4 buttons in a row, hard to click
- Album art takes space
- Poor alignment

**Target:**
- Larger buttons (60px diameter)
- Better visual feedback
- Album art on left side with proper spacing
- Buttons centered and easy to click

**Implementation:**
```rust
// Increase button size
let button_size = 60.0; // was 40.0
// Add proper spacing
ui.add_space(20.0); // between album art and buttons
```

## Phase 3: Hover Effects and Transitions (Priority: MEDIUM)

### 3.1 Button Hover Effects

**Required:**
- Color change on hover
- Slight scale animation
- Visual feedback on click

**Implementation:**
```rust
// In control_buttons.rs
let response = ui.button("▶");
if response.hovered() {
    // Highlight color
    // Scale animation
}
if response.clicked() {
    // Click animation
}
```

**Files to Modify:**
- `control_buttons.rs` - Add hover effects
- `custom_widgets.rs` - Add hover effects to all buttons
- `animations.rs` - Extend animation system

### 3.2 UI Transitions

**Required:**
- Smooth color transitions when changing skins
- Smooth visibility transitions for sections
- Smooth progress bar updates

**Implementation:**
- Use AnimationTimer for transitions
- Lerp between colors
- Smooth progress updates

## Phase 4: Audio Format Detection (Priority: HIGH)

### 4.1 Fix Format Detection

**Current Issue:**
- Shows "24kHz • Mono" instead of actual format
- Likely reading wrong metadata

**Root Cause:**
- Probably reading from audio engine instead of file metadata
- Need to extract actual format from file

**Implementation:**
```rust
// In track_display.rs or ui_components.rs
// Instead of:
format!("{}Hz • {}", sample_rate, channels)

// Should be:
// Extract from symphonia metadata
let format = track_info.format; // "MP3", "FLAC", etc.
let bitrate = track_info.bitrate; // "320kbps", etc.
format!("{} • {} • {}", format, bitrate, sample_rate)
```

**Files to Modify:**
- `track_display.rs` - Fix format display
- `oneamp-core/src/symphonia_player.rs` - Ensure correct metadata extraction

### 4.2 Add Format Display

**Target Display:**
```
Track Title
MP3 • 320kbps • 44.1kHz • Stereo
```

Instead of current:
```
Track Title
24kHz • Mono
```

## Phase 5: Icons and Visibility (Priority: MEDIUM)

### 5.1 Improve Icon Visibility

**Current:**
- Icons are small and hard to see
- Low contrast with background

**Changes:**
- Use larger Unicode icons
- Add color to icons
- Better contrast

**Icon Updates:**
```
Current → Target
▶ → ▶ (larger, colored)
⏸ → ⏸ (larger, colored)
⏹ → ⏹ (larger, colored)
◄ → ◄ (larger, colored)
► → ► (larger, colored)
🎨 → 🎨 (keep, already good)
```

**Implementation:**
```rust
// Increase icon size
let icon_size = 24.0; // was 16.0
ui.label(RichText::new("▶").size(icon_size).color(accent_color));
```

### 5.2 Add Visual Indicators

**Required:**
- Active button highlighting
- Current track highlighting in playlist
- Visualizer activity indicator
- Equalizer band activity

**Implementation:**
- Use skin accent color for active states
- Add glow effect for active elements
- Add progress animation

## Phase 6: ALSA Warning Suppression (Priority: LOW)

### 6.1 Suppress Console Output

**Current:**
```
ALSA lib pcm_dmix.c:1000:(snd_pcm_dmix_open) unable to open slave
Cannot connect to server socket err = No such file or directory
...
```

**Solution:**
- Redirect stderr in audio engine initialization
- Or use environment variable to suppress ALSA warnings

**Implementation:**
```rust
// In oneamp-core/src/lib.rs or cpal_output.rs
// Suppress ALSA warnings
std::env::set_var("ALSA_CARD", "default");
// Or redirect stderr
let _guard = gag::Redirect::stderr(std::fs::File::open("/dev/null")?)?;
```

## Implementation Order

1. **Phase 4** (Audio Format Detection) - Quick win, high impact
2. **Phase 1** (Skin Application) - Foundation for other improvements
3. **Phase 2** (Layout and Spacing) - Major visual improvement
4. **Phase 5** (Icons and Visibility) - Enhance readability
5. **Phase 3** (Hover Effects) - Polish
6. **Phase 6** (ALSA Warnings) - Nice to have

## Files to Modify (Summary)

### High Priority
- `oneamp-desktop/src/main.rs` - Layout improvements, spacing
- `oneamp-desktop/src/ui_components.rs` - Apply skin colors, improve display
- `oneamp-desktop/src/control_buttons.rs` - Larger buttons, hover effects
- `oneamp-desktop/src/track_display.rs` - Fix format detection
- `oneamp-core/src/symphonia_player.rs` - Correct metadata extraction

### Medium Priority
- `oneamp-desktop/src/custom_widgets.rs` - Skin styling, hover effects
- `oneamp-desktop/src/equalizer_display.rs` - Apply skin colors
- `oneamp-desktop/src/animations.rs` - Extend for transitions
- `oneamp-desktop/src/skins/manager.rs` - Improve skin application

### Low Priority
- `oneamp-core/src/cpal_output.rs` - Suppress ALSA warnings
- `oneamp-core/src/rodio_output.rs` - Suppress ALSA warnings

## Testing Checklist

- [ ] Skin colors applied to all UI elements
- [ ] Layout properly spaced and readable
- [ ] Buttons have hover effects
- [ ] Audio format displays correctly
- [ ] Icons are visible and properly sized
- [ ] No console warnings on startup
- [ ] Smooth transitions between skins
- [ ] All buttons clickable and responsive
- [ ] Playlist displays correctly
- [ ] Equalizer displays correctly

## Estimated Effort

- Phase 1: 4-6 hours
- Phase 2: 6-8 hours
- Phase 3: 4-6 hours
- Phase 4: 2-3 hours
- Phase 5: 3-4 hours
- Phase 6: 1-2 hours

**Total: 20-29 hours**

## Success Criteria

✅ Application looks professional and modern
✅ All UI elements properly themed
✅ Good visual hierarchy and spacing
✅ Responsive buttons with visual feedback
✅ Correct audio format display
✅ No console warnings
✅ Smooth animations and transitions
✅ Easy to use and navigate

## Notes

- All changes must maintain backward compatibility
- No breaking changes to API
- All changes must be tested on Linux
- Code must follow Rust best practices
- All comments and documentation in English
