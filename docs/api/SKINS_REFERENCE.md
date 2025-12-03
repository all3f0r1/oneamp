# OneAmp Skins Reference

**Date:** December 3, 2025  
**Author:** Manus AI

---

## 1. Introduction

This document provides a complete reference for all the options available in the `skin.toml` file.

---

## 2. `[metadata]`

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | String | `"Default"` | The name of the skin. |
| `author` | String | `"OneAmp"` | The author of the skin. |
| `version` | String | `"1.0"` | The version of the skin. |
| `description` | String | `"Default skin"` | A short description of the skin. |

---

## 3. `[colors]`

All colors must be specified as hex strings (e.g., `"#RRGGBB"`).

| Key | Default | Description |
|---|---|---|
| `dark_mode` | `true` | `true` for dark themes, `false` for light themes. |
| `background` | `"#0a0a0a"` | The main background color. |
| `text` | `"#ffffff"` | The default text color. |
| `window_fill` | `"#1a1a1a"` | The background color of windows. |
| `window_stroke` | `"#404040"` | The border color of windows. |
| `panel_fill` | `"#0f0f0f"` | The background color of panels. |
| `widget_bg` | `"#2a2a2a"` | The background color of widgets. |
| `widget_stroke` | `"#404040"` | The border color of widgets. |
| `hovered_widget_bg` | `"#3a3a3a"` | The background color of hovered widgets. |
| `active_widget_bg` | `"#4a4a4a"` | The background color of active widgets. |
| `inactive_widget_bg` | `"#1a1a1a"` | The background color of inactive widgets. |
| `accent` | `"#00d4ff"` | The accent color for highlights and selections. |
| `error` | `"#ff4444"` | The color for error messages. |
| `warning` | `"#ffbb33"` | The color for warning messages. |
| `playlist_current_track` | `"#00d4ff"` | The color for the current track in the playlist. |
| `playlist_selected_bg` | `"#404040"` | The background color of selected tracks in the playlist. |

---

## 4. `[fonts]`

| Key | Type | Default | Description |
|---|---|---|---|
| `proportional` | String | `"Arial"` | The default font for most UI text. |
| `monospace` | String | `"Courier New"` | The font for timers and other monospaced text. |
| `timer_font` | String (Path) | `null` | An optional path to a custom font file for the timer. |

---

## 5. `[metrics]`

| Key | Type | Default | Description |
|---|---|---|---|
| `window_rounding` | Float | `4.0` | The corner rounding for windows. |
| `widget_rounding` | Float | `2.0` | The corner rounding for widgets. |
| `scrollbar_width` | Float | `8.0` | The width of scrollbars. |
| `window_padding` | Float | `8.0` | The padding inside windows. |
| `button_padding` | Array [Float, Float] | `[12.0, 4.0]` | The padding inside buttons [x, y]. |
| `body_text_size` | Float | `14.0` | The default font size for body text. |
| `heading_text_size` | Float | `18.0` | The font size for headings. |
| `timer_text_size` | Float | `48.0` | The font size for the timer display. |

