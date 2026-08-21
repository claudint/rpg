use godot::prelude::*;

mod battle;
mod dev_console;
mod geometry;
mod persistence;
mod session;
mod world;

// Point d'entrée obligatoire : c'est ce que Godot cherche via "entry_symbol"
// dans le fichier .gdextension. Sans ça, Godot ne détecte aucune classe Rust.
struct MonJeuExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MonJeuExtension {}
