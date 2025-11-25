# Analyse d'Intégration OneDrop dans OneAmp

## 📊 Vue d'Ensemble

OneDrop est un moteur de visualisation Milkdrop écrit en Rust avec une architecture modulaire en 6 crates. L'objectif est de l'intégrer dans OneAmp comme visualiseur avancé.

## 🏗️ Architecture OneDrop

### Crates Disponibles
| Crate | Description | LOC | Dépendances Clés |
|-------|-------------|-----|------------------|
| `onedrop-parser` | Parse .milk files | 700 | - |
| `onedrop-eval` | Expression evaluation | 950 | evalexpr |
| `onedrop-renderer` | GPU rendering | 1220 | wgpu |
| `onedrop-engine` | Visualization engine | 1450 | wgpu, tokio |
| `onedrop-cli` | CLI interface | 350 | clap |
| `onedrop-gui` | GUI application | 400 | egui, eframe |

**Total** : ~5070 lignes

### API Principale (onedrop-engine)

```rust
use onedrop_engine::{EngineConfig, MilkEngine};

let config = EngineConfig::default();
let mut engine = MilkEngine::new(config).await?;

engine.load_preset("preset.milk")?;

loop {
    let audio_samples = capture_audio();
    engine.update(&audio_samples, 0.016)?;
    display(engine.render_texture());
}
```

## 🔌 Stratégies d'Intégration

### Option 1 : Intégration Complète (Recommandée)

**Approche** : Ajouter `onedrop-engine` comme dépendance de `oneamp-desktop`

**Avantages** :
- ✅ Utilise l'API stable de OneDrop
- ✅ Pas de duplication de code
- ✅ Mises à jour faciles
- ✅ Tests déjà existants

**Inconvénients** :
- ⚠️ Dépendance externe (mais c'est votre projet)
- ⚠️ Nécessite wgpu (déjà utilisé par egui)

**Implémentation** :
```toml
# oneamp-desktop/Cargo.toml
[dependencies]
onedrop-engine = { path = "../../onedrop/onedrop-engine" }
```

---

### Option 2 : Copie des Crates

**Approche** : Copier les crates onedrop dans oneamp comme sous-modules

**Avantages** :
- ✅ Contrôle total
- ✅ Pas de dépendance externe

**Inconvénients** :
- ❌ Duplication de code
- ❌ Maintenance difficile
- ❌ Perte de synchronisation

**Non recommandé**

---

### Option 3 : Git Submodule

**Approche** : Ajouter onedrop comme submodule git

**Avantages** :
- ✅ Pas de duplication
- ✅ Synchronisation facile

**Inconvénients** :
- ⚠️ Complexité git
- ⚠️ Submodules parfois problématiques

---

## 🎯 Recommandation : Option 1

**Utiliser `onedrop-engine` comme dépendance avec path local**

### Étapes d'Intégration

#### 1. Ajouter la Dépendance

```toml
# oneamp-desktop/Cargo.toml
[dependencies]
onedrop-engine = { path = "../../onedrop/onedrop-engine" }
wgpu = "22.1"  # Si pas déjà présent
```

#### 2. Créer le Module d'Intégration

```rust
// oneamp-desktop/src/onedrop_visualizer.rs

use onedrop_engine::{EngineConfig, MilkEngine};
use std::path::PathBuf;

pub struct OneDropVisualizer {
    engine: Option<MilkEngine>,
    current_preset: Option<PathBuf>,
    presets: Vec<PathBuf>,
    current_index: usize,
}

impl OneDropVisualizer {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let config = EngineConfig {
            width: 800,
            height: 600,
            ..Default::default()
        };
        
        let engine = MilkEngine::new(config).await?;
        
        Ok(Self {
            engine: Some(engine),
            current_preset: None,
            presets: Vec::new(),
            current_index: 0,
        })
    }
    
    pub fn load_presets(&mut self, preset_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        // Scan directory for .milk files
        self.presets = std::fs::read_dir(preset_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().map_or(false, |ext| ext == "milk"))
            .collect();
        
        Ok(())
    }
    
    pub fn update(&mut self, audio_samples: &[f32], delta_time: f32) {
        if let Some(ref mut engine) = self.engine {
            let _ = engine.update(audio_samples, delta_time);
        }
    }
    
    pub fn render(&self) -> Option<&wgpu::Texture> {
        self.engine.as_ref().map(|e| e.render_texture())
    }
    
    pub fn next_preset(&mut self) {
        if self.presets.is_empty() {
            return;
        }
        
        self.current_index = (self.current_index + 1) % self.presets.len();
        if let Some(engine) = &mut self.engine {
            let _ = engine.load_preset(&self.presets[self.current_index]);
        }
    }
    
    pub fn previous_preset(&mut self) {
        if self.presets.is_empty() {
            return;
        }
        
        self.current_index = if self.current_index == 0 {
            self.presets.len() - 1
        } else {
            self.current_index - 1
        };
        
        if let Some(engine) = &mut self.engine {
            let _ = engine.load_preset(&self.presets[self.current_index]);
        }
    }
}
```

#### 3. Intégrer dans OneAmpApp

```rust
// oneamp-desktop/src/main.rs

struct OneAmpApp {
    // ... existing fields ...
    
    // Visualizers
    visualizer: Visualizer,  // Spectrum analyzer (existing)
    onedrop: Option<OneDropVisualizer>,  // Milkdrop visualizer
    use_onedrop: bool,  // Toggle between visualizers
}

impl OneAmpApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ... existing code ...
        
        // Initialize OneDrop asynchronously
        let onedrop = pollster::block_on(async {
            OneDropVisualizer::new().await.ok()
        });
        
        Self {
            // ... existing fields ...
            visualizer: Visualizer::new(),
            onedrop,
            use_onedrop: false,
        }
    }
}

impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ... existing code ...
        
        // Update visualizer
        if self.use_onedrop {
            if let Some(ref mut onedrop) = self.onedrop {
                let audio_samples = self.visualizer.get_audio_samples();
                onedrop.update(&audio_samples, ctx.input(|i| i.unstable_dt));
            }
        } else {
            // Use existing spectrum visualizer
        }
        
        // Render visualizer
        ui.horizontal(|ui| {
            if ui.button(if self.use_onedrop { "Spectrum" } else { "Milkdrop" }).clicked() {
                self.use_onedrop = !self.use_onedrop;
            }
            
            if self.use_onedrop {
                if ui.button("◄").clicked() {
                    if let Some(ref mut onedrop) = self.onedrop {
                        onedrop.previous_preset();
                    }
                }
                if ui.button("►").clicked() {
                    if let Some(ref mut onedrop) = self.onedrop {
                        onedrop.next_preset();
                    }
                }
            }
        });
        
        // Display visualizer
        if self.use_onedrop {
            if let Some(ref onedrop) = self.onedrop {
                if let Some(texture) = onedrop.render() {
                    // Render wgpu texture in egui
                    render_wgpu_texture(ui, texture);
                }
            }
        } else {
            // Render spectrum visualizer (existing)
            ui_components::render_player_section(...);
        }
    }
}
```

#### 4. Rendu de Texture wgpu dans egui

```rust
// oneamp-desktop/src/wgpu_texture_renderer.rs

use eframe::egui;
use wgpu;

pub fn render_wgpu_texture(ui: &mut egui::Ui, texture: &wgpu::Texture) {
    // Option 1: Utiliser egui_wgpu pour intégration directe
    // egui_wgpu peut afficher des textures wgpu directement
    
    // Option 2: Copier la texture vers une image egui
    // Plus simple mais moins performant
    
    // Pour l'instant, utiliser un placeholder
    ui.label("OneDrop Visualizer");
    ui.add(egui::widgets::Image::new(egui::include_image!("../../icon_256.png")));
}
```

---

## 🎨 Interface Utilisateur

### Toggle Visualiseur

```
┌─────────────────────────────────────────┐
│ Visualizer: [Spectrum] [Milkdrop]      │
│                                         │
│  [Milkdrop actif]                      │
│  Preset: Flexi - Mindblob Reflecto... │
│  [◄] [►] [Random]                      │
│                                         │
│  [Visualisation plein écran]           │
└─────────────────────────────────────────┘
```

### Raccourcis Clavier

- `V` : Toggle Spectrum/Milkdrop
- `←/→` : Preset précédent/suivant (si Milkdrop)
- `R` : Random preset (si Milkdrop)
- `F` : Fullscreen visualizer

---

## 🔧 Défis Techniques

### 1. Intégration wgpu ↔ egui

**Problème** : OneDrop utilise wgpu directement, egui utilise son propre backend

**Solutions** :
- **Option A** : Utiliser `egui_wgpu` qui permet l'intégration
- **Option B** : Copier la texture wgpu vers une image egui (CPU overhead)
- **Option C** : Fenêtre séparée pour OneDrop (plus simple)

**Recommandation** : Option A (egui_wgpu)

### 2. Async Initialization

**Problème** : `MilkEngine::new()` est async

**Solution** : Utiliser `pollster::block_on()` dans `OneAmpApp::new()`

### 3. Audio Samples Format

**Problème** : Format des samples audio peut différer

**Solution** : Adapter le format dans `OneDropVisualizer::update()`

### 4. Performance

**Problème** : OneDrop peut être gourmand en GPU

**Solution** :
- Limiter la résolution (800x600 par défaut)
- Option pour désactiver
- Monitoring FPS

---

## 📦 Dépendances Additionnelles

```toml
# oneamp-desktop/Cargo.toml

[dependencies]
# OneDrop integration
onedrop-engine = { path = "../../onedrop/onedrop-engine" }
wgpu = "22.1"
pollster = "0.3"  # For blocking on async

# Optional: for better wgpu integration
egui_wgpu = "0.30"
```

---

## 🧪 Tests

### Tests d'Intégration

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_onedrop_visualizer_creation() {
        let visualizer = pollster::block_on(async {
            OneDropVisualizer::new().await
        });
        
        assert!(visualizer.is_ok());
    }
    
    #[test]
    fn test_preset_loading() {
        let mut visualizer = pollster::block_on(async {
            OneDropVisualizer::new().await.unwrap()
        });
        
        let preset_dir = PathBuf::from("../../onedrop/test-presets");
        let result = visualizer.load_presets(&preset_dir);
        
        assert!(result.is_ok());
        assert!(!visualizer.presets.is_empty());
    }
}
```

---

## 📊 Estimation

### Temps de Développement
- **Module d'intégration** : 2-3 heures
- **UI toggle** : 1 heure
- **Tests** : 1 heure
- **Debug & polish** : 2 heures
- **Total** : 6-7 heures

### Complexité
- **Technique** : ⭐⭐⭐⭐ (Élevée - intégration wgpu)
- **Architecture** : ⭐⭐⭐ (Moyenne)
- **Tests** : ⭐⭐ (Faible)

### Impact
- **Visuel** : ⭐⭐⭐⭐⭐ (Énorme - Milkdrop!)
- **Performance** : ⭐⭐⭐ (Moyen - GPU intensif)
- **UX** : ⭐⭐⭐⭐⭐ (Excellent)

---

## 🚀 Plan d'Action

### Phase 1 : Setup (v0.10.0)
1. Ajouter dépendance `onedrop-engine`
2. Créer module `onedrop_visualizer.rs`
3. Initialisation basique dans `OneAmpApp`
4. Toggle UI Spectrum/Milkdrop

### Phase 2 : Intégration (v0.10.1)
1. Intégration wgpu texture rendering
2. Audio samples feeding
3. Preset navigation (←/→)
4. Keyboard shortcuts

### Phase 3 : Polish (v0.10.2)
1. Preset browser UI
2. Random preset
3. Fullscreen mode
4. Performance monitoring

### Phase 4 : Advanced (v0.11.0)
1. Preset favorites
2. Transition effects
3. Custom presets
4. Beat detection visualization

---

## ✅ Prêt pour Implémentation

**Recommandation** : Commencer par la Phase 1 (Setup) pour v0.10.0

**Objectif** : Toggle fonctionnel entre Spectrum et Milkdrop avec preset navigation basique

**Temps estimé** : 3-4 heures

Voulez-vous que je procède avec l'implémentation ?
