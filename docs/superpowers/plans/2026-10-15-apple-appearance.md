# Apple Appearance Implementation Plan

**Goal:** Adapter l'UI à une esthétique et aux principes Apple, selon la demande explicite de l'utilisateur.

**Architecture:** React/Tauri/Radix existants. Couleurs par rôles dans themes.ts, comportement système abonné dans applyTheme avec nettoyage par useEffect. Matériaux CSS limités à la coque. Primitives partagées pour les panneaux, boutons et champs. Aucun changement au moteur natif.

**Spec:** [2026-10-15-apple-appearance.md](../specs/2026-10-15-apple-appearance.md)

## Contraintes et revue

Préserver FR/EN, données et cycles de vie indépendants. Compatibilité des palettes manuelles. Pas de dépendance ou de police distante. Revue : abonnement apparence libéré au changement ; sidebar encore accessible après masquage ; labels des formulaires ; absence de flou en transparence réduite ; lisibilité à 200 %.

- [x] Tester les mises à jour système et le nettoyage de applyTheme avant de modifier son comportement.
- [x] Remplacer les tokens acier par gris neutres/bleu, adapter surfaces, listes, typo et primitives.
- [x] Alléger la toolbar et la sidebar, permettre de masquer cette dernière avec un bouton accessible ; tester l'interaction.
- [x] Ajouter le mode système dans les préférences, relier les labels aux principaux contrôles, préserver les préférences manuelles.
- [x] Vérifier visuellement dashboard, formulaires et apparences ; mesurer les contrastes, le responsive, le focus et les replis d'accessibilité.
- [x] Revue indépendante, lint, tests frontend et build ; documenter les vérifications et leurs limites.
