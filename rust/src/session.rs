//! Petit état partagé en mémoire entre les différents écrans (ville, combat,
//! ...) : la case où replacer le joueur sur la carte d'exploration en
//! quittant un écran secondaire, et la progression cumulée du joueur (XP,
//! or). Pas une vraie sauvegarde (ça, c'est la Phase 2) : ça ne survit pas à
//! un redémarrage du jeu.

use std::sync::Mutex;

use crate::world::grid::GridPos;

static RETURN_POINT: Mutex<GridPos> = Mutex::new(GridPos { x: 0, y: 0 });

pub fn set_return_point(pos: GridPos) {
    *RETURN_POINT.lock().unwrap() = pos;
}

pub fn return_point() -> GridPos {
    *RETURN_POINT.lock().unwrap()
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerProgress {
    pub xp: i32,
    pub gold: i32,
}

static PROGRESS: Mutex<PlayerProgress> = Mutex::new(PlayerProgress { xp: 0, gold: 0 });

pub fn add_rewards(xp: i32, gold: i32) {
    let mut progress = PROGRESS.lock().unwrap();
    progress.xp += xp;
    progress.gold += gold;
}

pub fn progress() -> PlayerProgress {
    *PROGRESS.lock().unwrap()
}

/// Récompense d'un combat gagné, à afficher une fois de retour sur la carte
/// (specs, section 6 étape 5).
#[derive(Debug, Clone, Copy)]
pub struct PendingReward {
    pub xp: i32,
    pub gold: i32,
    pub loot: &'static str,
}

static PENDING_REWARD: Mutex<Option<PendingReward>> = Mutex::new(None);

pub fn queue_reward(reward: PendingReward) {
    *PENDING_REWARD.lock().unwrap() = Some(reward);
}

/// Récupère la récompense en attente et l'efface (affichée une seule fois).
pub fn take_pending_reward() -> Option<PendingReward> {
    PENDING_REWARD.lock().unwrap().take()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleResult {
    Victory,
    Defeat,
}

#[derive(Debug, Clone)]
pub struct BattleRecord {
    pub result: BattleResult,
    pub xp: i32,
    pub gold: i32,
    pub loot: Option<&'static str>,
}

static HISTORY: Mutex<Vec<BattleRecord>> = Mutex::new(Vec::new());

pub fn record_battle(record: BattleRecord) {
    HISTORY.lock().unwrap().push(record);
}

pub fn battle_history() -> Vec<BattleRecord> {
    HISTORY.lock().unwrap().clone()
}

static INVENTORY: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub fn add_loot(item: &'static str) {
    INVENTORY.lock().unwrap().push(item);
}

pub fn inventory() -> Vec<&'static str> {
    INVENTORY.lock().unwrap().clone()
}
