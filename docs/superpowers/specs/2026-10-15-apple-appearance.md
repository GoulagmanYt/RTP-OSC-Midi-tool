# OSCMidi — direction Apple

Cette demande remplace la direction visuelle « régie » du premier essai. L'utilisateur veut explicitement une esthétique Apple et l'application de ses principes de design. Le socle reste une app Tauri Windows ; l'objectif est une traduction soignée des HIG, pas une certification macOS ni une implémentation native de Liquid Glass.

## Intention et audit

Les bordures marquées, la palette acier, les angles courts et l'emploi étendu de monospace donnent encore une apparence technique au premier essai. Les connexions configurées et leurs liens sont utiles : les conserver. Remplacer leur présentation par des listes groupées et des panneaux calmes, privilégier l'espace et la typographie aux contours.

Références Apple lues dans `C:/Users/robin/.codex/skills/apple-design/references/hig/` : design-principles, accessibility, layout, typography, color, designing-for-macos, sidebars, toolbars, buttons, lists-and-tables, text-fields, dark-mode, materials et liquid-glass. La traduction technique suit cross-platform.

- `design-principles.md › Simplicity` : “Include just what’s necessary.” Retirer titre répété, sélecteur de thème de la toolbar et footer décoratif.
- `layout.md › Visual hierarchy` : “Group related items to clearly express related information or functions.” Listes groupées, espacement cohérent et séparations discrètes.
- `liquid-glass.md › Review checklist` : le verre reste sur les commandes et la navigation. Panneaux de contenu opaques, repli opaque pour transparence réduite et contraste augmenté.
- `dark-mode.md › Best practices` : “Ensure that your app looks good in both appearance modes.” Proposer le suivi des préférences système et conserver les préférences manuelles existantes par souci de compatibilité et de contrôle utilisateur.
- `accessibility.md › Motor` : navigation clavier, cibles confortables et labels explicites. Associer les contrôles principaux à leurs libellés.

## Système visuel

Six rôles clair / sombre : fond #F5F5F7 / #1D1D1F ; panneau #FFFFFF / #242426 ; texte #1D1D1F / #F5F5F7 ; secondaire #636369 / #AEAEB2 ; accent #0064D9 / #8DB9FF ; signal positif #217A3C / #82D998. Les couleurs d'alerte restent sémantiques. Vérifier les ratios sur les surfaces effectivement utilisées.

Polices système : `-apple-system`, `BlinkMacSystemFont`, puis Segoe UI Variable/Segoe UI ; aucune police Apple redistribuée. Titres 28 px / 600, sections 16 px / 600, corps 14 px, annotations 12 px. Chiffres tabulaires pour les mesures ; monospace réservé aux adresses et chemins. Espacements multiples de 4/8 px. Surfaces aux angles 16–24 px, contrôles 8–12 px, ombres discrètes. Ni halos ni animation décorative.

```
Sidebar douce | Sidebar on/off       Recherche   Langue   Bridge   Fenêtre
              | Titre + description
              | Connexions : sources → bridge → destinations
              | Audio                       Réseau
              | Événements
```

En largeur compacte ou avec le texte agrandi : navigation en icônes, toolbar compacte et listes empilées. La signature utile reste le trajet des messages. Le centre représente le bridge sans répéter le logo de l'app. Verre discret seulement sur la toolbar et la sidebar ; aucune simulation de refraction ou d'activité.

## Principes et limites

Purpose : connexions et santé visibles. Agency : navigation libre, sidebar masquable, palettes existantes préservées. Responsibility : configuration distincte du runtime, aucun changement de traitement audio/MIDI. Familiarity : interactions Radix et commandes Windows fonctionnelles. Flexibility : clavier, FR/EN, agrandissement, préférences système. Simplicity : toolbar et décoration allégées. Craft : contrastes et états vérifiés. Delight : équilibre et fluidité mesurée.

Valider couleurs, trois palettes manuelles et mode système, changements de préférence en direct, clavier, labels, repli opaque, largeur native minimale et texte 200 %. Refaire lint, tests et build. Les captures navigateur utilisent un backend simulé ; elles ne prouvent pas une conformité native Apple exhaustive.

## Vérifications du 15 octobre 2026

- Validation finale : `npm run lint` passe ; `npm test -- --exclude 'artifacts/**' --exclude 'vendor/**'` passe avec 124 tests dans 30 fichiers ; `npm run build` et `git diff --check` passent. Le build signale uniquement l'ancienneté des données Browserslist déjà présentes.

- Les 39 paires de couleurs mesurées sur les trois palettes dépassent 4,5:1. Le texte secondaire clair a été ajusté à #636369 : 5,97:1 sur panneau, 4,93:1 sur fond de sidebar opaque. Le bleu sur son fond d'accent atteint 4,75:1 en clair.
- Préférences système clair/sombre modifiées en direct dans Chromium ; abonnement et nettoyage couverts par tests unitaires. Un choix manuel reste stable quand la préférence système change.
- Bouton de sidebar, navigation au clavier entre les radios d'apparence, Ctrl+K, nom accessible de la palette et fermeture par Échap vérifiés. Les contrôles visibles des fixtures Routing, RTP, OSC, Audio et Settings ont un label ou un nom explicite ; ce n'est pas un audit exhaustif des états avancés.
- Dashboard sans débordement horizontal à 1240, 620 et 320 px et avec texte doublé à 1240 px. Repli des matériaux vérifié via émulation `prefers-reduced-transparency` : `backdrop-filter: none` sur sidebar et toolbar.
- Contraste élevé Windows : focus réel de 2 px restauré malgré les utilitaires qui suppriment l'outline ; curseurs des switches distincts de leur fond avec les couleurs système Highlight/HighlightText et Canvas/ButtonText.
- Revue indépendante : gestion du thème système, nettoyage des abonnements, navigation et labels validés ; défaut de focus en couleurs forcées reproduit puis corrigé et revérifié en navigateur.
- Captures dans `output/playwright/apple-light.png`, `apple-dark.png`, `apple-settings.png` et captures des états d'accessibilité. Essais avec données natives simulées, sans validation matérielle MIDI/audio ni certification Apple.
