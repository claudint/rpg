//! Motifs de zone génériques pour les sorts, ancrés sur une case cible.
//! Réutilisable pour n'importe quel sort (specs, section 8).

use crate::geometry::{GridBounds, GridPos};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum Pattern {
    /// Juste la case ciblée.
    Single,
    /// Croix (plus) de rayon `radius` autour de la case ciblée.
    Cross(i32),
    /// Carré de rayon `radius` autour de la case ciblée
    /// (rayon 1 => carré 3x3).
    Square(i32),
}

/// Cases touchées par un pattern ancré sur `origin`, réduites à celles qui
/// restent dans `bounds` (un plateau ne déborde pas sur l'autre camp).
pub fn cells(origin: GridPos, pattern: Pattern, bounds: GridBounds) -> Vec<GridPos> {
    let candidates = match pattern {
        Pattern::Single => vec![origin],
        Pattern::Cross(radius) => {
            let mut out = vec![origin];
            for d in 1..=radius {
                out.push(GridPos::new(origin.x + d, origin.y));
                out.push(GridPos::new(origin.x - d, origin.y));
                out.push(GridPos::new(origin.x, origin.y + d));
                out.push(GridPos::new(origin.x, origin.y - d));
            }
            out
        }
        Pattern::Square(radius) => {
            let mut out = Vec::new();
            for dx in -radius..=radius {
                for dy in -radius..=radius {
                    out.push(GridPos::new(origin.x + dx, origin.y + dy));
                }
            }
            out
        }
    };

    candidates.into_iter().filter(|pos| bounds.contains(*pos)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: GridBounds = GridBounds { width: 3, height: 4 };

    #[test]
    fn single_hits_only_origin() {
        let hits = cells(GridPos::new(1, 1), Pattern::Single, BOARD);
        assert_eq!(hits, vec![GridPos::new(1, 1)]);
    }

    #[test]
    fn cross_hits_four_neighbours_plus_origin() {
        let hits = cells(GridPos::new(1, 1), Pattern::Cross(1), BOARD);
        assert_eq!(hits.len(), 5);
        assert!(hits.contains(&GridPos::new(1, 0)));
        assert!(hits.contains(&GridPos::new(1, 2)));
        assert!(hits.contains(&GridPos::new(0, 1)));
        assert!(hits.contains(&GridPos::new(2, 1)));
    }

    #[test]
    fn square_hits_are_clamped_to_bounds() {
        // Coin (0, 0) : un carré de rayon 1 déborderait à x=-1 et y=-1.
        let hits = cells(GridPos::new(0, 0), Pattern::Square(1), BOARD);
        assert_eq!(hits.len(), 4);
    }
}
