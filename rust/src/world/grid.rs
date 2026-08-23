//! Grille logique de la carte d'exploration.
//!
//! Ce module ne dépend pas de Godot : uniquement des structs et des fonctions
//! pures (état -> nouvel état). C'est ce qui permettra, plus tard, de faire
//! tourner exactement ce code côté serveur headless (multijoueur) sans rien
//! réécrire (voir specs-jeu-rpg-tactique.md, section 8).

pub use crate::geometry::{GridBounds, GridPos};

/// Taille de la carte d'exploration (specs Phase 1 étape 1). Seule source
/// de vérité, partagée entre `WorldScene` (client) et `CoopSession` (serveur
/// headless coop, Phase 3 étape 7) pour qu'ils appliquent exactement les
/// mêmes limites.
pub const WORLD_BOUNDS: GridBounds = GridBounds { width: 10, height: 8 };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn offset(self) -> (i32, i32) {
        match self {
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
        }
    }

    /// Code compact pour transmettre une direction sur le réseau (RPC
    /// `request_move`, voir `network.rs`) sans dépendre d'un type Godot.
    pub fn to_code(self) -> i32 {
        match self {
            Direction::Up => 0,
            Direction::Down => 1,
            Direction::Left => 2,
            Direction::Right => 3,
        }
    }

    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Direction::Up),
            1 => Some(Direction::Down),
            2 => Some(Direction::Left),
            3 => Some(Direction::Right),
            _ => None,
        }
    }
}

/// Case d'arrivée en se déplaçant d'une case dans une direction, ou `None` si
/// ça sortirait de la grille. Pas d'obstacles pour l'instant (Phase 1).
pub fn step(from: GridPos, dir: Direction, bounds: GridBounds) -> Option<GridPos> {
    let (dx, dy) = dir.offset();
    let target = GridPos::new(from.x + dx, from.y + dy);
    bounds.contains(target).then_some(target)
}

/// Chemin en L (horizontal puis vertical) entre deux cases, case par case.
/// Suffisant tant qu'il n'y a pas d'obstacles sur la carte ; à remplacer par
/// un vrai pathfinding (A*) quand des cases bloquantes apparaîtront (mines,
/// murs de ville, etc.).
pub fn path_to(from: GridPos, to: GridPos) -> Vec<GridPos> {
    let mut path = Vec::new();
    let mut current = from;

    while current.x != to.x {
        current.x += (to.x - current.x).signum();
        path.push(current);
    }
    while current.y != to.y {
        current.y += (to.y - current.y).signum();
        path.push(current);
    }

    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_moves_one_cell() {
        let bounds = GridBounds { width: 5, height: 5 };
        let pos = GridPos::new(2, 2);
        assert_eq!(step(pos, Direction::Right, bounds), Some(GridPos::new(3, 2)));
        assert_eq!(step(pos, Direction::Up, bounds), Some(GridPos::new(2, 1)));
    }

    #[test]
    fn step_blocked_by_bounds() {
        let bounds = GridBounds { width: 5, height: 5 };
        assert_eq!(step(GridPos::new(0, 0), Direction::Left, bounds), None);
        assert_eq!(step(GridPos::new(0, 0), Direction::Up, bounds), None);
        assert_eq!(step(GridPos::new(4, 4), Direction::Right, bounds), None);
        assert_eq!(step(GridPos::new(4, 4), Direction::Down, bounds), None);
    }

    #[test]
    fn path_to_reaches_target() {
        let path = path_to(GridPos::new(0, 0), GridPos::new(2, -1));
        assert_eq!(path.last(), Some(&GridPos::new(2, -1)));
        // Un pas par case parcourue, pas de diagonale.
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn path_to_same_cell_is_empty() {
        let path = path_to(GridPos::new(1, 1), GridPos::new(1, 1));
        assert!(path.is_empty());
    }

    #[test]
    fn direction_code_round_trips() {
        for dir in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
            assert_eq!(Direction::from_code(dir.to_code()), Some(dir));
        }
        assert_eq!(Direction::from_code(42), None);
    }
}
