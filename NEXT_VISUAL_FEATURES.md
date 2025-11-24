# OneAmp - Prochaines Fonctionnalités Visuelles (v0.9+)

## 🎯 Analyse des Priorités

Maintenant que nous avons les **fondations visuelles** (effets, widgets, animations), voici les prochaines étapes logiques pour se rapprocher de Winamp Modern.

## 🏆 Top 3 Fonctionnalités Prioritaires

### 1. **Égaliseur Visuel Avancé** 🔥 PRIORITÉ #1

**Pourquoi c'est la priorité #1** :
- L'égaliseur est **déjà fonctionnel** mais visuellement basique
- Impact visuel **énorme** avec peu d'effort
- Utilise directement nos nouveaux effets visuels
- Caractéristique iconique de Winamp

**Ce qui manque actuellement** :
```rust
// Actuel : Sliders egui basiques
egui::Slider::new(gain, -12.0..=12.0)
    .vertical()
    .show_value(false)
```

**Ce qu'on veut** :
- Sliders 3D avec effet métallique
- Indicateurs de niveau avec dégradés (vert → jaune → rouge)
- Peak indicators (petits traits qui restent au sommet)
- Panneau avec effet verre/acrylique
- Labels de fréquence stylisés
- Preset selector avec boutons 3D
- Visualisation en temps réel des gains

**Complexité** : ⭐⭐ Moyenne  
**Impact visuel** : ⭐⭐⭐⭐⭐ Très élevé  
**Temps estimé** : 3-4 heures

**Implémentation** :
```rust
// oneamp-desktop/src/equalizer_display.rs
pub struct EqualizerDisplay {
    peak_values: Vec<f32>,        // Peak indicators
    current_values: Vec<f32>,     // Current gain values
    peak_decay: f32,              // Decay speed for peaks
    animation_timer: AnimationTimer,
}

impl EqualizerDisplay {
    pub fn render_fancy(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        eq_gains: &mut Vec<f32>,
        eq_frequencies: &[f32],
    ) -> bool {
        // Glass panel background
        VisualEffects::glass_panel(...);
        
        // For each frequency band
        for (i, gain) in eq_gains.iter_mut().enumerate() {
            // 3D slider with metallic effect
            render_eq_slider_3d(ui, gain, ...);
            
            // Level indicator with gradient
            render_level_indicator(ui, *gain, ...);
            
            // Peak indicator (stays at max)
            render_peak_indicator(ui, self.peak_values[i], ...);
            
            // Frequency label with engraved effect
            render_frequency_label(ui, eq_frequencies[i], ...);
        }
        
        // Preset selector
        render_preset_selector(ui, ...);
    }
}
```

**Effets utilisés** :
- `VisualEffects::glass_panel()` pour le fond
- `VisualEffects::metallic_panel()` pour les sliders
- `VisualEffects::gradient_rect_vertical()` pour indicateurs
- `VisualEffects::glow()` pour peaks
- `custom_widgets::button_3d()` pour presets

---

### 2. **Boutons de Contrôle Personnalisés** 🎮 PRIORITÉ #2

**Pourquoi c'est important** :
- Les boutons Play/Pause/Stop sont **l'interaction principale**
- Actuellement : boutons egui par défaut (basiques)
- Winamp Modern a des boutons iconiques et stylisés

**Ce qu'on veut** :
- Boutons circulaires ou arrondis (pas rectangulaires)
- Icônes vectorielles (triangles, carrés, doubles barres)
- Effet de pression 3D au clic
- Glow au survol avec couleur thème
- Animation de rotation pour "loading"
- Indicateur visuel de l'état (playing = glow animé)

**Complexité** : ⭐⭐ Moyenne  
**Impact visuel** : ⭐⭐⭐⭐ Élevé  
**Temps estimé** : 2-3 heures

**Implémentation** :
```rust
// oneamp-desktop/src/control_buttons.rs
pub enum ButtonIcon {
    Play,      // Triangle pointant droite
    Pause,     // Deux barres verticales
    Stop,      // Carré
    Previous,  // Double triangle gauche
    Next,      // Double triangle droite
}

impl ButtonIcon {
    fn draw(&self, painter: &Painter, center: Pos2, size: f32, color: Color32) {
        match self {
            ButtonIcon::Play => {
                // Triangle avec vertices
                let points = vec![
                    center + vec2(-size/3.0, -size/2.0),
                    center + vec2(-size/3.0, size/2.0),
                    center + vec2(size/2.0, 0.0),
                ];
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
            }
            ButtonIcon::Pause => {
                // Deux rectangles
                let bar_width = size / 5.0;
                let bar_height = size;
                // ... dessiner les barres
            }
            // ... autres icônes
        }
    }
}

pub fn control_button(
    ui: &mut Ui,
    theme: &Theme,
    icon: ButtonIcon,
    active: bool,
) -> Response {
    let size = 48.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(size),
        Sense::click()
    );
    
    let painter = ui.painter();
    
    // Bouton circulaire 3D
    let center = rect.center();
    let radius = size / 2.0;
    
    // Shadow
    if !response.clicked() {
        painter.circle_filled(
            center + vec2(2.0, 2.0),
            radius,
            Color32::from_black_alpha(100),
        );
    }
    
    // Button body with gradient
    draw_circular_gradient(painter, center, radius, ...);
    
    // Glow if active or hovered
    if active || response.hovered() {
        VisualEffects::glow(painter, rect, radius, 8.0, accent_color);
    }
    
    // Icon
    icon.draw(painter, center, size * 0.4, icon_color);
    
    response
}
```

**Bonus** : Animation de pulsation pour le bouton Play actif

---

### 3. **Custom Window Chrome** 🪟 PRIORITÉ #3

**Pourquoi c'est logique** :
- Winamp Modern a une barre de titre personnalisée
- Permet de **contrôler totalement** l'apparence
- Intégration parfaite avec le thème

**Ce qu'on veut** :
- Barre de titre personnalisée avec dégradé
- Boutons minimize/maximize/close stylisés
- Titre de l'application avec police personnalisée
- Icône de l'app dans la barre
- Double-clic pour maximize
- Drag pour déplacer la fenêtre

**Complexité** : ⭐⭐⭐⭐ Élevée  
**Impact visuel** : ⭐⭐⭐ Moyen  
**Temps estimé** : 4-6 heures

**Défis avec egui** :
- egui ne supporte **pas nativement** le custom window chrome
- Nécessite d'utiliser `eframe::NativeOptions::decorated = false`
- Puis recréer **toute** la logique de fenêtre

**Implémentation** :
```rust
// main.rs
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)  // Pas de barre de titre OS
            .with_transparent(true)   // Fond transparent
            .with_min_inner_size([600.0, 500.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "OneAmp",
        options,
        Box::new(|cc| Ok(Box::new(OneAmpApp::new(cc)))),
    )
}

// oneamp-desktop/src/window_chrome.rs
pub struct WindowChrome {
    dragging: bool,
    drag_offset: Vec2,
}

impl WindowChrome {
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        theme: &Theme,
        title: &str,
    ) {
        egui::TopBottomPanel::top("title_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // App icon
                ui.label("🎵");
                
                // Title
                ui.label(
                    egui::RichText::new(title)
                        .size(14.0)
                        .color(theme.colors.display_text)
                );
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Close button
                    if custom_widgets::window_button(ui, "×", theme).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    
                    // Maximize button
                    if custom_widgets::window_button(ui, "□", theme).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    }
                    
                    // Minimize button
                    if custom_widgets::window_button(ui, "−", theme).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                });
            });
            
            // Handle dragging
            let response = ui.interact(
                ui.max_rect(),
                ui.id().with("drag_area"),
                egui::Sense::drag(),
            );
            
            if response.dragged() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        });
    }
}
```

**Note** : Complexe car nécessite de gérer manuellement :
- Resize de la fenêtre (coins et bords)
- Double-clic pour maximize
- Snap to screen edges
- Multi-monitor support

---

## 📊 Comparaison des Options

| Fonctionnalité | Complexité | Impact | Temps | Utilise v0.8 | Priorité |
|----------------|------------|--------|-------|--------------|----------|
| **Égaliseur Avancé** | ⭐⭐ | ⭐⭐⭐⭐⭐ | 3-4h | ✅✅✅ | 🔥 #1 |
| **Boutons Contrôle** | ⭐⭐ | ⭐⭐⭐⭐ | 2-3h | ✅✅ | 🎮 #2 |
| **Window Chrome** | ⭐⭐⭐⭐ | ⭐⭐⭐ | 4-6h | ✅ | 🪟 #3 |
| Playlist Avancée | ⭐⭐⭐ | ⭐⭐⭐ | 3-4h | ✅✅ | #4 |
| Visualiseur Oscilloscope | ⭐⭐ | ⭐⭐⭐ | 2h | ✅✅ | #5 |
| Système de Skins | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | 8-12h | ✅ | #6 |

## 🎯 Recommandation : Égaliseur Avancé (v0.9)

**Pourquoi commencer par l'égaliseur** :

1. **ROI maximal** : Ratio impact/complexité le plus élevé
2. **Utilise v0.8** : Exploite directement tous les nouveaux effets
3. **Déjà fonctionnel** : On améliore l'existant, pas de nouvelle logique
4. **Iconique Winamp** : L'égaliseur est LA fonctionnalité signature
5. **Rapide** : 3-4 heures vs 4-6h pour window chrome

**Ce que ça apporterait** :
- Sliders métalliques 3D avec reflets
- Indicateurs de niveau avec dégradés dynamiques
- Peak indicators animés
- Panneau verre/acrylique
- Preset selector avec boutons 3D
- Visualisation temps réel

**Progression logique** :
```
v0.8 : Fondations visuelles (effets, widgets, animations)
  ↓
v0.9 : Égaliseur avancé (utilise tout v0.8)
  ↓
v1.0 : Boutons contrôle + Playlist avancée
  ↓
v1.1 : Window chrome personnalisé
  ↓
v2.0 : Système de skins complet
```

## 💡 Autres Idées Intéressantes

### Mini-Player Mode
- Vue compacte (200x100px)
- Seulement timer + contrôles + visualiseur
- Always-on-top
- Complexité : ⭐⭐ | Impact : ⭐⭐⭐

### Spectrum Analyzer Fullscreen
- Mode plein écran pour le visualiseur
- Effets de particules
- Synchronisation avec beat detection
- Complexité : ⭐⭐⭐⭐ | Impact : ⭐⭐⭐⭐⭐

### Album Art Display
- Affichage de la pochette d'album
- Extraction depuis tags ID3
- Effet de reflet en bas (iTunes style)
- Complexité : ⭐⭐ | Impact : ⭐⭐⭐⭐

### Lyrics Display
- Affichage des paroles synchronisées
- Support .lrc files
- Auto-scroll avec highlight
- Complexité : ⭐⭐⭐ | Impact : ⭐⭐⭐

## 🚀 Plan d'Action Recommandé

### Phase 1 : v0.9 (Égaliseur Avancé)
**Durée** : 1 session (3-4h)
- Créer `equalizer_display.rs`
- Sliders 3D métalliques
- Indicateurs de niveau
- Peak indicators
- Preset selector

### Phase 2 : v1.0 (Contrôles + Playlist)
**Durée** : 2 sessions (5-6h)
- Boutons de contrôle personnalisés
- Playlist avec animations
- Album art display

### Phase 3 : v1.1 (Window Chrome)
**Durée** : 1-2 sessions (4-6h)
- Barre de titre personnalisée
- Boutons window
- Drag & resize

### Phase 4 : v2.0 (Skins System)
**Durée** : 3-4 sessions (8-12h)
- Format de skin .toml
- Skin loader
- Skin editor UI
- Import Winamp .wsz (optionnel)

## ✨ Conclusion

**Prochaine étape recommandée** : **Égaliseur Visuel Avancé (v0.9)**

C'est le choix optimal car :
- ✅ Impact visuel maximal
- ✅ Complexité raisonnable
- ✅ Utilise pleinement v0.8
- ✅ Fonctionnalité iconique de Winamp
- ✅ Rapide à implémenter (3-4h)

Voulez-vous que je procède avec l'implémentation de l'égaliseur avancé pour la v0.9 ?
