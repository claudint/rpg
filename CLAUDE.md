# CLAUDE.md

Ce projet est un RPG tactique multijoueur. Les specs complètes sont dans
specs-jeu-rpg-tactique.md — les lire avant toute modification importante.

## Stack
- Godot 4 + Rust (gdext) pour le jeu, dans godot-project/ et rust/
- Toujours suivre l'ordre des phases du MVP (section 6 des specs)
- La logique de jeu (combat, déplacement) doit être une crate Rust pure,
  sans dépendance à l'affichage

## Commandes utiles
- Build Rust : cd rust && cargo build
- Le projet Godot est dans godot-project/