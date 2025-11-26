# Plan d'implémentation du Seek avec Symphonia

## Analyse de l'exemple symphonia-play

D'après le code de symphonia-play/src/main.rs (lignes 296-314), voici comment implémenter le seek :

```rust
// 1. Créer un SeekTo avec le temps en secondes
let seek_to = SeekTo::Time { 
    time: Time::from(seconds), 
    track_id: Some(track_id) 
};

// 2. Appeler reader.seek() avec SeekMode::Accurate
match reader.seek(SeekMode::Accurate, seek_to) {
    Ok(seeked_to) => {
        // seeked_to.required_ts contient le timestamp exact
        // Utiliser ce timestamp pour sauter les samples appropriés
    }
    Err(Error::ResetRequired) => {
        // Le format reader nécessite un reset
        // Recharger les tracks
    }
    Err(err) => {
        // Erreur de seek - continuer quand même
    }
}
```

## Problème avec l'architecture actuelle

Notre architecture actuelle utilise **rodio** qui :
- Prend un `Decoder` et le consomme dans un `Sink`
- Ne garde PAS de référence au `FormatReader`
- Ne permet PAS d'accéder au FormatReader après création du Sink

## Solution : Refactoriser pour utiliser Symphonia directement

### Option A : Remplacer rodio par symphonia + cpal (comme symphonia-play)

**Avantages :**
- Seek précis et natif
- Contrôle total sur le décodage
- Pas de limitations de rodio

**Inconvénients :**
- Refactoring majeur
- Doit gérer manuellement le resampling
- Doit gérer manuellement l'output audio avec cpal

### Option B : Architecture hybride (rodio pour playback, symphonia pour seek)

**Avantages :**
- Garde rodio pour la simplicité du playback
- Utilise symphonia uniquement pour le seek

**Inconvénients :**
- Architecture complexe
- Doit maintenir 2 décodeurs en parallèle
- Risque de désynchronisation

## Recommandation : Option A (Remplacer rodio)

C'est le bon moment pour migrer vers symphonia + cpal car :
1. Le projet est encore jeune (v0.13.1)
2. On a déjà symphonia comme dépendance
3. symphonia-play fournit un excellent exemple
4. Cela débloque le seek précis ET d'autres fonctionnalités futures

## Plan d'implémentation

### Phase 1 : Créer un nouveau module audio basé sur symphonia
- [ ] Créer `oneamp-core/src/symphonia_player.rs`
- [ ] Implémenter le décodage avec symphonia
- [ ] Implémenter l'output avec cpal (ou garder rodio::Sink pour l'output)
- [ ] Tester le playback basique

### Phase 2 : Implémenter le seek
- [ ] Garder une référence au FormatReader
- [ ] Implémenter AudioCommand::Seek avec reader.seek()
- [ ] Gérer le reset du decoder après seek
- [ ] Tester le seek

### Phase 3 : Intégrer avec l'equalizer et la visualisation
- [ ] Adapter EqualizerSource pour symphonia
- [ ] Adapter AudioCaptureSource pour symphonia
- [ ] Tester l'intégration complète

### Phase 4 : Migration et tests
- [ ] Remplacer l'ancien système par le nouveau
- [ ] Ajouter des tests unitaires
- [ ] Tester sur différents formats audio
- [ ] Pusher vers GitHub

## Estimation
- Phase 1 : 2-3 heures
- Phase 2 : 1 heure
- Phase 3 : 1 heure
- Phase 4 : 1 heure
- **Total : 5-6 heures**

## Alternative rapide (si le temps presse)

Implémenter un seek "approximatif" qui :
1. Arrête la lecture
2. Recharge le fichier depuis le début
3. Skip les N premières secondes de samples

C'est ce que fait actuellement notre implémentation, mais elle ne marche pas car `skip_one()` ne fait que sauter 1 packet, pas N secondes.

Pour un vrai skip approximatif :
```rust
// Recharger le fichier
let source = Decoder::new(file)?;
// Calculer combien de samples à skip
let sample_rate = 44100; // ou obtenir du decoder
let samples_to_skip = (seconds * sample_rate as f32) as usize;
// Utiliser .skip() pour sauter les samples
let skipped_source = source.skip(samples_to_skip);
```

Mais cela reste imprécis et ne fonctionne pas bien avec les formats compressés.
