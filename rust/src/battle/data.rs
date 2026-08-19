//! Chargement des données de combat (personnages, monstres, sorts) depuis du
//! JSON embarqué à la compilation. Specs, section 8 : modéliser avec des
//! données plutôt que du code en dur, pour ajouter du contenu sans toucher au
//! moteur (`battle::engine`). Le modèle de recrutement des personnages reste
//! à définir (specs, section 9) : équipe/escouade par défaut pour l'instant.

use serde::Deserialize;

use super::engine::{BoardPos, Side, Spell, Unit};

const CHARACTERS_JSON: &str = include_str!("../../data/characters.json");
const MONSTERS_JSON: &str = include_str!("../../data/monsters.json");
const SPELLS_JSON: &str = include_str!("../../data/spells.json");

#[derive(Debug, Clone, Deserialize)]
pub struct UnitDef {
    pub name: String,
    pub hp: i32,
    pub speed: i32,
    pub spell_ids: Vec<String>,
}

impl UnitDef {
    pub fn into_unit(self, side: Side, pos: BoardPos) -> Unit {
        Unit {
            name: self.name,
            side,
            pos,
            hp: self.hp,
            max_hp: self.hp,
            speed: self.speed,
            spell_ids: self.spell_ids,
        }
    }
}

pub fn default_player_roster() -> Vec<UnitDef> {
    serde_json::from_str(CHARACTERS_JSON).expect("rust/data/characters.json invalide")
}

pub fn default_enemy_roster() -> Vec<UnitDef> {
    serde_json::from_str(MONSTERS_JSON).expect("rust/data/monsters.json invalide")
}

pub fn spells() -> Vec<Spell> {
    serde_json::from_str(SPELLS_JSON).expect("rust/data/spells.json invalide")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_data_parses() {
        assert!(!default_player_roster().is_empty());
        assert!(!default_enemy_roster().is_empty());
        assert!(!spells().is_empty());
    }

    #[test]
    fn every_unit_spell_id_exists_in_spells() {
        let known: Vec<String> = spells().into_iter().map(|s| s.id).collect();
        for unit in default_player_roster().into_iter().chain(default_enemy_roster()) {
            for spell_id in &unit.spell_ids {
                assert!(known.contains(spell_id), "sort inconnu: {spell_id}");
            }
        }
    }
}
