//! Petit état partagé en mémoire entre les différents écrans (ville, combat,
//! ...) : pour l'instant, juste la case où replacer le joueur sur la carte
//! d'exploration quand il quitte un écran secondaire. Pas une vraie sauvegarde
//! (ça, c'est la Phase 2) : ça ne survit pas à un redémarrage du jeu.

use std::sync::Mutex;

use crate::world::grid::GridPos;

static RETURN_POINT: Mutex<GridPos> = Mutex::new(GridPos { x: 0, y: 0 });

pub fn set_return_point(pos: GridPos) {
    *RETURN_POINT.lock().unwrap() = pos;
}

pub fn return_point() -> GridPos {
    *RETURN_POINT.lock().unwrap()
}
