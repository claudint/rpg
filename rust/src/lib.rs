use godot::prelude::*;
 
// Point d'entrée obligatoire : c'est ce que Godot cherche via "entry_symbol"
// dans le fichier .gdextension. Sans ça, Godot ne détecte aucune classe Rust.
struct MonJeuExtension;
 
#[gdextension]
unsafe impl ExtensionLibrary for MonJeuExtension {}
 
// Un noeud Rust minimal, juste pour vérifier que la chaîne Godot <-> Rust
// fonctionne. Il affiche un message dans la console de Godot au démarrage.
#[derive(GodotClass)]
#[class(base=Node)]
struct TestNode {
    base: Base<Node>,
}
 
#[godot_api]
impl INode for TestNode {
    fn init(base: Base<Node>) -> Self {
        Self { base }
    }
 
    fn ready(&mut self) {
        godot_print!("TestNode Rust connecté à Godot avec succès !");
    }
}
 