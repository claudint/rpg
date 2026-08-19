//! Etat de combat pur : aucune dépendance à Godot. Prend un état, une action,
//! retourne un nouvel état — même principe que `world::grid` (specs, section 8).

use std::collections::HashMap;

use crate::geometry::{GridBounds, GridPos};

use super::pattern::{self, Pattern};

/// Plateau 3x4 par camp (specs, section 3.3).
pub const BOARD_BOUNDS: GridBounds = GridBounds { width: 3, height: 4 };

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

    /// Applique un sort lancé par `caster` sur la zone ancrée en `target_cell`.
    /// Retourne les unités touchées.
    pub fn resolve_action(&mut self, caster: UnitId, spell_id: &str, target_cell: BoardPos) -> Vec<UnitId> {
        let Some(spell) = self.spells.get(spell_id) else {
            return Vec::new();
        };
        let power = spell.power;
        let target_kind = spell.target;
        let affected_cells = pattern::cells(target_cell.cell, spell.pattern, BOARD_BOUNDS);
        let caster_side = self.units[caster].side;

        let mut hit_ids = Vec::new();
        for (id, unit) in self.units.iter_mut().enumerate() {
            if !unit.is_alive() || unit.pos.side != target_cell.side {
                continue;
            }
            if !affected_cells.contains(&unit.pos.cell) {
                continue;
            }
            let is_ally_of_caster = unit.side == caster_side;
            let matches_target = match target_kind {
                TargetKind::Ally => is_ally_of_caster,
                TargetKind::Enemy => !is_ally_of_caster,
            };
            if !matches_target {
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
        let hit = state.resolve_action(0, "strike", target);

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
        state.resolve_action(0, "strike", target);
        assert_eq!(state.outcome(), Some(Outcome::Victory));
    }
}
