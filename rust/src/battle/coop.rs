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
//!
//! Réutilisé tel quel pour le PvP (specs section 6, étape 9-10, "défi 1
//! contre 1") : un combat PvP est un combat coop où le côté `Side::Enemy`
//! est peuplé par un adversaire humain (`Opponent::Players`) au lieu de
//! l'IA (`Opponent::Ai`) — même répartition round-robin, même moteur de
//! tour/propriété/déconnexion, seul `start`/`start_fight`/`place` avaient
//! besoin de connaître le côté de chaque joueur (voir `peer_side`).

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

/// Adversaire du côté `Side::Enemy` d'un combat coop : l'IA (PvE, existant)
/// ou un ou plusieurs joueurs humains (PvP, specs section 6, étape 9-10).
pub(crate) enum Opponent {
    Ai,
    Players(Vec<i32>),
}

pub(crate) struct CoopBattle {
    phase: CoopPhase,
    /// Propriétaire (peer_id) de chaque unité, une fois le combat commencé
    /// (`CoopPhase::Fight`/`End`). Absent de la map = IA (ennemi, ou
    /// personnage dont le joueur s'est déconnecté).
    owners: HashMap<UnitId, i32>,
    /// Côté (`Side::Player` ou `Side::Enemy`) sur lequel chaque joueur
    /// place ses personnages, valable pour toute la durée du combat. En
    /// PvE, toujours `Side::Player` pour tout le monde. En PvP, le
    /// challenger est sur `Side::Player`, le défenseur sur `Side::Enemy`.
    peer_side: HashMap<i32, Side>,
    /// Vrai en PvE (le côté `Side::Enemy` est rempli par
    /// `data::default_enemy_roster()` une fois le placement terminé), faux
    /// en PvP (le côté `Side::Enemy` vient déjà de `placed`, comme
    /// `Side::Player`).
    ai_enemies: bool,
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
    Placement {
        pending_counts: HashMap<i32, usize>,
        /// Plateau attribué à chaque joueur (voir `CoopBattle::peer_side`).
        /// Toujours `Side::Player` pour tout le monde en PvE ; distingue
        /// challenger/défenseur en PvP.
        sides: HashMap<i32, Side>,
    },
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
    /// Démarre un combat pour `player_peers` (côté `Side::Player`), en
    /// répartissant le catalogue de personnages en round-robin entre eux ;
    /// `opponent` détermine qui peuple `Side::Enemy` — l'IA (PvE) ou une
    /// seconde répartition round-robin d'un jeu indépendant du même
    /// catalogue entre des joueurs adverses (PvP).
    pub(crate) fn start(player_peers: &[i32], opponent: Opponent) -> Self {
        let mut pending: HashMap<i32, VecDeque<data::UnitDef>> = HashMap::new();
        let mut peer_side: HashMap<i32, Side> = HashMap::new();
        round_robin_into(&mut pending, &mut peer_side, player_peers, Side::Player);

        let ai_enemies = match &opponent {
            Opponent::Ai => true,
            Opponent::Players(enemy_peers) => {
                round_robin_into(&mut pending, &mut peer_side, enemy_peers, Side::Enemy);
                false
            }
        };

        Self {
            phase: CoopPhase::Placement { pending, placed: Vec::new() },
            owners: HashMap::new(),
            peer_side,
            ai_enemies,
        }
    }

    /// `peer_id` place son prochain personnage en attente sur `cell`, sur
    /// le plateau qui lui est attribué (`peer_side`). Quand tout le monde a
    /// fini de placer, bascule automatiquement en combat — ajoute les
    /// ennemis IA en PvE (comme `BattleScene::start_fight` en solo), ou
    /// rien de plus en PvP (les deux camps viennent déjà de `placed`).
    pub(crate) fn place(&mut self, peer_id: i32, cell: GridPos) -> Result<(), &'static str> {
        let side = *self.peer_side.get(&peer_id).ok_or("joueur inconnu de ce combat")?;
        let CoopPhase::Placement { pending, placed } = &mut self.phase else {
            return Err("pas en phase de placement");
        };
        let queue = pending.get_mut(&peer_id).ok_or("joueur inconnu de ce combat")?;
        if queue.is_empty() {
            return Err("plus rien à placer pour ce joueur");
        }
        if placed.iter().any(|(unit, _)| unit.pos.cell == cell && unit.pos.side == side) {
            return Err("case déjà occupée");
        }

        let def = queue.pop_front().expect("file non vide, vérifié ci-dessus");
        let unit = def.into_unit(side, BoardPos { side, cell });
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
        if self.ai_enemies {
            for (i, def) in data::default_enemy_roster().into_iter().enumerate() {
                let cell = GridPos::new(1, i as i32);
                units.push(def.into_unit(Side::Enemy, BoardPos { side: Side::Enemy, cell }));
            }
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
                // Pas d'économie en PvP (specs section 6, étape 9-10) :
                // `victory_rewards()` calcule XP/argent à partir du camp
                // adverse défait, ce qui n'a de sens que face à l'IA.
                let rewards = (outcome == Outcome::Victory && self.ai_enemies).then(|| battle.victory_rewards());
                self.phase = CoopPhase::End { outcome, rewards };
            }
        }
    }

    /// Vrai si le côté `Side::Enemy` de ce combat est un adversaire humain
    /// (`Opponent::Players`) plutôt que l'IA — utile côté serveur pour
    /// savoir si un tirage de butin a un sens (voir
    /// `network::CoopSession::drive_and_broadcast_battle`).
    pub(crate) fn is_pvp(&self) -> bool {
        !self.ai_enemies
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
                    sides: self.peer_side.clone(),
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

/// Distribue un jeu indépendant de `data::default_player_roster()` en
/// round-robin entre `peers`, sur `side` — factorisé pour être appelé une
/// fois par camp (`Side::Player` toujours, `Side::Enemy` en plus en PvP).
/// No-op si `peers` est vide (PvE sans joueur connecté : ne devrait pas
/// arriver, mais `CoopBattle::start` reste sûr dans ce cas).
fn round_robin_into(
    pending: &mut HashMap<i32, VecDeque<data::UnitDef>>,
    peer_side: &mut HashMap<i32, Side>,
    peers: &[i32],
    side: Side,
) {
    for &peer in peers {
        pending.entry(peer).or_default();
        peer_side.insert(peer, side);
    }
    if peers.is_empty() {
        return;
    }
    for (i, def) in data::default_player_roster().into_iter().enumerate() {
        let peer = peers[i % peers.len()];
        pending.get_mut(&peer).expect("peer initialisé ci-dessus").push_back(def);
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
        let battle = CoopBattle::start(&[1], Opponent::Ai);
        let CoopPhase::Placement { pending, .. } = &battle.phase else { panic!("attendu Placement") };
        assert_eq!(pending.get(&1).unwrap().len(), data::default_player_roster().len());
    }

    #[test]
    fn round_robin_splits_between_two_peers() {
        let battle = CoopBattle::start(&[1, 2], Opponent::Ai);
        let CoopPhase::Placement { pending, .. } = &battle.phase else { panic!("attendu Placement") };
        let total = pending.get(&1).unwrap().len() + pending.get(&2).unwrap().len();
        assert_eq!(total, data::default_player_roster().len());
        // "un ou plusieurs personnages chacun" (specs 3.4) : personne à zéro.
        assert!(pending.get(&1).unwrap().len() >= 1);
        assert!(pending.get(&2).unwrap().len() >= 1);
    }

    #[test]
    fn placement_transitions_to_fight_once_everyone_is_done() {
        let mut battle = CoopBattle::start(&[1], Opponent::Ai);
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            battle.place(1, GridPos::new(0, i as i32)).unwrap();
        }
        assert!(matches!(battle.phase, CoopPhase::Fight { .. }));
    }

    #[test]
    fn a_player_cannot_act_for_someone_elses_unit() {
        let mut battle = CoopBattle::start(&[1, 2], Opponent::Ai);
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
        let mut battle = CoopBattle::start(&[1], Opponent::Ai);
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            battle.place(1, GridPos::new(0, i as i32)).unwrap();
        }
        assert!(!battle.owners.is_empty());

        battle.on_disconnect(1);
        assert!(battle.owners.is_empty());
        assert!(battle.needs_ai_turn() || battle.outcome().is_some());
    }

    // --- PvP (specs section 6, étape 9-10) : Opponent::Players ---

    #[test]
    fn pvp_distributes_a_full_independent_roster_to_each_side() {
        let battle = CoopBattle::start(&[1], Opponent::Players(vec![2]));
        let CoopPhase::Placement { pending, .. } = &battle.phase else { panic!("attendu Placement") };
        let roster_len = data::default_player_roster().len();
        assert_eq!(pending.get(&1).unwrap().len(), roster_len);
        assert_eq!(pending.get(&2).unwrap().len(), roster_len);
        assert_eq!(battle.peer_side.get(&1), Some(&Side::Player));
        assert_eq!(battle.peer_side.get(&2), Some(&Side::Enemy));
    }

    #[test]
    fn pvp_placement_on_the_same_cell_on_both_boards_does_not_collide() {
        let mut battle = CoopBattle::start(&[1], Opponent::Players(vec![2]));
        // Même case (0,0), mais sur deux plateaux différents : ne doit pas
        // se refuser mutuellement (contrairement à un vrai conflit sur le
        // même plateau, déjà couvert par le comportement existant).
        battle.place(1, GridPos::new(0, 0)).unwrap();
        battle.place(2, GridPos::new(0, 0)).unwrap();
    }

    #[test]
    fn pvp_transitions_to_fight_without_ai_roster() {
        let mut battle = CoopBattle::start(&[1], Opponent::Players(vec![2]));
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            battle.place(1, GridPos::new(0, i as i32)).unwrap();
            battle.place(2, GridPos::new(0, i as i32)).unwrap();
        }
        let CoopPhase::Fight { battle: state } = &battle.phase else { panic!("attendu Fight") };
        // Les deux camps viennent uniquement des joueurs (pas d'IA en plus).
        assert_eq!(state.units.len(), roster_len * 2);
        assert!(battle.owners.values().all(|&owner| owner == 1 || owner == 2));
    }

    #[test]
    fn pvp_defender_cannot_act_for_challengers_unit() {
        let mut battle = CoopBattle::start(&[1], Opponent::Players(vec![2]));
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            battle.place(1, GridPos::new(0, i as i32)).unwrap();
            battle.place(2, GridPos::new(0, i as i32)).unwrap();
        }
        let CoopPhase::Fight { battle: state } = &battle.phase else { unreachable!() };
        let current = state.current_unit_id();
        let real_owner = *battle.owners.get(&current).unwrap();
        let impostor = if real_owner == 1 { 2 } else { 1 };

        let target = BoardPos { side: engine::opposite(if real_owner == 1 { Side::Player } else { Side::Enemy }), cell: GridPos::new(0, 0) };
        let result = battle.act(impostor, strike_spell_ids()[0], target);
        assert!(result.is_err());
    }

    #[test]
    fn pvp_disconnect_hands_defenders_units_to_ai() {
        let mut battle = CoopBattle::start(&[1], Opponent::Players(vec![2]));
        let roster_len = data::default_player_roster().len();
        for i in 0..roster_len {
            battle.place(1, GridPos::new(0, i as i32)).unwrap();
            battle.place(2, GridPos::new(0, i as i32)).unwrap();
        }
        assert!(battle.owners.values().any(|&owner| owner == 2));

        battle.on_disconnect(2);
        assert!(battle.owners.values().all(|&owner| owner != 2));
    }
}
