//! Sauvegarde de la progression via l'API backend (specs section 4-5) : le
//! jeu ne parle jamais directement à Postgres, seulement en HTTP à
//! `backend-api` (http://127.0.0.1:8080 par défaut, lancé séparément avec
//! `cargo run` dans backend-api/). Les requêtes elles-mêmes (noeuds
//! `HTTPRequest`, lecture des réponses) sont pilotées depuis
//! `dev_console.rs`, seul noeud qui persiste entre les écrans ; ce module ne
//! fait que construire/lire les données JSON échangées.

use godot::classes::HttpRequest;
use godot::classes::http_client::Method;
use godot::prelude::*;
use serde::{Deserialize, Serialize};

use crate::battle;
use crate::session;
use crate::world::grid::GridPos;

const BASE_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveData {
    pub xp: i32,
    pub gold: i32,
    pub pos_x: i32,
    pub pos_y: i32,
    pub inventory: Vec<String>,
    pub history: Vec<SavedBattleRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedBattleRecord {
    pub victory: bool,
    pub xp: i32,
    pub gold: i32,
    pub loot: Option<String>,
}

impl SaveData {
    /// Instantané de l'état courant (`session.rs`), prêt à envoyer au backend.
    pub fn snapshot() -> Self {
        let progress = session::progress();
        let pos = session::return_point();
        let inventory = session::inventory().into_iter().map(|s| s.to_string()).collect();
        let history = session::battle_history()
            .into_iter()
            .map(|r| SavedBattleRecord {
                victory: r.result == session::BattleResult::Victory,
                xp: r.xp,
                gold: r.gold,
                loot: r.loot.map(|s| s.to_string()),
            })
            .collect();

        Self { xp: progress.xp, gold: progress.gold, pos_x: pos.x, pos_y: pos.y, inventory, history }
    }

    /// Réinjecte cet état dans `session.rs` (chargement d'une sauvegarde).
    /// Un nom de butin sauvegardé qui ne correspond plus à rien de connu
    /// (table de butin modifiée entre-temps) est simplement ignoré.
    pub fn apply(self) {
        session::set_progress(self.xp, self.gold);
        session::set_return_point(GridPos::new(self.pos_x, self.pos_y));

        let known = battle::known_loot_items();
        let resolve = |name: &str| known.iter().find(|s| **s == name).copied();

        let inventory = self.inventory.iter().filter_map(|name| resolve(name)).collect();
        session::set_inventory(inventory);

        let history = self
            .history
            .into_iter()
            .map(|r| session::BattleRecord {
                result: if r.victory { session::BattleResult::Victory } else { session::BattleResult::Defeat },
                xp: r.xp,
                gold: r.gold,
                loot: r.loot.as_deref().and_then(resolve),
            })
            .collect();
        session::set_history(history);
    }
}

/// Lance une requête `PUT /save` avec l'état courant. La réponse est ignorée
/// côté jeu (pas besoin de savoir si ça a marché pour l'instant).
pub fn request_save(http: &mut Gd<HttpRequest>) {
    let data = SaveData::snapshot();
    let Ok(body) = serde_json::to_string(&data) else {
        return;
    };

    let mut headers = PackedStringArray::new();
    headers.push("Content-Type: application/json");

    let url = format!("{BASE_URL}/save");
    let _ = http.request_ex(&url).method(Method::PUT).custom_headers(&headers).request_data(&body).done();
}

/// Lance une requête `GET /save`. La réponse arrive plus tard via le signal
/// `request_completed` du même noeud ; voir `parse_response`.
pub fn request_load(http: &mut Gd<HttpRequest>) {
    let url = format!("{BASE_URL}/save");
    let _ = http.request(&url);
}

/// Parse le corps d'une réponse `GET /save` réussie. `None` si le JSON est
/// invalide ou si la requête a échoué (backend éteint, etc.) — dans ce cas
/// on garde simplement l'état par défaut, pas d'erreur bloquante.
pub fn parse_response(body: &PackedByteArray) -> Option<SaveData> {
    let text = body.get_string_from_utf8().to_string();
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_data_round_trips_through_json() {
        let data = SaveData {
            xp: 42,
            gold: 10,
            pos_x: 3,
            pos_y: 1,
            inventory: vec!["Potion de soin".to_string()],
            history: vec![SavedBattleRecord {
                victory: true,
                xp: 15,
                gold: 10,
                loot: Some("Vieille carte".to_string()),
            }],
        };

        let json = serde_json::to_string(&data).unwrap();
        let back: SaveData = serde_json::from_str(&json).unwrap();

        assert_eq!(back.xp, 42);
        assert_eq!(back.inventory, vec!["Potion de soin".to_string()]);
        assert_eq!(back.history.len(), 1);
        assert_eq!(back.history[0].loot.as_deref(), Some("Vieille carte"));
    }
}
