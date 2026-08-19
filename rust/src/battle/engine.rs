//! Etat de combat pur : aucune dépendance à Godot. Prend un état, une action,
//! retourne un nouvel état — même principe que `world::grid` (specs, section 8).

use std::collections::HashMap;

use crate::geometry::{GridBounds, GridPos};

use super::pattern::{self, Pattern};

/// Plateau 3x4 par camp (specs, section 3.3).
pub const BOARD_BOUNDS: GridBounds = GridBounds { width: 3, height: 4 };

/// Les deux plateaux mis bout à bout (le bord droit du plateau joueur touche
/// le bord gauche du plateau ennemi) : un sort de zone ancré près de cette
/// frontière peut donc toucher les deux camps, comme si les plateaux étaient
/// collés l'un à l'autre.
pub const COMBINED_BOUNDS: GridBounds = GridBounds { width: BOARD_BOUNDS.width * 2, height: BOARD_BOUNDS.height };

pub fn to_combined(pos: BoardPos) -> GridPos {
    let x = match pos.side {
        Side::Player => pos.cell.x,
        Side::Enemy => BOARD_BOUNDS.width + pos.cell.x,
    };
    GridPos::new(x, pos.cell.y)
}

pub fn from_combined(pos: GridPos) -> BoardPos {
    if pos.x < BOARD_BOUNDS.width {
        BoardPos { side: Side::Player, cell: pos }
    } else {
        BoardPos { side: Side::Enemy, cell: GridPos::new(pos.x - BOARD_BOUNDS.width, pos.y) }
    }
}

pub type UnitId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Player,
    Enemy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardPos {
    pub side: Side,
    pub cell: GridPos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum TargetKind {
    Ally,
    Enemy,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Spell {
    pub id: String,
    pub name: String,
    /// Delta de PV appliqué aux cibles touchées (négatif = dégâts, positif = soin).
    pub power: i32,
    pub target: TargetKind,
    pub pattern: Pattern,
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub name: String,
    pub side: Side,
    pub pos: BoardPos,
    pub hp: i32,
    pub max_hp: i32,
    pub speed: i32,
    pub spell_ids: Vec<String>,
}

impl Unit {
    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Victory,
    Defeat,
}

pub struct BattleState {
    pub units: Vec<Unit>,
    spells: HashMap<String, Spell>,
    turn_order: Vec<UnitId>,
    current_turn: usize,
}

impl BattleState {
    /// L'ordre des tours est fixé une fois pour toutes au début du combat,
    /// par vitesse décroissante (specs, section 3.3).
    pub fn new(units: Vec<Unit>, spells: Vec<Spell>) -> Self {
        let mut turn_order: Vec<UnitId> = (0..units.len()).collect();
        turn_order.sort_by(|&a, &b| units[b].speed.cmp(&units[a].speed));

        let spells = spells.into_iter().map(|s| (s.id.clone(), s)).collect();

        Self { units, spells, turn_order, current_turn: 0 }
    }

    pub fn current_unit_id(&self) -> UnitId {
        self.turn_order[self.current_turn]
    }

    pub fn unit(&self, id: UnitId) -> &Unit {
        &self.units[id]
    }

    pub fn spell(&self, id: &str) -> Option<&Spell> {
        self.spells.get(id)
    }

    /// Applique un sort ancré en `target_cell`. La géométrie seule décide qui
    /// est touché : les deux plateaux sont traités comme collés l'un à
    /// l'autre (`to_combined`), donc un sort de zone ancré près de la
    /// frontière peut toucher des unités des deux camps, y compris des
    /// alliés du lanceur (et le lanceur lui-même). `Spell::target` ne sert
    /// qu'à valider quel plateau on a le droit de viser (voir
    /// `battle::scene`), pas à filtrer les dégâts après coup.
    /// Retourne les unités touchées.
    pub fn resolve_action(&mut self, spell_id: &str, target_cell: BoardPos) -> Vec<UnitId> {
        let Some(spell) = self.spells.get(spell_id) else {
            return Vec::new();
        };
        let power = spell.power;
        let affected = pattern::cells(to_combined(target_cell), spell.pattern, COMBINED_BOUNDS);

        let mut hit_ids = Vec::new();
        for (id, unit) in self.units.iter_mut().enumerate() {
            if !unit.is_alive() || !affected.contains(&to_combined(unit.pos)) {
                continue;
            }

            unit.hp = (unit.hp + power).clamp(0, unit.max_hp);
            hit_ids.push(id);
        }

        hit_ids
    }

    /// Passe à la prochaine unité vivante dans l'ordre des tours.
    pub fn advance_turn(&mut self) {
        for _ in 0..self.turn_order.len() {
            self.current_turn = (self.current_turn + 1) % self.turn_order.len();
            if self.units[self.turn_order[self.current_turn]].is_alive() {
                return;
            }
        }
    }

    pub fn outcome(&self) -> Option<Outcome> {
        let player_alive = self.units.iter().any(|u| u.side == Side::Player && u.is_alive());
        let enemy_alive = self.units.iter().any(|u| u.side == Side::Enemy && u.is_alive());

        if !player_alive {
            Some(Outcome::Defeat)
        } else if !enemy_alive {
            Some(Outcome::Victory)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str, side: Side, x: i32, y: i32, hp: i32, speed: i32, spell_ids: &[&str]) -> Unit {
        Unit {
            name: name.to_string(),
            side,
            pos: BoardPos { side, cell: GridPos::new(x, y) },
            hp,
            max_hp: hp,
            speed,
            spell_ids: spell_ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn strike_spell() -> Spell {
        Spell {
            id: "strike".to_string(),
            name: "Coup".to_string(),
            power: -8,
            target: TargetKind::Enemy,
            pattern: Pattern::Single,
        }
    }

    #[test]
    fn turn_order_is_by_descending_speed() {
        let units = vec![
            unit("Lent", Side::Player, 0, 0, 20, 5, &["strike"]),
            unit("Rapide", Side::Enemy, 0, 0, 15, 12, &["strike"]),
        ];
        let state = BattleState::new(units, vec![strike_spell()]);
        assert_eq!(state.current_unit_id(), 1); // "Rapide" agit en premier
    }

    #[test]
    fn resolve_action_damages_only_matching_target() {
        let units = vec![
            unit("Heros", Side::Player, 0, 0, 20, 10, &["strike"]),
            unit("Gobelin", Side::Enemy, 1, 1, 15, 5, &["strike"]),
            unit("Allie", Side::Player, 1, 1, 20, 5, &["strike"]),
        ];
        let mut state = BattleState::new(units, vec![strike_spell()]);

        let target = BoardPos { side: Side::Enemy, cell: GridPos::new(1, 1) };
        let hit = state.resolve_action("strike", target);

        assert_eq!(hit, vec![1]);
        assert_eq!(state.unit(1).hp, 7);
        assert_eq!(state.unit(2).hp, 20); // l'allié sur la même case côté joueur n'est pas touché
    }

    #[test]
    fn advance_turn_skips_dead_units() {
        let mut units = vec![
            unit("A", Side::Player, 0, 0, 20, 10, &["strike"]),
            unit("B", Side::Enemy, 0, 0, 20, 8, &["strike"]),
            unit("C", Side::Enemy, 1, 0, 20, 6, &["strike"]),
        ];
        units[1].hp = 0; // "B" déjà mort
        let mut state = BattleState::new(units, vec![strike_spell()]);

        assert_eq!(state.current_unit_id(), 0); // "A", le plus rapide
        state.advance_turn();
        assert_eq!(state.current_unit_id(), 2); // "B" est sauté car mort
    }

    #[test]
    fn outcome_detects_defeat_and_victory() {
        let units = vec![
            unit("Heros", Side::Player, 0, 0, 8, 10, &["strike"]),
            unit("Gobelin", Side::Enemy, 0, 0, 8, 5, &["strike"]),
        ];
        let mut state = BattleState::new(units, vec![strike_spell()]);
        assert_eq!(state.outcome(), None);

        let target = BoardPos { side: Side::Enemy, cell: GridPos::new(0, 0) };
        state.resolve_action("strike", target);
        assert_eq!(state.outcome(), Some(Outcome::Victory));
    }

    #[test]
    fn zone_spell_splashes_across_the_board_boundary() {
        let cross_spell = Spell {
            id: "boule".to_string(),
            name: "Boule".to_string(),
            power: -5,
            target: TargetKind::Enemy,
            pattern: Pattern::Cross(1),
        };
        let units = vec![
            unit("Mage", Side::Player, 2, 1, 20, 10, &["boule"]), // colonne 2 = 1re ligne joueur
            unit("Gobelin", Side::Enemy, 0, 1, 15, 5, &["boule"]), // colonne 0 = 1re ligne ennemie, juste en face
        ];
        let mut state = BattleState::new(units, vec![cross_spell]);

        // Le mage vise le gobelin juste en face, à la frontière des deux plateaux.
        let target = BoardPos { side: Side::Enemy, cell: GridPos::new(0, 1) };
        let hit = state.resolve_action("boule", target);

        assert!(hit.contains(&0), "le mage se prend l'explosion en pleine face, les plateaux sont collés");
        assert!(hit.contains(&1));
        assert_eq!(state.unit(0).hp, 15);
        assert_eq!(state.unit(1).hp, 10);
    }

    #[test]
    fn zone_spell_does_not_splash_when_anchored_away_from_boundary() {
        let cross_spell = Spell {
            id: "boule".to_string(),
            name: "Boule".to_string(),
            power: -5,
            target: TargetKind::Enemy,
            pattern: Pattern::Cross(1),
        };
        let units = vec![
            unit("Mage", Side::Player, 0, 1, 20, 10, &["boule"]), // colonne 0 = dernière ligne joueur, loin de la frontière
            unit("Gobelin", Side::Enemy, 2, 1, 15, 5, &["boule"]), // colonne 2 = dernière ligne ennemie, loin aussi
        ];
        let mut state = BattleState::new(units, vec![cross_spell]);

        let target = BoardPos { side: Side::Enemy, cell: GridPos::new(2, 1) };
        let hit = state.resolve_action("boule", target);

        assert_eq!(hit, vec![1]);
        assert_eq!(state.unit(0).hp, 20); // trop loin de la frontière pour être touché
    }
}
