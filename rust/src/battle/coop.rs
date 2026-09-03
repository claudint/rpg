//! Orchestration d'un combat coop (specs section 6, étape 8) : plusieurs
//! joueurs placent et jouent chacun leurs propres personnages dans le même
//! combat. Contrairement à `engine.rs`, ce module n'a pas besoin d'être
//! strictement pur (il peut dépendre de Godot comme `battle::scene` le fait
//! déjà pour son IA) — mais pour rester testable sans lancer Godot, aucune
//! des méthodes ci-dessous n'appelle `randf()` elle-même : le tirage
//! aléatoire pour l'IA est fourni par l'appelant (voir `resolve_ai_turn`),
//! exactement comme `engine::choose_ai_action` le fait déjà.
//!
//! Répartition des personnages (decision utilisateur, specs 3.4 "un ou
//! plusieurs personnages chacun") : le catalogue fixe de
//! `data::default_player_roster()` est distribué en round-robin entre les
//! joueurs connectés au moment où le combat démarre — 1 joueur récupère
//! tout (comportement identique au solo), 2 joueurs se partagent en 2+1,
//! etc. Pas d'UI de sélection d'équipe pour l'instant (specs section 9).

use std::collections::{HashMap, VecDeque};

use crate::geometry::GridPos;

use super::data;
use super::engine::{self, BattleState, BoardPos, Outcome, Rewards, Side, TargetKind, UnitId};

enum CoopPhase {
    Placement {
        /// Personnages restant à placer, par joueur (file : le prochain à
        /// placer est toujours en tête).
        pending: HashMap<i32, VecDeque<data::UnitDef>>,
        /// Personnages déjà placés, avec leur propriétaire, dans l'ordre où
        /// ils seront insérés dans `BattleState.units` (les ennemis sont
        /// ajoutés après, au moment de basculer en `Fight`).
        placed: Vec<(engine::Unit, i32)>,
    },
    Fight {
        battle: BattleState,
    },
    /// `rewards` est `Some` uniquement pour une victoire (`victory_rewards`
    /// est pur/déterministe, calculable ici) ; le butin (qui a besoin d'un
    /// tirage aléatoire, `randf()` de Godot) reste décidé côté
    /// `network::CoopSession`, seul endroit de ce combat coop qui touche
    /// Godot.
    End { outcome: Outcome, rewards: Option<Rewards> },
}

pub(crate) struct CoopBattle {
    phase: CoopPhase,
    /// Propriétaire (peer_id) de chaque unité côté joueur. Absent de la map
    /// = IA (ennemi, ou personnage dont le joueur s'est déconnecté).
    owners: HashMap<UnitId, i32>,
}

/// Un personnage à afficher côté client, indépendant de la représentation
/// interne du moteur (même choix que `network::RemotePlayer` découplé de
/// `GridPos` ailleurs dans le projet).
pub(crate) struct CoopUnitSnapshot {
    pub(crate) owner: Option<i32>,
    pub(crate) name: String,
    pub(crate) side: Side,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) hp: i32,
    pub(crate) max_hp: i32,
    pub(crate) spell_ids: Vec<String>,
}

pub(crate) enum CoopPhaseSnapshot {
    Placement { pending_counts: HashMap<i32, usize> },
    Fight {
        current_turn_owner: Option<i32>,
        /// Nom/sorts de l'unité dont c'est le tour, pour que le client
        /// puisse construire ses boutons de sort sans avoir à deviner
        /// laquelle de ses unités (s'il en possède plusieurs) est active.
        current_unit_name: String,
        current_unit_spell_ids: Vec<String>,
    },
    End { outcome: Outcome, rewards: Option<Rewards> },
}

pub(crate) struct CoopSnapshot {
    pub(crate) phase: CoopPhaseSnapshot,
    pub(crate) units: Vec<CoopUnitSnapshot>,
}

impl CoopBattle {
    /// Démarre un combat pour `connected_peers`, en répartissant le
    /// catalogue de personnages en round-robin entre eux.
    pub(crate) fn start(connected_peers: &[i32]) -> Self {
        let mut pending: HashMap<i32, VecDeque<data::UnitDef>> =
            connected_peers.iter().map(|&peer| (peer, VecDeque::new())).collect();

        if !connected_peers.is_empty() {
            for (i, def) in data::default_player_roster().into_iter().enumerate() {
                let peer = connected_peers[i % connected_peers.len()];
                pending.get_mut(&peer).expect("peer initialisé ci-dessus").push_back(def);
            }
        }

        Self { phase: CoopPhase::Placement { pending, placed: Vec::new() }, owners: HashMap::new() }
    }

    /// `peer_id` place son prochain personnage en attente sur `cell` (son
    /// propre plateau). Quand tout le monde a fini de placer, bascule
    /// automatiquement en combat (ajoute les ennemis, comme
    /// `BattleScene::start_fight` en solo).
    pub(crate) fn place(&mut self, peer_id: i32, cell: GridPos) -> Result<(), &'static str> {
        let CoopPhase::Placement { pending, placed } = &mut self.phase else {
            return Err("pas en phase de placement");
        };
        let queue = pending.get_mut(&peer_id).ok_or("joueur inconnu de ce combat")?;
        if queue.is_empty() {
            return Err("plus rien à placer pour ce joueur");
        }
        if placed.iter().any(|(unit, _)| unit.pos.cell == cell) {
            return Err("case déjà occupée");
        }

        let def = queue.pop_front().expect("file non vide, vérifié ci-dessus");
        let unit = def.into_unit(Side::Player, BoardPos { side: Side::Player, cell });
        placed.push((unit, peer_id));

        if pending.values().all(VecDeque::is_empty) {
            self.start_fight();
        }
        Ok(())
    }

    fn start_fight(&mut self) {
        let CoopPhase::Placement { placed, .. } = &self.phase else {
            return;
        };

        let mut units = Vec::new();
        let mut owners = HashMap::new();
        for (unit, peer) in placed.iter().cloned() {
            owners.insert(units.len(), peer);
            units.push(unit);
        }
        for (i, def) in data::default_enemy_roster().into_iter().enumerate() {
            let cell = GridPos::new(1, i as i32);
            units.push(def.into_unit(Side::Enemy, BoardPos { side: Side::Enemy, cell }));
        }

        self.owners = owners;
        self.phase = CoopPhase::Fight { battle: BattleState::new(units, data::spells()) };
    }

    /// `peer_id` agit pour le personnage dont c'est le tour. Refusé si ce
    /// n'est pas un de ses personnages (y compris si c'est le tour d'un
    /// ennemi), ou si `target` n'est pas sur le plateau attendu par le sort
    /// (même validation que `BattleScene::try_target` en solo, mais faite
    /// ici côté serveur — autoritaire, contrairement au solo où elle n'est
    /// que côté client).
    pub(crate) fn act(&mut self, peer_id: i32, spell_id: &str, target: BoardPos) -> Result<(), &'static str> {
        let CoopPhase::Fight { battle } = &mut self.phase else {
            return Err("pas en combat");
        };
        let current = battle.current_unit_id();
        if self.owners.get(&current) != Some(&peer_id) {
            return Err("ce n'est pas le tour d'un de tes personnages");
        }

        let caster = battle.unit(current);
        let Some(spell) = battle.spell(spell_id) else {
            return Err("sort inconnu");
        };
        let expected_side = match spell.target {
            TargetKind::Enemy => engine::opposite(caster.side),
            TargetKind::Ally => caster.side,
        };
        if target.side != expected_side {
            return Err("cible invalide pour ce sort");
        }

        battle.resolve_action(spell_id, target);
        battle.advance_turn();
        self.check_outcome();
        Ok(())
    }

    /// Vrai quand le combat est en cours et que le tour actuel n'appartient
    /// à aucun joueur connecté (ennemi, ou personnage dont le joueur s'est
    /// déconnecté) : l'appelant doit alors fournir un tirage aléatoire à
    /// `resolve_ai_turn` pour faire avancer le combat.
    pub(crate) fn needs_ai_turn(&self) -> bool {
        match &self.phase {
            CoopPhase::Fight { battle } => !self.owners.contains_key(&battle.current_unit_id()),
            _ => false,
        }
    }

    /// Joue le tour courant à l'IA (même décision que le combat solo, voir
    /// `engine::choose_ai_action`). N'a d'effet que si `needs_ai_turn()`.
    pub(crate) fn resolve_ai_turn(&mut self, roll: f64) {
        let CoopPhase::Fight { battle } = &mut self.phase else {
            return;
        };
        let current = battle.current_unit_id();
        if let Some((spell_id, target)) = engine::choose_ai_action(battle, current, roll) {
            battle.resolve_action(&spell_id, target);
        }
        battle.advance_turn();
        self.check_outcome();
    }

    fn check_outcome(&mut self) {
        if let CoopPhase::Fight { battle } = &self.phase {
            if let Some(outcome) = battle.outcome() {
                let rewards = (outcome == Outcome::Victory).then(|| battle.victory_rewards());
                self.phase = CoopPhase::End { outcome, rewards };
            }
        }
    }

    pub(crate) fn outcome(&self) -> Option<Outcome> {
        match &self.phase {
            CoopPhase::End { outcome, .. } => Some(*outcome),
            _ => None,
        }
    }

    /// `peer_id` s'est déconnecté : ses personnages passent à l'IA (retirés
    /// de `owners`) pour ne pas bloquer les autres joueurs. Ne gère que le
    /// cas d'un combat déjà en cours (`Fight`) — une déconnexion pendant le
    /// placement laisse simplement sa file d'attente non vidée, hors scope
    /// pour cette itération.
    pub(crate) fn on_disconnect(&mut self, peer_id: i32) {
        self.owners.retain(|_, owner| *owner != peer_id);
        if let CoopPhase::Placement { pending, .. } = &mut self.phase {
            pending.remove(&peer_id);
        }
    }

    /// État courant, pour diffusion réseau (voir `network::CoopSession`).
    pub(crate) fn snapshot(&self) -> CoopSnapshot {
        match &self.phase {
            CoopPhase::Placement { pending, placed } => CoopSnapshot {
                phase: CoopPhaseSnapshot::Placement {
                    pending_counts: pending.iter().map(|(&peer, queue)| (peer, queue.len())).collect(),
                },
                units: placed
                    .iter()
                    .map(|(unit, peer)| CoopUnitSnapshot {
                        owner: Some(*peer),
                        name: unit.name.clone(),
                        side: unit.side,
                        x: unit.pos.cell.x,
                        y: unit.pos.cell.y,
                        hp: unit.hp,
                        max_hp: unit.max_hp,
                        spell_ids: unit.spell_ids.clone(),
                    })
                    .collect(),
            },
            CoopPhase::Fight { battle } => {
                let current = battle.current_unit_id();
                let unit = battle.unit(current);
                CoopSnapshot {
                    phase: CoopPhaseSnapshot::Fight {
                        current_turn_owner: self.owners.get(&current).copied(),
                        current_unit_name: unit.name.clone(),
                        current_unit_spell_ids: unit.spell_ids.clone(),
                    },
                    units: self.battle_units_snapshot(battle),
                }
            }
            CoopPhase::End { outcome, rewards } => {
                CoopSnapshot { phase: CoopPhaseSnapshot::End { outcome: *outcome, rewards: *rewards }, units: Vec::new() }
            }
        }
    }

    fn battle_units_snapshot(&self, battle: &BattleState) -> Vec<CoopUnitSnapshot> {
        battle
            .units
            .iter()
            .enumerate()
            .map(|(id, unit)| CoopUnitSnapshot {
                owner: self.owners.get(&id).copied(),
                name: unit.name.clone(),
                side: unit.side,
                x: unit.pos.cell.x,
                y: unit.pos.cell.y,
                hp: unit.hp,
                max_hp: unit.max_hp,
                spell_ids: unit.spell_ids.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strike_spell_ids() -> &'static [&'static str] {
        &["coup_epee"]
    }

    #[test]
    fn round_robin_distributes_all_characters_with_one_peer() {
        let battle = CoopBattle::start(&[1]);
        let CoopPhase::Placement { pending, .. } = &battle.phase else { panic!("attendu Placement") };
        assert_eq!(pending.get(&1).unwrap().len(), data::default_player_roster().len());
    }

    #[test]
    fn round_robin_splits_between_two_peers() {
        let battle = CoopBattle::start(&[1, 2]);
        let CoopPhase::Placement { pending, .. } = &battle.phase else { panic!("attendu Placement") };
        let total = pending.get(&1).unwrap().len() + pending.get(&2).unwrap().len();
        assert_eq!(total, data::default_player_roster().len());
        // "un ou plusieurs personnages chacun" (specs 3.4) : personne à zéro.
        assert!(pending.get(&1).unwrap().len() >= 1);
        assert!(pending.get(&2).unwrap().len() >= 1);
    }

    #[test]
    fn placement_transitions_to_fight_once_everyone_is_done() {
        let mut battle = CoopBattle::start(&[1]);
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            battle.place(1, GridPos::new(0, i as i32)).unwrap();
        }
        assert!(matches!(battle.phase, CoopPhase::Fight { .. }));
    }

    #[test]
    fn a_player_cannot_act_for_someone_elses_unit() {
        let mut battle = CoopBattle::start(&[1, 2]);
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            // peer_id qui possède réellement le prochain personnage en attente
            let owner = if battle_pending_len(&battle, 1) > 0 { 1 } else { 2 };
            battle.place(owner, GridPos::new(0, i as i32)).unwrap();
        }
        assert!(matches!(battle.phase, CoopPhase::Fight { .. }));

        let CoopPhase::Fight { battle: state } = &battle.phase else { unreachable!() };
        let current = state.current_unit_id();
        let real_owner = *battle.owners.get(&current).unwrap();
        let impostor = if real_owner == 1 { 2 } else { 1 };

        let target = BoardPos { side: Side::Enemy, cell: GridPos::new(1, 0) };
        let result = battle.act(impostor, strike_spell_ids()[0], target);
        assert!(result.is_err());
    }

    fn battle_pending_len(battle: &CoopBattle, peer: i32) -> usize {
        match &battle.phase {
            CoopPhase::Placement { pending, .. } => pending.get(&peer).map(VecDeque::len).unwrap_or(0),
            _ => 0,
        }
    }

    #[test]
    fn disconnect_mid_fight_hands_unit_to_ai() {
        let mut battle = CoopBattle::start(&[1]);
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            battle.place(1, GridPos::new(0, i as i32)).unwrap();
        }
        assert!(!battle.owners.is_empty());

        battle.on_disconnect(1);
        assert!(battle.owners.is_empty());
        assert!(battle.needs_ai_turn() || battle.outcome().is_some());
    }
}
