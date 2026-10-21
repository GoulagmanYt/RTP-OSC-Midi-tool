# Audit du moteur audio et du callback — 21 octobre 2026

## Objectif et méthode

Audit Windows x64 du trajet MIDI → VST → sortie, limité aux callbacks, à la
négociation du flux, au cycle de vie, au rendu anticipé et au réglage Windows.
Lecture du README et de l'architecture, puis vérification dans le code et les
manifestes. Aucun `AGENTS.md` sur disque ; les instructions fournies dans la
conversation ont été appliquées. État initial : seul `.playwright-cli/` était
non suivi ; ce dossier est préservé.

## Chemin reconstitué et contrôles

- Le processus principal route le MIDI vers le worker x64 par IPC. Le worker
  possède le périphérique CPAL et le plugin ; le pont x86 reste isolé.
- `stream_config.rs` choisit la fréquence supportée la plus proche, privilégie
  F32 et la stéréo, borne la taille demandée à la plage annoncée et dimensionne
  les buffers du plugin pour le maximum annoncé (8192 minimum si inconnu).
  `stream_runtime.rs` charge le plugin à cette fréquence et tente les tailles
  de repli puis la taille par défaut. `stream.buffer_size()` et la taille
  observée dans le callback alimentent des diagnostics distincts de la demande.
- `runtime_state.rs` préalloue les buffers audio et MIDI, borne la vidange,
  réserve des places aux messages critiques et invalide les générations après
  panic. `midi_playback.rs` maintient le temps source lors d'un retard ; les
  offsets restent ordonnés et dans le bloc. VST2 reçoit son lot préalloué ;
  VST3 reçoit les seuls événements admis et conserve le reliquat après rejet.
- Le callback utilise l'admission atomique et `try_lock`, avec silence ou
  fondu lors d'une indisponibilité. Les canaux du plugin sont bornés à la
  limite préallouée ; le mono est dupliqué, les deux premiers canaux sont
  envoyés et les canaux supplémentaires du périphérique sont mis à zéro.
- Le rendu passe par EQ → gain/limiteur → conversion. L'EQ existant assainit les
  valeurs non finies et son état. Le rendu anticipé échange des buffers
  préalloués ; l'attente du producteur appartient au thread DSP, pas au pilote.
- Le démarrage suspend le flux avant restauration, amorce le rendu puis lance
  le flux. L'arrêt ferme l'admission et attend hors callback avant destruction.
  Les gardes MMCSS appartiennent au thread natif ; la résolution du timer est
  libérée par sa garde. Les tests existants couvrent la réutilisation du thread
  par un autre flux et la remise à zéro des diagnostics MMCSS.
- CPAL est réellement fixé à **0.18.2**, dans le manifeste et le lockfile.
  L'architecture indiquait encore 0.18.1 : cette mention a été corrigée.
  Vérification dans les sources locales CPAL : un xrun est distinct d'une
  invalidation/reset ASIO ou d'un changement de fréquence. Le callback compte
  les xruns sans reconstruction ; les erreurs fatales demandent la récupération.
  Le heartbeat du worker tente alors un checkpoint avant de signaler la panne
  au superviseur. Aucun changement spéculatif dans ces mécanismes.

## Bugs confirmés et corrections

### 1. Audio périmé après panic avec une réserve vide

- Déclencheur : consommer entièrement un bloc non silencieux, laisser le
  producteur en pause, puis changer de génération MIDI ou signaler un reset.
- Attendu : silence immédiat, sans réutiliser l'audio de l'ancienne génération.
- Observé avant correction : pour un historique à 0,8, sortie
  `[0.6, 0.4, 0.2, 0.0]` après le reset.
- Cause : `AudioOutput::copy_into` vidait l'historique seulement lorsqu'il
  rencontrait un bloc périmé ; aucun bloc n'était disponible dans ce scénario.
- Correction : invalider aussi `last` et enclencher la reprise progressive dès
  l'observation du reset, avant de consulter la réserve.
- Fichier : `src-tauri/src/audio/render_ahead.rs`, autour de la ligne 342.
- Régression : `reset_with_an_empty_ready_queue_never_replays_previous_generation`,
  couvrant séparément la génération et le signal d'urgence.

### 2. Conversion PCM 24 bits hors plage avec le limiteur désactivé

- Déclencheur : échantillons flottants `[2.0, -2.0, 1.0, 0.5]`, sortie I24.
- Attendu : PCM borné `[8388607, -8388608, 8388607, 4194304]`.
- Observé avant correction : `[16777216, -16777216, 8388608, 4194304]`.
  Le repli sur historique présentait le même défaut.
- Cause : `dasp_sample` exige une entrée entière normalisée dans `[-1, 1)` ;
  son constructeur I24 est sans contrôle de plage. Le limiteur désactivé
  autorise légitimement une amplitude flottante supérieure à 1.
- Correction : conversion partagée `device_sample`, avec assainissement des
  non-finis et bornage réservé aux formats entiers. Les sorties F32/F64
  conservent leur amplitude. Les chemins direct, anticipé et de repli l'utilisent.
- Fichiers : `callback_render.rs:125`, `callback.rs:500`, `render_ahead.rs`
  dans les copies de sortie et de repli.
- Régressions : `integer_device_output_saturates_unlimited_plugin_samples`,
  `integer_fallback_saturates_audio_saved_with_the_limiter_disabled` et
  couverture des conversions F32/F64/I16/U16/I24/I32 et valeurs non finies.

### 3. Diagnostic fatal écrasé et récupération non comptée

- Déclencheur : callback du pilote plus grand que la capacité préallouée avec
  rendu anticipé, éventuellement après une première erreur de périphérique.
- Attendu : silence, signal de reset, première cause fatale conservée et une
  demande de récupération comptée, comme dans le chemin direct.
- Observé avant correction : compteur à zéro ; un code fatal antérieur était
  remplacé par 13 à chaque callback trop grand.
- Cause : écriture inconditionnelle du code fatal sans comptage.
- Correction : publication atomique seulement si aucun défaut n'est encore
  présent, avec incrément lors de cette première publication.
- Fichier : `src-tauri/src/audio/render_ahead.rs`, autour de la ligne 321.
- Régression :
  `oversized_driver_callback_requests_recovery_once_and_preserves_first_fault`.

## Vérifications

- Avant correction : `cargo test --manifest-path src-tauri/Cargo.toml --locked
  --lib --all-features audio::render_ahead::tests -- --nocapture` : 3 échecs
  attendus, 8 réussites. Journal : `artifacts/audio-callback-red.log`.
- Avant correction : même commande filtrée sur `integer_fallback` : échec
  attendu. Journal : `artifacts/audio-fallback-red.log`.
- Après correction : `cargo test --manifest-path src-tauri/Cargo.toml --locked
  --lib --all-features audio::` : **123 réussites, 0 échec, 3 ignorés**.
  Journal : `artifacts/audio-callback-green.log`.
  Les tests ignorés sont les tests tiers natifs déjà désactivés dans le dépôt,
  destinés aux diagnostics isolés. Aucun test ou contrôle n'a été désactivé.
- `cargo fmt --manifest-path src-tauri/Cargo.toml --package osc-midi-bridge`
  et `git diff --check` exécutés.
- Revue indépendante du diff : aucune anomalie critique ou importante relevée.
- `build_windows.bat` : **code 0**, pipeline complet, aucune erreur. Cela
  inclut lint/TypeScript, **141 tests frontend**, fmt/Clippy avec warnings
  interdits, **305 tests unitaires Rust** (3 ignorés préexistants), intégration,
  supervision, diagnostics, bibliothèques vendoriées et **18 tests C++ audio**.
  Le helper de test de jonction reste lui aussi ignoré dans son exécution
  directe, comme avant ; sa suite dédiée passe. Workers x64/x86, application,
  MSI et ZIP portable compilés et exportés dans `artifacts/`.
  Journal : `artifacts/logs/build-2026-10-21_18-16-30-OK.log` ; capture complète :
  `artifacts/audio-callback-windows-build.log`.
- `cargo build --manifest-path tools/diagnostics/Cargo.toml --locked --bin
  audio_reliability` : **code 0**. Pour ces commandes Cargo de diagnostic et
  de tests, `TAURI_CONFIG={"bundle":{"externalBin":[]}}` évite la nécessité
  d'embarquer les sidecars dans le binaire de test ; le build Windows complet
  utilise le packaging normal, et les essais suivants utilisent le worker
  réellement exporté.

### Essais natifs supplémentaires

Deux exécutions séquentielles du diagnostic existant, avec
`OSCMIDI_VST_WORKER_X64_PATH` pointant sur
`artifacts/portable/vst-host-worker-x64.exe` (profil release) :

```powershell
.cargo-target/debug/audio_reliability.exe --vst "C:/Program Files/VSTPlugins/Upright Piano Windows VST 64-bit/Upright Piano.dll" --device "Voicemeeter AUX Virtual ASIO" --buffer 512 --render-ahead-blocks 0 --notes 32 --events-per-second 20 --report artifacts/audio-callback-native-0.json
.cargo-target/debug/audio_reliability.exe --vst "C:/Program Files/VSTPlugins/Upright Piano Windows VST 64-bit/Upright Piano.dll" --device "Voicemeeter AUX Virtual ASIO" --buffer 512 --render-ahead-blocks 1 --notes 32 --events-per-second 20 --report artifacts/audio-callback-native-1.json
```

**Configuration :** Windows x64, Voicemeeter AUX Virtual ASIO **1.0.2.2**,
Upright Piano VST2 x64, SHA-256
`65095B00AEA68547ADADBF7EBD3CFA5746A8804347AD9E384BC7863F59E57BE4`.
Gain −24 dB, limiteur activé, EQ par défaut, Game Mode actif, politique MMCSS.
MIDI injecté par IPC ; 32 notes de 50 ms, 32 relâchements, transmission de
3,201 s, puis observation et reset d'urgence. Métadonnées conservées dans
`artifacts/audio-callback-native-config.json`.

| Observation | Direct | Rendu anticipé, 1 bloc |
| --- | ---: | ---: |
| Code de sortie du diagnostic | 0 | 0 |
| Fréquence demandée / rapportée | 48000 / 48000 Hz | 48000 / 48000 Hz |
| Buffer demandé / rapporté | 512 / 512 | 512 / 512 |
| Note-On / Note-Off soumis | 32 / 32 | 32 / 32 |
| Pertes MIDI / xruns / sous-alimentations / gaps | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |
| Reset observé / redémarrages du worker | 1 / 0 | 1 / 0 |
| Pic audio relevé | 0,0037204 | 0,0036254 |
| Livraison MIDI p99, histogramme | 12000 µs | 12000 µs |
| Livraison MIDI maximale relevée | 11990 µs | 11973 µs |
| Durée maximale relevée du callback pilote | 1040 µs | 23 µs |
| Durée maximale relevée du traitement VST | 1006 µs | 939 µs |

Ce sont des observations courtes de la configuration indiquée, sans comparaison
avant/après correction. Le callback pilote du rendu anticipé n'exécute pas le
DSP : ses durées ne représentent donc pas le coût audio total. La période
théorique de 10,667 ms est **calculée** par 512 / 48000 ; ce n'est pas une
mesure de latence analogique. La fréquence rapportée est celle de la
configuration CPAL acceptée ; aucune horloge physique n'a été mesurée.

Un premier essai de 128 notes en 160–161 ms passait déjà les contrôles de
livraison/timing dans les deux modes, mais n'avait relevé aucun pic dans le
mode direct. Ce résultat ne suffisait pas pour valider le signal ; les essais
avec notes plus longues ci-dessus ont capturé du signal dans les deux modes.
Les rapports initiaux sont conservés sous `audio-callback-native-*-short.json`.
La cause de cette absence de pic dans l'essai court n'a pas été établie et
n'a motivé aucune modification du moteur.

## Limites

Les tests déterministes utilisent des buffers synthétiques, sans pilote ni
plugin ; ils ne constituent pas une mesure de latence ou de performance.
Le compteur d'allocations existant couvre son scénario préparé et ne démontre
pas l'absence d'allocation ou d'attente dans un plugin tiers ou un pilote.
La perte physique d'un périphérique, les notifications de reset ASIO externes,
WASAPI avec matériel réel, les instruments VST3/x86 avec pilote réel et la
latence analogique nécessitent une validation dédiée. Les valeurs hors plage
I24 ont été reproduites de manière synthétique ; les essais natifs ne forcent
pas une saturation PCM. Aucun gain de performance n'est revendiqué.
