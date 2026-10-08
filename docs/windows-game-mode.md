# Routage et audio sous charge Windows

Le mode jeu est activé par défaut, y compris lorsqu'une ancienne configuration
ne contient pas `audio.gameMode`. Les valeurs audio explicitement enregistrées
sont conservées. Les essais ci-dessous utilisent 48 kHz, 512 échantillons et
Voicemeeter AUX Virtual ASIO, avec les workers isolés x64/x86.

## Changements

1. Diagnostics : horodatages QPC de réception, écriture du pipe, réception worker
   et consommation DSP ; maximum d'attente par file ; pertes par cause ; état
   MMCSS ; progression et intervalle des callbacks. L'échéance OSC/RTP reste
   distincte de l'horodatage de réception. Le p99 utilise des classes de 250 µs.
2. Surveillance : heartbeats indépendants des commandes et du processeur,
   toutes les 500 ms ; avertissement à 1,5 s, récupération à 5 s. Progression
   audio surveillée séparément, maintenance annoncée avec délai de 10 s.
   Le budget de trois redémarrages par minute et la détection de crash restent
   actifs. Une rupture du pipe MIDI constitue une panne de transport distincte.
3. Livraison : runtimes Tokio mono-thread dédiés à RTP/OSC et aux pipes MIDI,
   MMCSS Pro Audio/HIGH puis Audio et priorité HIGHEST en repli. Lots binaires
   de 256 au plus, lectures complètes, annulation à l'arrêt. Les capacités de
   production restent inchangées ; une réserve d'admission protège les
   libérations critiques dans la même FIFO. Les générations invalident les
   anciens événements avant le reset, sans supprimer les nouveaux. Les sorties
   auxiliaires disposent de leur récupération ; le panic explicite reste global.
4. Protection VST : paramètres et diagnostics servis par caches/atomiques,
   commandes de paramètres appliquées dans le backend audio, sauvegarde native
   et éditeur différés en mode jeu. La sauvegarde est fusionnée ; seule la dernière
   demande d'éditeur est conservée. Les paramètres connus du host sont persistés
   dans un fichier `.live.json` avec le debounce existant. La sortie du mode jeu
   annonce une maintenance, suspend le stream, capture l'état sur le thread
   requis puis reprend. Une erreur conserve le dernier état natif valide et les
   demandes différées. Le bridge x86 réamorce son pipeline après cette capture.

Le protocole de contrôle est **9** ; les trames MIDI font **28 octets**. Le
protocole mémoire du bridge x86 est **6**. Rebuild et distribution des deux
workers avec l'application sont nécessaires ; les anciens workers sont refusés.

Le dépôt ne propose pas de commande indépendante de chargement de preset VST.
Les changements par l'éditeur passent donc par son ouverture différée ; le
routage MIDI Program Change existant est conservé. Les changements internes
non publiés par le plug-in nécessitent une future capture native pour être
récupérables. Un arrêt/rechargement explicite peut interrompre le rendu.

## Vérification automatisée

Depuis la racine, avec les outils de compilation Windows du dépôt :

```powershell
./scripts/prepare-vst-worker.ps1 -Profile debug
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features
npm run lint
npm exec --yes --package=node@24 -- node ./node_modules/vitest/vitest.mjs run
npm run build
cargo build --manifest-path tools/diagnostics/Cargo.toml --bin audio_reliability
```

Les fixtures couvrent commande lente avec heartbeat actif, silence de deux
secondes sans redémarrage, silence prolongé, callback arrêté, crash, budget de
récupération et refus du protocole 8. Les tests de files couvrent réserve critique,
FIFO, resets, lectures partielles, échéances futures et datagrammes invalides.
Le test matériel x86 est séparé et ignoré dans la suite standard :

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib installed_church_organ_renders_through_x86_bridge -- --ignored --nocapture
```

Pour préserver les états habituels pendant les essais de maintenance, créer un
répertoire d'états séparé et définir `OSCMIDI_VST_STATE_DIR`. Copier le preset
natif voulu dans ce répertoire avant un comparatif ; garder exactement le même
état pour les deux versions. Le test `--exercise-game-mode true` vérifie paramètres,
sauvegarde/éditeur différés, continuité des callbacks et exécution à la sortie.

## Résultats courts, 2026-10-08

Les JSON dans `docs/measurements` et leur provenance identifient les exécutables
et plug-ins. Ce sont des essais de **10 secondes d'injection MIDI dans le pipe
du worker**, hors VRChat, avec le matériel ASIO installé. Ils ne constituent pas
une validation de la chaîne réseau complète ou de la continuité audible.

Splice à 200 événements/s : aucune perte applicative, aucun reset avant le panic
final explicite, aucune contention du processeur imposée par le host, aucun
redémarrage ; p99 de livraison 12,25 ms, maximum 12,981 ms. Le délai mesuré ici
commence à l'injection applicative dans le pipe. Les essais VST2/VST3 Upright
Piano et x86 Church Organ incluent les opérations différées.

À environ 5 000 événements/s avec Splice, aucune perte ni redémarrage n'est
observé, mais le DSP p99 atteint 41,5 ms et 338 callbacks dépassent leur budget.
Le p99 de livraison atteint 36,25 ms : ce résultat échoue aux critères de timing
du test musical. Il faut corréler ces dépassements avec une trace et une capture
audio avant de valider la récupération sous surcharge. Le panic final contrôlé
ne démontre pas, à lui seul, l'absence de notes bloquées dans toutes les situations.

Le premier essai x86 a exposé deux underruns après maintenance ; le pipeline
attendait un bloc invalidé par la sauvegarde. Le réamorçage corrige ce défaut et
l'essai répété passe sans xrun. Le test direct `vst_smoke` d'Upright Piano VST3
a rendu du signal mais a quitté avec le code 1 lors de sa destruction ; la
validation retenue utilise le worker de production isolé.

## Comparatif matériel restant

Conserver des binaires **avant** et **après** séparés, leurs SHA-256, versions de
Splice/Voicemeeter/Windows, configuration, preset, fréquence CPU et paramètres
de VRChat. Les comparaisons de timing doivent employer des builds du même
profil, de préférence `release`. Les essais courts ci-dessus sont en `debug`.

Pour chaque version, à 200 événements/s : 10 minutes sans charge, 30 minutes dans
la même scène VRChat, puis rafales CPU identiques. Exécuter séparément le flux à
5 000 événements/s. `audio-soak.ps1` conserve les métriques, hashes et ressources
des processus ; indiquer explicitement le worker à tester :

```powershell
./tools/diagnostics/audio-soak.ps1 -DurationSeconds 600 -EventsPerSecond 200 -WorkerBinary '<worker-apres.exe>' -StateDirectory '<copie-des-etats>' -ReportName 'apres-repos-200'
./tools/diagnostics/audio-soak.ps1 -DurationSeconds 1800 -EventsPerSecond 200 -WorkerBinary '<worker-apres.exe>' -StateDirectory '<copie-des-etats>' -ReportName 'apres-vrchat-200'
```

Pendant ces essais, enregistrer simultanément la sortie Voicemeeter avec un
enregistreur externe et une trace WPR contenant CPU sampling, context switches,
ReadyThread, DPC/ISR et fautes mémoire. Vérifier les profils disponibles dans
WPR et prévoir l'espace disque ; WPA/WPR du Windows Performance Toolkit n'ont
pas été trouvés dans le PATH de cette session. La capture n'est pas simulée par
les compteurs applicatifs. Pour la chaîne complète, injecter en RTP/OSC dans
l'application et exporter également `RuntimeMetrics.realtime`, qui contient les
files réseau/routage et les compteurs par destination.

À 48 kHz/512, une période vaut 10,667 ms. Acceptation musicale : zéro perte,
reset inattendu, redémarrage et contention host ; p99 réception applicative →
consommation ≤21,333 ms, maximum ≤53,333 ms, aucune interruption dans la capture
audio. En surcharge : récupération bornée, aucune note bloquée et pertes
comptées. Toute cible manquée exige l'explication dans WPA avant validation.
MMCSS ne garantit pas une échéance face à un pilote ou un plug-in bloqué ; aucun
agrandissement automatique du buffer en réponse à une surcharge n'est ajouté.
