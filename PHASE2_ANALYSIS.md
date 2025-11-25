# Phase 2 Analysis: Rendering wgpu Textures in egui

## 📊 Research Summary

Based on egui GitHub discussions and PRs, the solution for displaying wgpu textures in egui is:

### Key Finding: `register_native_texture`

**PR #1660** added the ability to register arbitrary wgpu textures as user textures in egui-wgpu.

## 🔧 Solution Architecture

### Method 1: Using `register_native_texture` (Recommended)

```rust
// In eframe with wgpu backend
use eframe::egui_wgpu::RenderState;

// Get the render state
let render_state = frame.wgpu_render_state().unwrap();

// Register the wgpu texture
let texture_id = render_state.renderer.write().register_native_texture(
    &render_state.device,
    &wgpu_texture.create_view(&Default::default()),
    wgpu::FilterMode::Linear,
);

// Display in egui
ui.image(egui::load::SizedTexture::new(
    texture_id,
    egui::vec2(width as f32, height as f32)
));
```

### Method 2: Copy to CPU (Fallback)

If `register_native_texture` doesn't work, we can copy the texture to CPU and upload as egui texture:

```rust
// Read texture to CPU
let buffer = device.create_buffer(&wgpu::BufferDescriptor { ... });
encoder.copy_texture_to_buffer(...);

// Upload to egui
let color_image = egui::ColorImage::from_rgba_unmultiplied(...);
let texture = ctx.load_texture("onedrop", color_image, Default::default());
ui.image(&texture);
```

**Note**: Method 2 is less performant but guaranteed to work.

---

## 🎯 Implementation Plan

### Step 1: Access RenderState in OneAmpApp

eframe provides access to wgpu render state via `Frame`:

```rust
impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Access wgpu render state
        if let Some(render_state) = frame.wgpu_render_state() {
            // We can now register textures
        }
    }
}
```

### Step 2: Register OneDrop Texture

```rust
// In update() method
if self.use_onedrop {
    if let Some(ref mut onedrop) = self.onedrop {
        // Update OneDrop with audio
        let audio_samples = self.get_audio_samples();
        let delta_time = ctx.input(|i| i.unstable_dt);
        let _ = onedrop.update(&audio_samples, delta_time);
        
        // Get texture from OneDrop
        if let Some(texture) = onedrop.render_texture() {
            // Register with egui
            if let Some(render_state) = frame.wgpu_render_state() {
                let texture_id = render_state.renderer.write()
                    .register_native_texture(
                        &render_state.device,
                        &texture.create_view(&Default::default()),
                        wgpu::FilterMode::Linear,
                    );
                
                // Display
                ui.image(egui::load::SizedTexture::new(
                    texture_id,
                    egui::vec2(800.0, 600.0)
                ));
            }
        }
    }
}
```

### Step 3: Cache TextureId

To avoid re-registering every frame:

```rust
struct OneAmpApp {
    // ...
    onedrop_texture_id: Option<egui::TextureId>,
}

// Register once, reuse
if self.onedrop_texture_id.is_none() {
    self.onedrop_texture_id = Some(texture_id);
}
```

---

## 🚀 Audio Samples Integration

OneDrop needs audio samples to react to music. We need to extract them from the audio engine:

```rust
// In OneAmpApp
fn get_audio_samples(&self) -> Vec<f32> {
    // Option 1: From visualizer (already has FFT data)
    self.visualizer.get_audio_samples()
    
    // Option 2: From audio engine directly
    // Need to add a method to AudioEngine to expose samples
}
```

**Challenge**: The current `Visualizer` doesn't expose raw audio samples, only spectrum data.

**Solution**: Add a method to `Visualizer` to get raw audio samples:

```rust
// In visualizer.rs
impl Visualizer {
    pub fn get_audio_samples(&self) -> &[f32] {
        &self.audio_buffer
    }
}
```

---

## 📐 Layout Considerations

### Option A: Replace Spectrum Area

When Milkdrop is active, replace the spectrum visualizer area with the Milkdrop render:

```
┌─────────────────────────────────────────┐
│ [Album Art] [Play] [Pause] [Stop]      │
│                                         │
│ [========== Milkdrop Render ==========] │
│ [        800x600 visualization        ] │
│ [=====================================] │
│                                         │
│ Visualizer: [Spectrum] [Milkdrop] ✓    │
└─────────────────────────────────────────┘
```

### Option B: Dedicated Area

Add a dedicated area below the player section:

```
┌─────────────────────────────────────────┐
│ [Album Art] [Play] [Pause] [Stop]      │
│ [Progress Bar]                          │
│                                         │
│ Visualizer: [Spectrum] [Milkdrop] ✓    │
│                                         │
│ [========== Milkdrop Render ==========] │
│ [        800x600 visualization        ] │
│ [=====================================] │
│                                         │
│ 🎚 Equalizer                            │
└─────────────────────────────────────────┘
```

**Recommendation**: Option B (dedicated area) for better UX.

---

## 🎮 Fullscreen Mode

Add a button to toggle fullscreen visualizer:

```rust
if ui.button("🖵 Fullscreen").clicked() {
    self.visualizer_fullscreen = !self.visualizer_fullscreen;
}

if self.visualizer_fullscreen {
    // Render in a separate window or full central panel
    egui::CentralPanel::default().show(ctx, |ui| {
        // Full screen Milkdrop render
        ui.image(egui::load::SizedTexture::new(
            texture_id,
            ui.available_size()  // Fill available space
        ));
    });
}
```

---

## 📊 Performance Monitoring

Add FPS counter to monitor performance:

```rust
struct OneAmpApp {
    // ...
    frame_times: VecDeque<f32>,
    show_fps: bool,
}

impl OneAmpApp {
    fn update_fps(&mut self, delta_time: f32) {
        self.frame_times.push_back(delta_time);
        if self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }
    }
    
    fn get_fps(&self) -> f32 {
        let avg_time: f32 = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
        1.0 / avg_time
    }
}

// In UI
if self.show_fps {
    ui.label(format!("FPS: {:.1}", self.get_fps()));
}
```

---

## 🧪 Testing Strategy

### Test 1: Texture Registration
```rust
#[test]
fn test_texture_registration() {
    // Verify texture can be registered
    // Verify TextureId is valid
}
```

### Test 2: Audio Samples Flow
```rust
#[test]
fn test_audio_samples_to_onedrop() {
    // Verify audio samples reach OneDrop
    // Verify format is correct (f32, 44100 Hz)
}
```

### Test 3: Performance
```rust
#[test]
fn test_performance() {
    // Verify FPS stays above 30
    // Verify memory usage is reasonable
}
```

---

## ⚠️ Potential Issues

### Issue 1: Texture Format Mismatch

OneDrop renders to a specific wgpu texture format. egui might expect a different format.

**Solution**: Check texture format and convert if needed.

### Issue 2: Texture Size

OneDrop renders at 800x600 by default. This might not fit well in the UI.

**Solution**: Make size configurable or scale the image in egui.

### Issue 3: Performance

Rendering Milkdrop + egui UI might be too heavy.

**Solution**: 
- Lower Milkdrop resolution (640x480)
- Disable per-pixel shaders
- Monitor FPS and adjust

### Issue 4: Audio Latency

Audio samples might not sync with visuals.

**Solution**: Use a small buffer (e.g., 512 samples) for low latency.

---

## 📝 Implementation Checklist

Phase 2 Implementation:

- [ ] Add `get_audio_samples()` to `Visualizer`
- [ ] Access `RenderState` in `OneAmpApp::update()`
- [ ] Register OneDrop texture with `register_native_texture`
- [ ] Cache `TextureId` to avoid re-registration
- [ ] Display texture in UI with `ui.image()`
- [ ] Update OneDrop with audio samples every frame
- [ ] Add fullscreen toggle button
- [ ] Add FPS counter
- [ ] Test on real hardware
- [ ] Document performance characteristics

---

## 🎯 Success Criteria

Phase 2 is complete when:

1. ✅ OneDrop texture is visible in OneAmp UI
2. ✅ Visualization reacts to music in real-time
3. ✅ FPS stays above 30 (preferably 60)
4. ✅ Fullscreen mode works
5. ✅ No crashes or memory leaks
6. ✅ Tests pass

---

## 📚 References

- [egui PR #1660](https://github.com/emilk/egui/pull/1660) - `register_native_texture`
- [egui Discussion #1663](https://github.com/emilk/egui/discussions/1663) - Texture rendering
- [eframe Frame docs](https://docs.rs/eframe/latest/eframe/struct.Frame.html) - wgpu_render_state()
- [egui-wgpu RenderState](https://docs.rs/egui-wgpu/latest/egui_wgpu/struct.RenderState.html)

---

**Ready for implementation!**
