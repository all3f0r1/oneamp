# Analyse de l'architecture actuelle OneAmp

## Architecture actuelle (avec rodio)

### Flux de données
```
AudioCommand::Play(path)
    ↓
TrackInfo::from_file() → Utilise symphonia pour metadata
    ↓
load_and_play() → Crée Decoder (rodio) depuis File
    ↓
EqualizerSource → Wrapper rodio::Source
    ↓
AudioCaptureSource → Wrapper rodio::Source
    ↓
Sink::append() → Consomme le Source
    ↓
rodio OutputStream → Playback audio
```

### Problèmes identifiés

1. **Pas de FormatReader persistant** : Le Decoder de rodio consomme le fichier, on ne garde pas de référence au FormatReader de symphonia
2. **Seek impossible** : rodio::Sink ne supporte pas le seek
3. **Double usage de symphonia** : On utilise symphonia pour les metadata mais rodio (qui utilise aussi symphonia) pour le playback

### Modules existants

- **lib.rs** : AudioEngine, AudioCommand, AudioEvent, TrackInfo, audio_thread_main()
- **equalizer.rs** : Equalizer avec filtres biquad (10 bandes)
- **eq_source.rs** : EqualizerSource (wrapper rodio::Source)
- **audio_capture.rs** : AudioCaptureBuffer + AudioCaptureSource (pour visualisation)

## Nouvelle architecture (avec symphonia + cpal)

### Flux de données proposé
```
AudioCommand::Play(path)
    ↓
SymphoniaPlayer::load() → Crée FormatReader + Decoder
    ↓
SymphoniaPlayer::play_loop() → Boucle de décodage
    ↓
FormatReader::next_packet() → Lit packets
    ↓
Decoder::decode() → Décode en AudioBuffer
    ↓
Equalizer::process() → Applique EQ
    ↓
AudioCaptureBuffer::update() → Capture pour viz
    ↓
cpal Stream → Playback audio
```

### Avantages

1. **FormatReader persistant** : On garde une référence, donc seek possible
2. **Seek précis** : FormatReader::seek() avec SeekMode::Accurate
3. **Contrôle total** : On gère nous-mêmes le décodage et le playback
4. **Pas de double usage** : Un seul usage de symphonia

### Modules à créer/modifier

#### Nouveau : symphonia_player.rs
```rust
pub struct SymphoniaPlayer {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    equalizer: Arc<Mutex<Equalizer>>,
    capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
}

impl SymphoniaPlayer {
    pub fn load(path: &Path, eq: Arc<Mutex<Equalizer>>, capture: Arc<Mutex<AudioCaptureBuffer>>) -> Result<Self>;
    pub fn seek(&mut self, seconds: f32) -> Result<()>;
    pub fn decode_next(&mut self) -> Result<Option<AudioBuffer>>;
}
```

#### Nouveau : cpal_output.rs
```rust
pub struct CpalOutput {
    stream: cpal::Stream,
    sample_buffer: Arc<Mutex<VecDeque<f32>>>,
}

impl CpalOutput {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self>;
    pub fn write_samples(&self, samples: &[f32]);
    pub fn play(&self);
    pub fn pause(&self);
}
```

#### Modifier : lib.rs
- Remplacer `Sink` par `SymphoniaPlayer` + `CpalOutput`
- Adapter audio_thread_main() pour utiliser la nouvelle architecture
- Garder AudioCommand, AudioEvent, TrackInfo inchangés (API publique)

#### Modifier : equalizer.rs
- Ajouter méthode `process_buffer()` pour traiter des buffers entiers
- Optimiser pour éviter les allocations

#### Modifier : audio_capture.rs
- Adapter pour recevoir des buffers f32 directement (pas de conversion i16)
- Simplifier car on n'a plus besoin d'implémenter rodio::Source

## Plan de migration

### Phase 1 : Créer symphonia_player.rs
- Implémenter SymphoniaPlayer::load()
- Implémenter SymphoniaPlayer::decode_next()
- Tester le décodage basique

### Phase 2 : Créer cpal_output.rs
- Implémenter CpalOutput avec ring buffer
- Tester le playback basique
- Intégrer avec SymphoniaPlayer

### Phase 3 : Implémenter le seek
- Implémenter SymphoniaPlayer::seek()
- Gérer le reset du decoder
- Tester le seek

### Phase 4 : Intégrer EQ et capture
- Adapter Equalizer pour traiter des buffers
- Adapter AudioCaptureBuffer
- Intégrer dans la boucle de décodage

### Phase 5 : Migrer audio_thread_main()
- Remplacer Sink par SymphoniaPlayer + CpalOutput
- Adapter tous les AudioCommand
- Tester l'intégration complète

### Phase 6 : Cleanup
- Supprimer eq_source.rs (plus nécessaire)
- Simplifier audio_capture.rs
- Mettre à jour les dépendances dans Cargo.toml
- Ajouter tests unitaires

## Dépendances à ajouter

```toml
[dependencies]
cpal = "0.15"  # Pour l'output audio
ringbuf = "0.3"  # Pour le buffer circulaire (optionnel)
```

## Estimation du temps

- Phase 1 : 1-2h (création SymphoniaPlayer)
- Phase 2 : 1-2h (création CpalOutput + intégration)
- Phase 3 : 30min (implémentation seek)
- Phase 4 : 1h (intégration EQ + capture)
- Phase 5 : 1h (migration audio_thread_main)
- Phase 6 : 30min (cleanup + tests)

**Total : 5-6 heures**
