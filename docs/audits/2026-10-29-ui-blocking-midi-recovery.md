# Commandes bloquantes et observation du rattrapage MIDI

## Vérification de l'audit

Le défaut des commandes est confirmé par le code et par la documentation
[Tauri v2](https://v2.tauri.app/develop/calling-rust/#async-commands) : une commande
synchrone s'exécute sur le thread principal ; une commande async est une tâche
du runtime. Appeler un service bloquant directement depuis cette tâche occupe
son thread pendant toute l'opération.

Le stress test attend dans une boucle ; le retest VST attend un probe isolé
(timeout de 60 s). Les commandes audio peuvent attendre le worker via
`safe_block_on`. Déclarer ces seules commandes async ne suffit donc pas.

Les 16 commandes désignées dans l'audit délèguent désormais leur corps au pool
`tauri::async_runtime::spawn_blocking`, via `blocking_command`. Ce helper conserve
les erreurs de service et convertit les erreurs de tâche en `CommandError` avec
un code propre à l'opération se terminant par `-task-failed`.
Les services application restent inchangés. Les verrous qu'ils prennent restent
dans le corps bloquant ; aucun second verrou de configuration n'est ajouté.
Les noms, arguments JavaScript et valeurs de succès restent identiques.
Les deux listes audio utilisent désormais un `Result` côté Rust pour permettre
de rejeter la promesse avec un `CommandError` en cas d'échec de la tâche ; le
frontend reçoit toujours les mêmes tableaux en cas de succès.

Le test `long_command_yields_to_the_calling_thread` utilise un runtime à un seul
thread. Une seconde future doit libérer une commande longue simulée. Avant
isolation, la commande empêche cette future de progresser et le test échoue ;
après isolation, elle est libérée et s'exécute sur un thread distinct. Un autre
test vérifie la conversion d'une panique de tâche en erreur structurée.
Ce test n'est pas une mesure de réactivité d'une fenêtre Windows réelle.

La revue du passage au pool bloquant a aussi identifié un risque d'inversion
des écritures de paramètres : le contrôleur frontend envoyait plusieurs valeurs
sans attendre les précédentes. La file conserve maintenant une seule requête
en cours et la dernière valeur en attente par paramètre. Elle continue après
un échec, abandonne les valeurs en attente lors d'un changement de plugin et
termine les écritures en attente lors de la navigation. Les signatures d'API
restent identiques. La file est partagée entre les instances du contrôleur
pour conserver l'ordre lors de la navigation aller-retour. Un test maintient
la première requête en attente pendant plusieurs mouvements de curseur pour
vérifier l'ordre et la valeur finale ; d'autres couvrent le remontage de la page
avec le même plugin ou un autre plugin.

## Télémétrie ajoutée

Les diagnostics exportés exposent, dans `status.realtime.worker` :

| Champ | Unité et sens |
| --- | --- |
| `midiRecoveryMaxLagUs` | µs ; écart maximal entre le début normal d'un bloc et le début de l'horloge MIDI en rattrapage |
| `midiRecoveryMaxDurationUs` | µs ; durée maximale d'un épisode, du début du premier bloc de rattrapage à la fin du dernier bloc admis, y compris un épisode encore actif |

La durée suit l'horloge source audio, y compris lorsque le rendu est anticipé.
Elle n'est ni un temps CPU ni une mesure de latence jusqu'au haut-parleur.
Les maxima persistent pendant la vie de la télémétrie du moteur ; un reset MIDI
termine l'épisode actif sans effacer les maxima. Un redémarrage du moteur crée
une nouvelle télémétrie. Les deux maxima sont indépendants et ne désignent pas
nécessairement le même épisode.

La publication suit l'admission des blocs VST2, VST3 et du bridge x86. Une
soumission VST3 rejetée ne fait avancer ni l'horloge ni ces compteurs. La mesure
utilise les timestamps existants et deux maxima atomiques, sans allocation,
verrou ou nouvelle lecture d'horloge dans le chemin audio. Les anciens workers
qui omettent les champs restent lisibles (valeurs par défaut à zéro).

## Mesures par simulation du comportement actuel

Test : `audio::midi_playback::tests::simulated_stalls_measure_recovery_lag_and_duration`.
Conditions : 48 kHz, blocs de 480 échantillons (10 ms), pas de nouvelle pause ni
de saturation de capacité pendant la récupération. Le retard est mesuré par
rapport au début normal du premier bloc ; l'horloge poursuit sa récupération
jusqu'au retour au temps réel.

| Retard initial | Rattrapage ×2 observé |
| --- | --- |
| 50 ms | 50 ms |
| 100 ms | 100 ms |
| 300 ms | 300 ms |

Le test existant `late_note_retains_duration_and_clock_catches_up` reste
inchangé : une note de 50 ms est rejouée en 1 200 échantillons à 48 kHz, soit
25 ms, et conserve une durée non nulle. Aucun comportement de lecture n'est
modifié par cette instrumentation. Ces résultats ne démontrent pas que VRChat
est la cause des retards observés sur une machine réelle.

Pour mesurer en situation : démarrer le moteur, jouer sous charge VRChat,
exporter les diagnostics après un pic, relever les deux champs avec
`callbackIntervalMaxUs`, `midiDeliveryMaxUs`, `renderUnderflows` et
`callbackGapCount`. Redémarrer le moteur entre scénarios pour comparer des
maxima ne contenant pas les épisodes précédents. Aucun relevé VRChat réel
n'a été effectué dans cette session.

## Proposition de réglage, sans implémentation ni activation

Ajouter ultérieurement à `AudioConfig` une politique `midi_catch_up` sérialisée
en `midiCatchUp`, avec deux variantes :

- `Accelerated { factor: 2 | 4 }`, défaut ×2 pour les configurations existantes.
- `ImmediateAbove { threshold_ms: u32 }`, accélération ×2 sous le seuil, puis
  admission des événements en retard dès le bloc courant au-delà du seuil.

Le facteur ×4 réduit théoriquement le temps de rattrapage à environ un tiers du
retard initial, à un bloc près, en comprimant davantage les durées de notes.
Le mode immédiat peut ramener une note entièrement en retard à une durée nulle
dans le bloc ; il doit rester volontaire et conserver l'ordre FIFO, les
Note-Off, la pédale et les limites d'admission. Ni ce mode ni ×4 ne garantissent
le rattrapage en un bloc quand le lot MIDI est plein.

Avant implémentation, comparer des relevés réels pour choisir le seuil N.
Transmettre ensuite la politique via `AudioSettings` au worker et au bridge x86,
en conservant les limites de lot et les retries après rejet. Les tests à ajouter
seraient : compatibilité de configuration absente (×2), durées des notes ×2/×4,
temps de récupération pour les trois retards ci-dessus, seuil exact et juste
au-dessus, ordre des accords/relâchements/pédale, saturation, reset et échec
d'admission. Ce patch conserve exclusivement la politique ×2 existante.

## Validation

- `rtk cargo test tauri::commands` : 9 tests réussis.
- `rtk cargo test --lib` : 373 réussis, 3 ignorés.
- `rtk cargo clippy -- -D warnings` : aucune erreur.
- `rtk cargo fmt --check` : conforme.
- `rtk npm test` : 300 tests réussis dans 52 fichiers.
- `rtk npm run lint` : ESLint et TypeScript conformes.
- `rtk npm run tauri:build -- --bundles msi` : release réussie, workers x64/x86,
  frontend, `OSCMidi.exe` et `OSCMidi_2.6.3_x64_en-US.msi` générés.
- Revue indépendante du diff : les deux risques d'ordre des paramètres relevés
  pendant la revue ont été corrigés ; aucune anomalie restante signalée.

Limites : aucune interaction avec une fenêtre Windows pendant un stress test
de 30 s et aucune mesure sous charge VRChat n'ont été effectuées. Les mesures
de récupération ci-dessus proviennent des tests déterministes, pas d'un relevé
audio matériel.
