# Creating Custom Skins for OneAmp

**Date:** December 3, 2025  
**Author:** Manus AI

---

## 1. Introduction

OneAmp's skinning system allows you to completely change the look and feel of the application. This guide will walk you through the process of creating your own custom skin.

### What You Can Customize

- **Colors:** Change the color of every UI element, from the background to the text and accents.
- **Fonts:** Specify custom fonts for different parts of the UI.
- **Metrics:** Adjust the size, spacing, and rounding of UI elements.

### How It Works

Skins are defined in a simple TOML file (`skin.toml`). OneAmp discovers and loads these files from the `skins` directory in the application's root folder.

---

## 2. Getting Started

### 2.1. Create a Skin Directory

First, create a new directory for your skin inside the `skins` folder. The directory name should be unique and descriptive.

```bash
mkdir skins/my-custom-skin
```

### 2.2. Create the `skin.toml` File

Inside your new directory, create a file named `skin.toml`. This is where you will define your skin's properties.

```bash
touch skins/my-custom-skin/skin.toml
```

### 2.3. Basic Skin Structure

Your `skin.toml` file should have the following structure:

```toml
[metadata]
# ...

[colors]
# ...

[fonts]
# ...

[metrics]
# ...
```

---

## 3. Skin Definition

### 3.1. Metadata

The `[metadata]` section contains information about your skin.

```toml
[metadata]
name = "My Custom Skin"
author = "Your Name"
version = "1.0"
description = "A cool new skin for OneAmp."
```

| Key | Description |
|---|---|
| `name` | The name of your skin, as it will appear in the UI. |
| `author` | Your name or username. |
| `version` | The version of your skin. |
| `description` | A short description of your skin. |

### 3.2. Colors

The `[colors]` section defines the color palette for your skin. All colors must be specified as hex strings (e.g., `"#RRGGBB"`).

```toml
[colors]
dark_mode = true
background = "#0a0a0a"
text = "#ffffff"
window_fill = "#1a1a1a"
# ... and so on
```

**Key Colors:**

| Key | Description |
|---|---|
| `dark_mode` | `true` for dark themes, `false` for light themes. |
| `background` | The main background color of the application. |
| `text` | The default text color. |
| `accent` | The accent color for highlights and selections. |

For a full list of available colors, see the [Skins Reference](</docs/api/SKINS_REFERENCE.md).

### 3.3. Fonts

The `[fonts]` section allows you to specify fonts for the UI.

```toml
[fonts]
proportional = "Arial"
monospace = "Courier New"
timer_font = "path/to/your/font.ttf" # Optional
```

| Key | Description |
|---|---|
| `proportional` | The default font for most UI text. |
| `monospace` | The font for timers and other monospaced text. |
| `timer_font` | An optional path to a custom font file for the timer. |

### 3.4. Metrics

The `[metrics]` section controls the layout and spacing of the UI.

```toml
[metrics]
window_rounding = 4.0
widget_rounding = 2.0
body_text_size = 14.0
# ... and so on
```

**Key Metrics:**

| Key | Description |
|---|---|
| `window_rounding` | The corner rounding for windows. |
| `widget_rounding` | The corner rounding for buttons, sliders, etc. |
| `body_text_size` | The default font size for body text. |

For a full list of available metrics, see the [Skins Reference](</docs/api/SKINS_REFERENCE.md).

---

## 4. Testing Your Skin

To test your skin, simply launch OneAmp. Your new skin should appear in the skin selector menu. If you make changes to your `skin.toml` file while OneAmp is running, the changes will be applied automatically.

---

## 5. Example Skins

For inspiration, check out the default skins in the `skins` directory:

- `oneamp-dark` - The default dark theme.
- `winamp5-classified` - A theme inspired by the classic Winamp skin.

Happy skinning! 🎨
