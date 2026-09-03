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
use godot::global::randf;
use godot::prelude::*;
use serde::{Deserialize, Serialize};

use crate::battle::coop::{CoopBattle, CoopPhaseSnapshot, CoopSnapshot};
use crate::battle::engine::{self, BoardPos, Outcome, Side};
use crate::session;
use crate::world::encounter;
use crate::world::grid::{self, Direction};
use crate::world::grid::GridPos;
use crate::world::scene::ENCOUNTER_CHANCE;

const DEFAULT_PORT: i32 = 9000;
const MAX_PLAYERS: i32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemotePlayer {
    peer_id: i32,
    name: String,
    x: i32,
    y: i32,
}

/// Représentation réseau d'un combat coop (specs section 6, étape 8), reçue
/// par diffusion (`sync_battle`). Découplée des types internes de
/// `battle::coop`/`battle::engine`, même choix que `RemotePlayer` pour la
/// position sur la carte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CoopBattleUnitWire {
    pub(crate) owner: Option<i32>,
    pub(crate) name: String,
    pub(crate) side: u8, // 0 = joueur, 1 = ennemi
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) hp: i32,
    pub(crate) max_hp: i32,
    pub(crate) spell_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum CoopBattlePhaseWire {
    Placement { pending_counts: Vec<(i32, usize)> },
    Fight { current_turn_owner: Option<i32>, current_unit_name: String, current_unit_spell_ids: Vec<String> },
    End { victory: bool, xp: i32, gold: i32, loot: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CoopBattleWire {
    pub(crate) phase: CoopBattlePhaseWire,
    pub(crate) units: Vec<CoopBattleUnitWire>,
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
    /// Combat coop en cours, état autoritaire (serveur uniquement — `None`
    /// côté client, qui lit `remote_battle` à la place).
    battle: Option<CoopBattle>,
    /// Dernier état de combat reçu du serveur (client uniquement). Sa seule
    /// présence indique qu'un combat coop est affiché : `BattleScene` lit ce
    /// champ plutôt que de gérer un `BattleState` local.
    remote_battle: Option<CoopBattleWire>,
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
            battle: None,
            remote_battle: None,
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

    /// Un joueur déconnecté ne bloque pas les autres : ses personnages dans
    /// un combat coop en cours passent à l'IA (specs section 6, étape 8,
    /// limite acceptée : rien de spécial en phase de placement).
    fn on_peer_disconnected(&mut self, peer_id: i32) {
        self.roster.remove(&peer_id);
        self.broadcast_state();

        let has_battle = if let Some(battle) = &mut self.battle {
            battle.on_disconnect(peer_id);
            true
        } else {
            false
        };
        if has_battle {
            self.drive_and_broadcast_battle();
        }
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

    /// Vrai une fois la connexion coop établie (`WorldScene` s'en sert pour
    /// désactiver son propre jet de rencontre local, laissé au serveur).
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Vrai quand un combat coop est affiché (dernier état reçu du
    /// serveur) : `BattleScene` s'en sert pour savoir si elle doit se
    /// piloter localement (solo) ou attendre les diffusions du serveur.
    pub fn in_coop_battle(&self) -> bool {
        self.remote_battle.is_some()
    }

    /// Dernier état de combat coop reçu, pour affichage par `BattleScene`.
    pub fn remote_battle(&self) -> Option<&CoopBattleWire> {
        self.remote_battle.as_ref()
    }

    /// Identité réseau du joueur local, pour que `BattleScene` sache quelles
    /// unités du combat lui appartiennent (`CoopBattleUnitWire::owner`).
    pub fn local_peer_id(&self) -> i32 {
        self.base().get_multiplayer().map(|mp| mp.get_unique_id()).unwrap_or(0)
    }

    /// Demande au serveur de placer le prochain personnage en attente du
    /// joueur local sur `cell`. Appelée par `BattleScene` en coop, à la
    /// place de l'appel direct au moteur utilisé en solo.
    pub fn send_placement(&mut self, cell: GridPos) {
        let _ = self.rpcs().request_place(cell.x, cell.y).call_id(1);
    }

    /// Demande au serveur de résoudre une action pour le personnage du
    /// joueur local dont c'est le tour. Refusée silencieusement par le
    /// serveur si ce n'est pas son tour (voir `battle::coop::CoopBattle::act`).
    pub fn send_battle_action(&mut self, spell_id: &str, target: BoardPos) {
        let side = match target.side {
            Side::Player => 0,
            Side::Enemy => 1,
        };
        let _ = self.rpcs().request_battle_action(spell_id, side, target.cell.x, target.cell.y).call_id(1);
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

        // Même jet que le solo (`world::scene::ENCOUNTER_CHANCE`), rejoué
        // côté serveur : évite que deux joueurs déclenchent chacun leur
        // propre combat indépendamment (specs section 6, étape 8). Un seul
        // combat à la fois pour la session.
        if self.battle.is_none() && encounter::should_trigger(randf(), ENCOUNTER_CHANCE) {
            self.start_coop_battle();
        }
    }

    #[rpc(authority, reliable)]
    fn sync_state(&mut self, json: GString) {
        let Ok(players) = serde_json::from_str::<Vec<RemotePlayer>>(&json.to_string()) else {
            return;
        };
        self.roster = players.into_iter().map(|player| (player.peer_id, player)).collect();
    }

    /// Démarre un combat coop pour tous les joueurs actuellement connectés
    /// (specs section 6, étape 8 : "tous les joueurs connectés embarquent").
    fn start_coop_battle(&mut self) {
        let peers: Vec<i32> = self.roster.keys().copied().collect();
        if peers.is_empty() {
            return;
        }
        godot_print!("CoopSession: combat coop déclenché pour {} joueur(s)", peers.len());
        self.battle = Some(CoopBattle::start(&peers));
        self.broadcast_battle(None);
    }

    #[rpc(any_peer, reliable)]
    fn request_place(&mut self, x: i32, y: i32) {
        let Some(multiplayer) = self.base().get_multiplayer() else {
            return;
        };
        let sender = multiplayer.get_remote_sender_id();
        let Some(battle) = &mut self.battle else {
            return;
        };
        if battle.place(sender, GridPos::new(x, y)).is_err() {
            return;
        }
        self.drive_and_broadcast_battle();
    }

    #[rpc(any_peer, reliable)]
    fn request_battle_action(&mut self, spell_id: GString, target_side: i32, target_x: i32, target_y: i32) {
        let Some(multiplayer) = self.base().get_multiplayer() else {
            return;
        };
        let sender = multiplayer.get_remote_sender_id();
        let Some(battle) = &mut self.battle else {
            return;
        };
        let side = if target_side == 0 { Side::Player } else { Side::Enemy };
        let target = BoardPos { side, cell: GridPos::new(target_x, target_y) };
        if battle.act(sender, &spell_id.to_string(), target).is_err() {
            return;
        }
        self.drive_and_broadcast_battle();
    }

    /// Rejoue les tours IA en attente (ennemis, ou personnages dont le
    /// joueur s'est déconnecté) puis diffuse l'état résultant à tous les
    /// clients. Appelée après chaque placement/action qui a réellement
    /// changé l'état du combat.
    fn drive_and_broadcast_battle(&mut self) {
        let Some(battle) = &mut self.battle else {
            return;
        };
        let was_ongoing = battle.outcome().is_none();
        while battle.needs_ai_turn() {
            battle.resolve_ai_turn(randf());
        }

        // Le butin a besoin d'un tirage aléatoire (Godot) : décidé ici,
        // seule fois où la transition Fight -> End vient d'avoir lieu,
        // plutôt que dans `battle::coop` qui reste sans dépendance à Godot.
        let just_ended = was_ongoing && battle.outcome().is_some();
        let loot = if just_ended && battle.outcome() == Some(Outcome::Victory) {
            let index = ((randf() * engine::LOOT_TABLE.len() as f64) as usize).min(engine::LOOT_TABLE.len() - 1);
            Some(engine::LOOT_TABLE[index].to_string())
        } else {
            None
        };

        self.broadcast_battle(loot);
        if just_ended {
            // État renvoyé, plus besoin de le garder côté serveur : prêt
            // pour un prochain combat.
            self.battle = None;
        }
    }

    fn broadcast_battle(&mut self, loot: Option<String>) {
        let Some(battle) = &self.battle else {
            return;
        };
        let wire = to_battle_wire(&battle.snapshot(), loot);
        let Ok(json) = serde_json::to_string(&wire) else {
            return;
        };
        let json = GString::from(&json);
        let _ = self.rpcs().sync_battle(&json).call();
    }

    #[rpc(authority, reliable)]
    fn sync_battle(&mut self, json: GString) {
        let Ok(wire) = serde_json::from_str::<CoopBattleWire>(&json.to_string()) else {
            return;
        };

        if let CoopBattlePhaseWire::End { victory, xp, gold, loot } = &wire.phase {
            self.apply_coop_battle_end(*victory, *xp, *gold, loot.clone());
            return;
        }

        let is_new_battle = self.remote_battle.is_none();
        self.remote_battle = Some(wire);
        if is_new_battle {
            let mut tree = self.base().get_tree();
            let path = Variant::from(GString::from("res://scenes/battle.tscn"));
            tree.call_deferred("change_scene_to_file", &[path]);
        }
    }

    /// Fin de combat coop : chaque client applique la même récompense à son
    /// propre état local (pas de logique côté serveur pour répartir quoi
    /// que ce soit, exactement comme `BattleScene::enter_end_state` en
    /// solo), puis revient sur la carte. Pas d'écran de victoire/défaite
    /// dédié en coop (contrairement au solo) : le retour est immédiat pour
    /// tout le monde, plus simple que de gérer un bouton "Retour" par
    /// joueur sur un combat partagé.
    fn apply_coop_battle_end(&mut self, victory: bool, xp: i32, gold: i32, loot: Option<String>) {
        self.remote_battle = None;

        if victory {
            let known = crate::battle::known_loot_items();
            let resolved_loot = loot.as_deref().and_then(|name| known.iter().find(|&&item| item == name).copied());

            session::add_rewards(xp, gold);
            if let Some(loot) = resolved_loot {
                session::add_loot(loot);
                session::queue_reward(session::PendingReward { xp, gold, loot });
            }
            session::record_battle(session::BattleRecord {
                result: session::BattleResult::Victory,
                xp,
                gold,
                loot: resolved_loot,
            });
            self.trigger_auto_save();
        } else {
            session::record_battle(session::BattleRecord {
                result: session::BattleResult::Defeat,
                xp: 0,
                gold: 0,
                loot: None,
            });
        }

        let mut tree = self.base().get_tree();
        let path = Variant::from(GString::from("res://scenes/world.tscn"));
        tree.call_deferred("change_scene_to_file", &[path]);
    }

    /// Même mécanique que `BattleScene::trigger_auto_save` en solo :
    /// `DevConsole` est le seul noeud qui porte les requêtes HTTP vers
    /// `backend-api`, atteint par son chemin absolu d'autoload.
    fn trigger_auto_save(&self) {
        let Some(root) = self.base().get_tree().get_root() else {
            return;
        };
        let Some(node) = root.get_node_or_null("DevConsole") else {
            return;
        };
        let Ok(console) = node.try_cast::<crate::dev_console::DevConsole>() else {
            return;
        };
        console.bind().save();
    }
}

fn to_battle_wire(snapshot: &CoopSnapshot, loot: Option<String>) -> CoopBattleWire {
    let phase = match &snapshot.phase {
        CoopPhaseSnapshot::Placement { pending_counts } => {
            CoopBattlePhaseWire::Placement { pending_counts: pending_counts.iter().map(|(&p, &c)| (p, c)).collect() }
        }
        CoopPhaseSnapshot::Fight { current_turn_owner, current_unit_name, current_unit_spell_ids } => {
            CoopBattlePhaseWire::Fight {
                current_turn_owner: *current_turn_owner,
                current_unit_name: current_unit_name.clone(),
                current_unit_spell_ids: current_unit_spell_ids.clone(),
            }
        }
        CoopPhaseSnapshot::End { outcome, rewards } => {
            let (xp, gold) = rewards.map(|r| (r.xp, r.gold)).unwrap_or((0, 0));
            CoopBattlePhaseWire::End { victory: *outcome == Outcome::Victory, xp, gold, loot }
        }
    };

    let units = snapshot
        .units
        .iter()
        .map(|unit| CoopBattleUnitWire {
            owner: unit.owner,
            name: unit.name.clone(),
            side: match unit.side {
                Side::Player => 0,
                Side::Enemy => 1,
            },
            x: unit.x,
            y: unit.y,
            hp: unit.hp,
            max_hp: unit.max_hp,
            spell_ids: unit.spell_ids.clone(),
        })
        .collect();

    CoopBattleWire { phase, units }
}
