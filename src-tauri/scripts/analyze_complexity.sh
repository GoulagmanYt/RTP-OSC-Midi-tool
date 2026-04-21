#!/bin/bash

# Script d'analyse de complexité cyclomatique pour le projet OSC-MIDI
# Utilise cargo-count pour mesurer les métriques de maintenabilité

echo "=== Analyse de Complexité - OSC-MIDI Bridge ==="
echo "Date: $(date)"
echo ""

# Installation de cargo-count si nécessaire
if ! command -v cargo-count &> /dev/null; then
    echo "Installation de cargo-count..."
    cargo install cargo-count
fi

echo "=== Métriques Globales ==="
cargo count --everything --safe

echo ""
echo "=== Top 10 des Fonctions par Complexité ==="
cargo count --everything --safe --sort-by=cc --top=10

echo ""
echo "=== Fichiers par Nombre de Lignes ==="
cargo count --everything --safe --files --sort-by=lines --top=15

echo ""
echo "=== Complexité par Module ==="
cargo count --everything --safe --modules

echo ""
echo "=== Fonctions avec > 7 arguments ==="
echo "Analyse manuelle des fonctions avec trop d'arguments..."

# Rechercher les fonctions avec beaucoup de paramètres
echo "Recherche des fonctions avec > 7 paramètres..."
find src/ -name "*.rs" -exec grep -H "fn.*(" {} \; | \
    grep -E "fn.*\([^)]{15,}" | \
    head -10

echo ""
echo "=== Types de Données Complexes ==="
echo "Recherche des structs avec > 10 champs..."
find src/ -name "*.rs" -exec grep -H "struct.*{" {} \; | \
    grep -E "struct.*\{[^}]{200,}" | \
    head -5

echo ""
echo "=== Fin de l'Analyse ==="
