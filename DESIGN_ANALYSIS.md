# Analyse du Design Winamp Modern pour OneAmp v0.7

## Observations du Screenshot

### Layout Principal (de haut en bas)

1. **Barre de titre** : "WACUP" avec menu (File, Play, Options, View, Help)
2. **Zone d'affichage principale** (display area)
   - Timer digital (00:02) en gros
   - Informations de lecture (bitrate, sample rate, etc.)
   - Visualiseur audio (spectre) à droite
3. **Zone de texte défilant** : "DJ MIKE LLAMA - LLAMA WHIPPIN'"
4. **Barre de progression** : Slider horizontal avec volume à droite
5. **Contrôles de lecture** : Previous, Play, Pause, Stop, Next, avec boutons additionnels

### Palette de Couleurs

- **Fond principal** : Bleu foncé/gris métallique
- **Affichage numérique** : Bleu clair lumineux (LCD style)
- **Texte défilant** : Bleu clair sur fond bleu foncé
- **Boutons** : Style métallique avec relief
- **Accents** : Orange/doré pour certains éléments

### Typographie

- **Timer** : Police digitale/LCD style, grande taille
- **Texte défilant** : Police monospace ou sans-serif, style rétro
- **Menu** : Police système standard

## Architecture pour OneAmp v0.7

### Structure Verticale

```
┌─────────────────────────────────────┐
│  Menu Bar (File, Play, View, etc.) │
├─────────────────────────────────────┤
│  PLAYER SECTION                     │
│  ├─ Timer Display (large)           │
│  ├─ Track Info (scrolling text)     │
│  ├─ Visualizer                      │
│  └─ Progress Bar (interactive)      │
├─────────────────────────────────────┤
│  CONTROLS                           │
│  └─ Play/Pause/Stop/Prev/Next       │
├─────────────────────────────────────┤
│  EQUALIZER SECTION                  │
│  └─ 10-band EQ sliders              │
├─────────────────────────────────────┤
│  PLAYLIST SECTION                   │
│  └─ Track list with drag-drop       │
└─────────────────────────────────────┘
```

### Fonctionnalités Clés

1. **Barre de progression interactive** : Cliquer pour se déplacer dans la piste
2. **Tags ID3** : Afficher "ARTIST - TITLE" au lieu de "Unknown"
3. **Drag-and-drop** : Glisser des fichiers vers la playlist
4. **Thème moddable** : Système de configuration JSON/TOML

## Plan d'Implémentation

### Phase 1 : Système de Thèmes
- Créer une structure `Theme` avec couleurs, polices, dimensions
- Charger depuis un fichier `theme.toml`
- Appliquer dynamiquement à l'UI

### Phase 2 : Tags ID3
- Utiliser la crate `id3` ou `lofty` pour lire les métadonnées
- Mettre à jour `TrackInfo` pour inclure artist/title/album
- Afficher dans le format "ARTIST - TITLE"

### Phase 3 : Layout Refonte
- Réorganiser le layout vertical (player/eq/playlist)
- Améliorer le style visuel (couleurs Winamp-like)
- Ajouter une police digitale pour le timer

### Phase 4 : Barre de Progression Interactive
- Implémenter le clic sur la barre pour seek
- Retirer le % et le spinner
- Style minimaliste et élégant

### Phase 5 : Drag-and-Drop
- Utiliser les capacités egui pour le drag-drop de fichiers
- Supporter les fichiers individuels et dossiers
- Afficher les tags ID3 dans la playlist
