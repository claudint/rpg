//! Session coop temps réel (specs section 6, Phase 3 étape 7) : synchronise
//! les positions des joueurs sur la carte d'exploration. Autoload Godot
//! (voir `project.godot`), présent à l'identique côté client et côté
//! serveur headless — ça garantit que les RPC ciblent le même chemin de
//! noeud des deux côtés sans passer par un `MultiplayerSpawner`.
//!
//! Un seul type gère les deux rôles, distingués par la présence de l'argument
//! `--server` (`Godot --headless --path rpg -- --server`) :
//! - côté serveur : héberge une partie ENet, autoritaire sur les positions
//!   (rejoue `world::grid::step`, la même logique pure que le solo).
//! - côté client : `connect_to` rejoint une partie ; `other_players` donne
//!   à `WorldScene` de quoi dessiner les joueurs distants.
//!
//! Hors scope pour cette étape (voir specs, étape 8 "combat coop") :
//! combats/villes partagés, réconciliation stricte serveur -> client (un
//! mouvement rejeté par le serveur est juste ignoré, pas de correction
//! forcée du client).

use std::collections::HashMap;

use godot::classes::{ENetMultiplayerPeer, INode, Node, Os};
use godot::prelude::*;
use serde::{Deserialize, Serialize};

use crate::world::grid::{self, Direction};
use crate::world::grid::GridPos;

const DEFAULT_PORT: i32 = 9000;
const MAX_PLAYERS: i32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemotePlayer {
    peer_id: i32,
    name: String,
    x: i32,
    y: i32,
}

#[derive(GodotClass)]
#[class(base=Node)]
pub struct CoopSession {
    base: Base<Node>,
    is_server: bool,
    /// Vrai côté client une fois `request_join` envoyée (serveur joignable
    /// et identité annoncée) : `notify_local_move` est un no-op tant que
    /// c'est faux, pour que le solo reste sans overhead réseau.
    connected: bool,
    pending_name: Option<String>,
    /// Position logique du joueur local au moment de la connexion (voir
    /// `connect_to`) : envoyée avec `request_join` pour que le serveur
    /// n'initialise pas tout le monde à (0,0) — sinon les joueurs déjà
    /// présents voient le nouveau arrivant "téléporter" depuis l'origine de
    /// la grille jusqu'à sa vraie case au premier déplacement.
    pending_position: Option<GridPos>,
    /// Positions connues de tous les joueurs (y compris soi-même côté
    /// client, filtré dans `other_players`). Côté serveur, c'est l'état
    /// autoritaire ; côté client, c'est une copie reçue via `sync_state`.
    roster: HashMap<i32, RemotePlayer>,
    /// Garde une référence Rust sur le peer ENet actif (serveur ou client) :
    /// `MultiplayerApi::set_multiplayer_peer` ne suffit pas à elle seule à
    /// éviter que gdext libère l'objet `RefCounted` si rien d'autre ne le
    /// référence.
    peer: Option<Gd<ENetMultiplayerPeer>>,
}

#[godot_api]
impl INode for CoopSession {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            is_server: false,
            connected: false,
            pending_name: None,
            pending_position: None,
            roster: HashMap::new(),
            peer: None,
        }
    }

    fn ready(&mut self) {
        let args: Vec<String> = Os::singleton().get_cmdline_user_args().to_vec().iter().map(GString::to_string).collect();
        if args.iter().any(|arg| arg == "--server") {
            let port = args
                .iter()
                .position(|arg| arg == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|p| p.parse::<i32>().ok())
                .unwrap_or(DEFAULT_PORT);
            self.start_server(port);
        }
    }
}

#[godot_api]
impl CoopSession {
    /// Héberge une partie coop. Bascule immédiatement sur une scène vide
    /// pour que `login.tscn` (scène de démarrage du client) ne s'exécute
    /// jamais en mode serveur — les autoloads sont prêts avant la scène
    /// principale, donc ce changement la préempte.
    fn start_server(&mut self, port: i32) {
        let mut peer = ENetMultiplayerPeer::new_gd();
        let err = peer.create_server_ex(port).max_clients(MAX_PLAYERS).done();
        if err != godot::global::Error::OK {
            godot_error!("CoopSession: impossible d'écouter sur le port {port} ({err:?})");
            return;
        }

        if let Some(mut multiplayer) = self.base().get_tree().get_multiplayer() {
            multiplayer.set_multiplayer_peer(&peer);

            let mut this = self.to_gd();
            let callable = Callable::from_fn("coop_peer_disconnected", move |args: &[&Variant]| {
                let peer_id = args[0].to::<i64>() as i32;
                this.bind_mut().on_peer_disconnected(peer_id);
                Variant::nil()
            });
            multiplayer.connect("peer_disconnected", &callable);
        }

        self.peer = Some(peer);
        self.is_server = true;
        godot_print!("CoopSession: serveur coop en écoute sur le port {port} (max {MAX_PLAYERS} joueurs)");

        // Différé : appelé depuis `ready()` d'un autoload, l'arbre de scène
        // est encore en train d'ajouter des noeuds (les autres autoloads) à
        // ce moment précis — un changement de scène synchrone ici échoue
        // ("Parent node is busy adding/removing children").
        let mut tree = self.base().get_tree();
        let path = Variant::from(GString::from("res://scenes/server_root.tscn"));
        tree.call_deferred("change_scene_to_file", &[path]);
    }

    /// Rejoint une partie coop hébergée à `address:port` sous le pseudo
    /// `name`, à partir de la case `pos` où le joueur se trouve déjà sur sa
    /// propre carte (position restaurée depuis la sauvegarde, pas
    /// forcément (0,0)). Appelée depuis le popup "Coop" de `WorldScene`.
    pub fn connect_to(&mut self, address: String, port: i32, name: String, pos: GridPos) {
        let mut peer = ENetMultiplayerPeer::new_gd();
        let err = peer.create_client(&address, port);
        if err != godot::global::Error::OK {
            godot_error!("CoopSession: connexion à {address}:{port} impossible ({err:?})");
            return;
        }
        godot_print!("CoopSession: ENet client créé vers {address}:{port}, en attente de la poignée de main");

        self.pending_name = Some(name);
        self.pending_position = Some(pos);

        if let Some(mut multiplayer) = self.base().get_tree().get_multiplayer() {
            multiplayer.set_multiplayer_peer(&peer);

            let mut this = self.to_gd();
            let callable = Callable::from_fn("coop_connected_to_server", move |_args: &[&Variant]| {
                this.bind_mut().on_connected_to_server();
                Variant::nil()
            });
            multiplayer.connect("connected_to_server", &callable);

            let callable = Callable::from_fn("coop_connection_failed", move |_args: &[&Variant]| {
                godot_error!("CoopSession: échec de connexion au serveur coop");
                Variant::nil()
            });
            multiplayer.connect("connection_failed", &callable);
        }

        self.peer = Some(peer);
    }

    fn on_connected_to_server(&mut self) {
        godot_print!("CoopSession: connecté au serveur");
        let Some(name) = self.pending_name.take() else {
            return;
        };
        let pos = self.pending_position.take().unwrap_or(GridPos::new(0, 0));
        self.connected = true;
        let _ = self.rpcs().request_join(&name, pos.x, pos.y).call_id(1);
    }

    fn on_peer_disconnected(&mut self, peer_id: i32) {
        self.roster.remove(&peer_id);
        self.broadcast_state();
    }

    /// Informe la session coop d'un déplacement local (appelée par
    /// `WorldScene::arrive_at`). No-op hors coop : le solo reste sans
    /// overhead réseau.
    pub fn notify_local_move(&mut self, dir: Direction) {
        if !self.connected {
            return;
        }
        let _ = self.rpcs().request_move(dir.to_code()).call_id(1);
    }

    /// Joueurs distants à dessiner (le joueur local est exclu : `WorldScene`
    /// dessine déjà sa propre position, éventuellement en cours de glissé).
    /// Vide hors coop : interroger `get_unique_id()` sur un peer pas encore
    /// (ou plus) actif fait une erreur Godot bruyante à chaque frame.
    pub fn other_players(&self) -> Vec<(String, GridPos)> {
        if !self.is_server && !self.connected {
            return Vec::new();
        }
        let local_id = self.base().get_multiplayer().map(|mp| mp.get_unique_id()).unwrap_or(0);
        self.roster
            .values()
            .filter(|player| player.peer_id != local_id)
            .map(|player| (player.name.clone(), GridPos::new(player.x, player.y)))
            .collect()
    }

    /// Diffuse l'état complet du roster à tous les clients (JSON, comme le
    /// reste du projet pour les échanges structurés, voir `persistence.rs`).
    /// Appelée côté serveur uniquement, après chaque join/move/leave.
    fn broadcast_state(&mut self) {
        let players: Vec<&RemotePlayer> = self.roster.values().collect();
        let Ok(json) = serde_json::to_string(&players) else {
            return;
        };
        let json = GString::from(&json);
        let _ = self.rpcs().sync_state(&json).call();
    }

    #[rpc(any_peer, reliable)]
    fn request_join(&mut self, name: GString, x: i32, y: i32) {
        let Some(multiplayer) = self.base().get_multiplayer() else {
            return;
        };
        let sender = multiplayer.get_remote_sender_id();
        godot_print!("CoopSession: {name} a rejoint (peer {sender}) à ({x}, {y})");
        self.roster.insert(sender, RemotePlayer { peer_id: sender, name: name.to_string(), x, y });
        self.broadcast_state();
    }

    #[rpc(any_peer, reliable)]
    fn request_move(&mut self, dir_code: i32) {
        let Some(multiplayer) = self.base().get_multiplayer() else {
            return;
        };
        let sender = multiplayer.get_remote_sender_id();

        let Some(dir) = Direction::from_code(dir_code) else {
            return;
        };
        let Some(player) = self.roster.get(&sender) else {
            return;
        };
        let Some(next) = grid::step(GridPos::new(player.x, player.y), dir, grid::WORLD_BOUNDS) else {
            return;
        };

        if let Some(player) = self.roster.get_mut(&sender) {
            player.x = next.x;
            player.y = next.y;
        }
        self.broadcast_state();
    }

    #[rpc(authority, reliable)]
    fn sync_state(&mut self, json: GString) {
        let Ok(players) = serde_json::from_str::<Vec<RemotePlayer>>(&json.to_string()) else {
            return;
        };
        self.roster = players.into_iter().map(|player| (player.peer_id, player)).collect();
    }
}
