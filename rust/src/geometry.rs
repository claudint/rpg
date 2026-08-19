//! Primitives géométriques pures (grille bornée), partagées entre la carte
//! d'exploration (`world`) et le plateau de combat (`battle`). Aucune
//! dépendance à Godot.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
}

impl GridPos {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GridBounds {
    pub width: i32,
    pub height: i32,
}

impl GridBounds {
    pub fn contains(self, pos: GridPos) -> bool {
        pos.x >= 0 && pos.x < self.width && pos.y >= 0 && pos.y < self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_contain_only_inside_cells() {
        let bounds = GridBounds { width: 3, height: 4 };
        assert!(bounds.contains(GridPos::new(0, 0)));
        assert!(bounds.contains(GridPos::new(2, 3)));
        assert!(!bounds.contains(GridPos::new(3, 0)));
        assert!(!bounds.contains(GridPos::new(0, 4)));
        assert!(!bounds.contains(GridPos::new(-1, 0)));
    }
}
