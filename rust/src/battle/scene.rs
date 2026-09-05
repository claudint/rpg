//! Ecran de combat : plateau 3x4 par camp, placement, tour par tour avec IA
//! simple côté ennemi (specs, section 6 étape 4). L'état de la bataille lui-
//! même vit dans `battle::engine`, pur ; ce fichier ne fait que l'afficher et
//! traduire les clics en actions.
//!
//! Piège Rust récurrent ici : on ne peut pas appeler une méthode `&mut self`
//! (ex. `self.base_mut()`) pendant qu'un champ de `self` (ex. `self.phase`)
//! est encore emprunté par un `match`/`if let`. La solution utilisée partout
//! dans ce fichier : calculer d'abord ce dont on a besoin (valeurs possédées,
//! pas des emprunts) dans un bloc séparé, puis appeler les méthodes `&mut
//! self` après, une fois ce bloc terminé.

use std::collections::HashMap;

use godot::classes::control::MouseFilter;
use godot::classes::{
    Button, ColorRect, Control, IControl, InputEvent, InputEventMouseButton, InputEventMouseMotion, Label,
};
use godot::global::{randf, MouseButton};
use godot::prelude::*;

use crate::dev_console::DevConsole;
use crate::geometry::GridPos;
use crate::network::{CoopBattlePhaseWire, CoopBattleWire, CoopSession};
use crate::session;

use super::data;
use super::engine::{self, BattleState, BoardPos, Outcome, Side, TargetKind, UnitId, BOARD_BOUNDS};
use super::pattern;

const CELL: f32 = 50.0;
const PLAYER_ORIGIN: Vector2 = Vector2::new(60.0, 160.0);
const ENEMY_ORIGIN: Vector2 = Vector2::new(420.0, 160.0);

fn board_origin(side: Side) -> Vector2 {
    match side {
        Side::Player => PLAYER_ORIGIN,
        Side::Enemy => ENEMY_ORIGIN,
    }
}

fn cell_screen_pos(pos: BoardPos) -> Vector2 {
    board_origin(pos.side) + Vector2::new(pos.cell.x as f32 * CELL, pos.cell.y as f32 * CELL)
}

fn board_pos_at(local: Vector2) -> Option<BoardPos> {
    for side in [Side::Player, Side::Enemy] {
        let rel = local - board_origin(side);
        if rel.x < 0.0 || rel.y < 0.0 {
            continue;
        }
        let cell = GridPos::new((rel.x / CELL) as i32, (rel.y / CELL) as i32);
        if BOARD_BOUNDS.contains(cell) {
            return Some(BoardPos { side, cell });
        }
    }
    None
}

enum Phase {
    Placement {
        pending: Vec<data::UnitDef>,
        placed: Vec<engine::Unit>,
    },
    Fight {
        battle: BattleState,
        pending_spell: Option<String>,
        hover: Option<BoardPos>,
        labels: HashMap<UnitId, Gd<Label>>,
    },
    /// Défaite uniquement : la victoire ne s'arrête pas ici, elle repart
    /// directement sur la carte (voir `enter_end_state`) avec la récompense
    /// en attente dans `session::PendingReward`, affichée là-bas.
    End(Outcome),
}

#[derive(GodotClass)]
#[class(base=Control)]
pub struct BattleScene {
    base: Base<Control>,
    phase: Phase,
    title: Option<Gd<Label>>,
    ui_container: Option<Gd<Control>>,
    /// Vrai si ce combat est piloté par un `CoopSession` distant (specs
    /// section 6, étape 8) plutôt que localement : `self.phase` reste alors
    /// la valeur par défaut d'`init()`, jamais utilisée — tout l'affichage
    /// et les clics passent par `coop_last`/les méthodes `*_coop_*`.
    coop: bool,
    /// Sort choisi par le joueur local en attendant qu'il clique une cible
    /// (équivalent coop de `Phase::Fight::pending_spell`, qui n'existe pas
    /// puisqu'il n'y a pas de `BattleState` local en coop).
    coop_pending_spell: Option<String>,
    /// Dernier état reçu de `CoopSession::remote_battle()`. Mis en cache
    /// (plutôt que relu à chaque frame) pour détecter les changements et ne
    /// reconstruire l'UI (`refresh_ui`) que lorsque l'état a vraiment
    /// changé, pas à chaque frame.
    coop_last: Option<CoopBattleWire>,
}

#[godot_api]
impl IControl for BattleScene {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            phase: Phase::Placement { pending: data::default_player_roster(), placed: Vec::new() },
            title: None,
            ui_container: None,
            coop: false,
            coop_pending_spell: None,
            coop_last: None,
        }
    }

    fn ready(&mut self) {
        // Un Control absorbe les clics par défaut (mouse_filter = Stop), ce
        // qui les empêche d'arriver jusqu'à `unhandled_input`. On l'ignore
        // explicitement partout où on ne veut pas intercepter les clics sur
        // le plateau — seuls les boutons doivent rester cliquables.
        self.base_mut().set_mouse_filter(MouseFilter::IGNORE);

        let viewport_size = self.base().get_viewport_rect().size;

        // Un enfant se dessine toujours par-dessus le `_draw()` de son
        // parent : sans z_index négatif, ce fond plein écran cacherait
        // complètement la grille et les unités dessinées plus bas.
        let mut background = ColorRect::new_alloc();
        background.set_size(viewport_size);
        background.set_color(Color::from_rgb(0.18, 0.18, 0.22));
        background.set_mouse_filter(MouseFilter::IGNORE);
        background.set_z_index(-1);
        self.base_mut().add_child(&background);

        let mut title = Label::new_alloc();
        title.set_position(Vector2::new(40.0, 20.0));
        self.base_mut().add_child(&title);
        self.title = Some(title);

        let mut ui_container = Control::new_alloc();
        ui_container.set_mouse_filter(MouseFilter::IGNORE);
        self.base_mut().add_child(&ui_container);
        self.ui_container = Some(ui_container);

        self.coop = self.coop_session().is_some_and(|coop| coop.bind().in_coop_battle());

        // "Fuir" n'a pas de sens sur un combat partagé pour cette itération
        // (abandonner l'équipe des autres joueurs en cours de combat n'est
        // pas géré) — masqué en coop plutôt que half-implémenté.
        if !self.coop {
            let this = self.to_gd();
            let mut flee_button = Button::new_alloc();
            flee_button.set_position(Vector2::new(600.0, 20.0));
            flee_button.set_size(Vector2::new(120.0, 40.0));
            flee_button.set_text("Fuir");
            flee_button.connect("pressed", &Callable::from_object_method(&this, "on_flee_pressed"));
            self.base_mut().add_child(&flee_button);
        }

        if self.coop {
            self.coop_last = self.coop_battle_snapshot();
        }
        self.refresh_ui();
    }

    fn process(&mut self, _delta: f64) {
        if self.coop {
            self.poll_coop_battle();
        }
        self.base_mut().queue_redraw();
    }

    fn draw(&mut self) {
        self.draw_board(Side::Player);
        self.draw_board(Side::Enemy);
        if self.coop {
            self.draw_coop_units();
        } else {
            self.draw_units();
            self.draw_hover_preview();
        }
    }

    fn unhandled_input(&mut self, event: Gd<InputEvent>) {
        if let Some(local) = self.mouse_click_local(&event) {
            if let Some(target) = board_pos_at(local) {
                self.handle_click(target);
            }
            return;
        }
        if let Some(local) = self.mouse_motion_local(&event) {
            self.handle_hover(board_pos_at(local));
        }
    }
}

#[godot_api]
impl BattleScene {
    #[func]
    fn on_flee_pressed(&mut self) {
        self.base().get_tree().change_scene_to_file("res://scenes/world.tscn");
    }
}

impl BattleScene {
    /// Soigne toute l'équipe joueur à fond (commande de dev `heal_team`).
    /// Erreur s'il n'y a pas de combat en cours (encore en placement, ou déjà
    /// terminé).
    pub fn heal_player_team(&mut self) -> Result<(), &'static str> {
        let healed = if let Phase::Fight { battle, .. } = &mut self.phase {
            for unit in battle.units.iter_mut() {
                if unit.side == Side::Player {
                    unit.hp = unit.max_hp;
                }
            }
            true
        } else {
            false
        };

        if healed {
            self.update_unit_labels();
            Ok(())
        } else {
            Err("pas de combat en cours")
        }
    }

    fn mouse_click_local(&self, event: &Gd<InputEvent>) -> Option<Vector2> {
        let mouse_event = event.clone().try_cast::<InputEventMouseButton>().ok()?;
        if mouse_event.get_button_index() != MouseButton::LEFT || !mouse_event.is_pressed() {
            return None;
        }
        Some(self.base().get_local_mouse_position())
    }

    fn mouse_motion_local(&self, event: &Gd<InputEvent>) -> Option<Vector2> {
        event.clone().try_cast::<InputEventMouseMotion>().ok()?;
        Some(self.base().get_local_mouse_position())
    }

    fn handle_click(&mut self, target: BoardPos) {
        if self.coop {
            self.handle_coop_click(target);
            return;
        }
        let in_placement = matches!(self.phase, Phase::Placement { .. });
        if in_placement {
            self.try_place(target);
        } else if matches!(self.phase, Phase::Fight { .. }) {
            self.try_target(target);
        }
    }

    fn handle_hover(&mut self, target: Option<BoardPos>) {
        if !matches!(self.phase, Phase::Fight { .. }) {
            return;
        }
        if let Phase::Fight { hover, .. } = &mut self.phase {
            *hover = target;
        }
        self.base_mut().queue_redraw();
    }

    fn try_place(&mut self, target: BoardPos) {
        if target.side != Side::Player {
            return;
        }

        let done = {
            let Phase::Placement { pending, placed } = &mut self.phase else {
                return;
            };
            if pending.is_empty() || placed.iter().any(|u| u.pos.cell == target.cell) {
                return;
            }
            let def = pending.remove(0);
            placed.push(def.into_unit(Side::Player, target));
            pending.is_empty()
        };

        if done {
            self.start_fight();
        } else {
            self.refresh_ui();
        }
        self.base_mut().queue_redraw();
    }

    fn start_fight(&mut self) {
        let mut units = match &self.phase {
            Phase::Placement { placed, .. } => placed.clone(),
            _ => return,
        };

        for (i, def) in data::default_enemy_roster().into_iter().enumerate() {
            let cell = GridPos::new(1, i as i32);
            units.push(def.into_unit(Side::Enemy, BoardPos { side: Side::Enemy, cell }));
        }

        let battle = BattleState::new(units, data::spells());
        self.phase = Phase::Fight { battle, pending_spell: None, hover: None, labels: HashMap::new() };

        self.build_unit_labels();
        self.drive_until_player_or_end();
    }

    fn try_target(&mut self, target: BoardPos) {
        let action = {
            let Phase::Fight { battle, pending_spell, .. } = &self.phase else {
                return;
            };
            let Some(spell_id) = pending_spell.clone() else {
                return;
            };
            let caster_id = battle.current_unit_id();
            let caster = battle.unit(caster_id).clone();
            if caster.side != Side::Player {
                return;
            }
            let Some(spell) = battle.spell(&spell_id).cloned() else {
                return;
            };

            let expected_side = match spell.target {
                TargetKind::Enemy => engine::opposite(caster.side),
                TargetKind::Ally => caster.side,
            };
            if target.side != expected_side {
                return;
            }

            spell_id
        };

        self.resolve_and_advance(&action, target);
        self.drive_until_player_or_end();
    }

    fn resolve_and_advance(&mut self, spell_id: &str, target: BoardPos) {
        if let Phase::Fight { battle, pending_spell, .. } = &mut self.phase {
            battle.resolve_action(spell_id, target);
            battle.advance_turn();
            *pending_spell = None;
        }
        self.update_unit_labels();
    }

    fn advance_turn_only(&mut self) {
        if let Phase::Fight { battle, pending_spell, .. } = &mut self.phase {
            battle.advance_turn();
            *pending_spell = None;
        }
    }

    /// Résout automatiquement tous les tours ennemis qui suivent, jusqu'à ce
    /// que ce soit de nouveau au joueur d'agir ou que le combat soit terminé.
    fn drive_until_player_or_end(&mut self) {
        loop {
            let stop = match &self.phase {
                Phase::Fight { battle, .. } => {
                    battle.outcome().is_some() || battle.unit(battle.current_unit_id()).side == Side::Player
                }
                _ => true,
            };
            if stop {
                break;
            }
            self.run_ai_turn();
        }

        let outcome = match &self.phase {
            Phase::Fight { battle, .. } => battle.outcome(),
            _ => None,
        };

        match outcome {
            Some(outcome) => self.enter_end_state(outcome),
            None => self.refresh_ui(),
        }
    }

    fn run_ai_turn(&mut self) {
        let action = {
            let Phase::Fight { battle, .. } = &self.phase else {
                return;
            };
            engine::choose_ai_action(battle, battle.current_unit_id(), randf())
        };

        match action {
            Some((spell_id, target)) => self.resolve_and_advance(&spell_id, target),
            None => {
                self.advance_turn_only();
                self.update_unit_labels();
            }
        }
    }

    /// La victoire n'affiche plus d'écran ici : elle calcule la récompense,
    /// la met en attente pour la carte, et y repart tout de suite. Seule la
    /// défaite garde un écran dédié (rien à récapituler, et la pénalité de
    /// défaite reste à définir — specs, section 9).
    fn enter_end_state(&mut self, outcome: Outcome) {
        if outcome == Outcome::Victory {
            if let Phase::Fight { battle, .. } = &self.phase {
                let rewards = battle.victory_rewards();
                let loot_index = ((randf() * engine::LOOT_TABLE.len() as f64) as usize).min(engine::LOOT_TABLE.len() - 1);
                let loot = engine::LOOT_TABLE[loot_index];
                session::add_rewards(rewards.xp, rewards.gold);
                session::add_loot(loot);
                session::queue_reward(session::PendingReward { xp: rewards.xp, gold: rewards.gold, loot });
                session::record_battle(session::BattleRecord {
                    result: session::BattleResult::Victory,
                    xp: rewards.xp,
                    gold: rewards.gold,
                    loot: Some(loot),
                });
            }
            self.trigger_auto_save();
            self.base().get_tree().change_scene_to_file("res://scenes/world.tscn");
            return;
        }

        session::record_battle(session::BattleRecord {
            result: session::BattleResult::Defeat,
            xp: 0,
            gold: 0,
            loot: None,
        });

        self.phase = Phase::End(outcome);
        self.refresh_ui();
        self.base_mut().queue_redraw();
    }

    /// Sauvegarde la progression via le backend (specs, Phase 2). `DevConsole`
    /// est le seul noeud qui persiste entre les écrans et porte les requêtes
    /// HTTP, on l'atteint donc via le chemin absolu de l'autoload plutôt que
    /// via une dépendance directe entre modules de scène.
    fn trigger_auto_save(&self) {
        let Some(root) = self.base().get_tree().get_root() else {
            return;
        };
        let Some(node) = root.get_node_or_null("DevConsole") else {
            return;
        };
        let Ok(console) = node.try_cast::<DevConsole>() else {
            return;
        };
        console.bind().save();
    }

    fn select_spell(&mut self, spell_id: String) {
        if self.coop {
            self.coop_pending_spell = Some(spell_id);
        } else if let Phase::Fight { pending_spell, .. } = &mut self.phase {
            *pending_spell = Some(spell_id);
        }
        self.base_mut().queue_redraw();
    }

    fn build_unit_labels(&mut self) {
        let entries: Vec<(UnitId, Vector2, String)> = match &self.phase {
            Phase::Fight { battle, .. } => battle
                .units
                .iter()
                .enumerate()
                .map(|(id, u)| {
                    let pos = cell_screen_pos(u.pos) + Vector2::new(-4.0, -16.0);
                    (id, pos, format!("{}\n{}/{}", u.name, u.hp, u.max_hp))
                })
                .collect(),
            _ => Vec::new(),
        };

        for (id, pos, text) in entries {
            let mut label = Label::new_alloc();
            label.set_position(pos);
            label.set_text(&text);
            self.base_mut().add_child(&label);

            if let Phase::Fight { labels, .. } = &mut self.phase {
                labels.insert(id, label);
            }
        }
    }

    fn update_unit_labels(&mut self) {
        let updates: Vec<(UnitId, String)> = match &self.phase {
            Phase::Fight { battle, .. } => battle
                .units
                .iter()
                .enumerate()
                .map(|(id, u)| {
                    let suffix = if u.is_alive() { "" } else { " (K.O.)" };
                    (id, format!("{}\n{}/{}{}", u.name, u.hp.max(0), u.max_hp, suffix))
                })
                .collect(),
            _ => Vec::new(),
        };

        if let Phase::Fight { labels, .. } = &mut self.phase {
            for (id, text) in updates {
                if let Some(label) = labels.get_mut(&id) {
                    label.set_text(&text);
                }
            }
        }
        self.base_mut().queue_redraw();
    }

    fn refresh_ui(&mut self) {
        self.clear_ui_container();

        if self.coop {
            self.refresh_coop_ui();
            return;
        }

        enum UiPlan {
            None,
            SpellButtons(Vec<String>),
            ReturnButton,
        }

        let (title_text, plan) = match &self.phase {
            Phase::Placement { pending, .. } => {
                let text = match pending.first() {
                    Some(next) => format!("Placement — clique une case de ton plateau pour poser : {}", next.name),
                    None => "Placement terminée".to_string(),
                };
                (text, UiPlan::None)
            }
            Phase::Fight { battle, .. } => {
                let unit = battle.unit(battle.current_unit_id());
                let text = format!("Tour de {}", unit.name);
                let plan = if unit.side == Side::Player {
                    UiPlan::SpellButtons(unit.spell_ids.clone())
                } else {
                    UiPlan::None
                };
                (text, plan)
            }
            Phase::End(Outcome::Victory) => ("Victoire !".to_string(), UiPlan::ReturnButton),
            Phase::End(Outcome::Defeat) => ("Défaite...".to_string(), UiPlan::ReturnButton),
        };

        self.set_title(&title_text);

        match plan {
            UiPlan::SpellButtons(spell_ids) => self.build_spell_buttons(spell_ids),
            UiPlan::ReturnButton => self.build_return_button(),
            UiPlan::None => {}
        }
    }

    fn set_title(&mut self, text: &str) {
        if let Some(title) = &mut self.title {
            title.set_text(text);
        }
    }

    fn clear_ui_container(&mut self) {
        let Some(container) = &self.ui_container else {
            return;
        };
        for mut child in container.get_children().iter_shared() {
            child.queue_free();
        }
    }

    /// Résout les noms de sorts depuis `data::spells()` (la liste statique,
    /// pas `self.phase`) : fonctionne aussi bien en solo qu'en coop, où il
    /// n'y a pas de `BattleState` local à consulter.
    fn build_spell_buttons(&mut self, spell_ids: Vec<String>) {
        let spells = data::spells();
        let spell_names: Vec<(String, String)> =
            spell_ids.iter().filter_map(|id| spells.iter().find(|s| &s.id == id).map(|s| (id.clone(), s.name.clone()))).collect();

        let Some(mut container) = self.ui_container.clone() else {
            return;
        };
        let this = self.to_gd();

        for (i, (spell_id, name)) in spell_names.into_iter().enumerate() {
            let mut button = Button::new_alloc();
            button.set_position(Vector2::new(40.0 + i as f32 * 160.0, 480.0));
            button.set_size(Vector2::new(150.0, 40.0));
            button.set_text(&name);

            let mut target = this.clone();
            let callable = Callable::from_fn("select_spell", move |_args: &[&Variant]| {
                target.bind_mut().select_spell(spell_id.clone());
                Variant::nil()
            });
            button.connect("pressed", &callable);

            container.add_child(&button);
        }
    }

    fn build_return_button(&mut self) {
        let Some(mut container) = self.ui_container.clone() else {
            return;
        };
        let this = self.to_gd();

        let mut button = Button::new_alloc();
        button.set_position(Vector2::new(40.0, 480.0));
        button.set_size(Vector2::new(150.0, 40.0));
        button.set_text("Retour");
        button.connect("pressed", &Callable::from_object_method(&this, "on_flee_pressed"));
        container.add_child(&button);
    }

    fn draw_board(&mut self, side: Side) {
        let origin = board_origin(side);
        let line_color = Color::from_rgb(0.4, 0.4, 0.45);

        for x in 0..BOARD_BOUNDS.width {
            for y in 0..BOARD_BOUNDS.height {
                let rect = Rect2::new(
                    origin + Vector2::new(x as f32 * CELL, y as f32 * CELL),
                    Vector2::new(CELL, CELL),
                );
                self.base_mut().draw_rect_ex(rect, line_color).filled(false).width(1.0).done();
            }
        }
    }

    fn draw_units(&mut self) {
        let units: Vec<(BoardPos, bool)> = match &self.phase {
            Phase::Placement { placed, .. } => placed.iter().map(|u| (u.pos, true)).collect(),
            Phase::Fight { battle, .. } => battle.units.iter().map(|u| (u.pos, u.is_alive())).collect(),
            Phase::End(_) => Vec::new(),
        };

        for (pos, alive) in units {
            if !alive {
                continue;
            }
            let color = match pos.side {
                Side::Player => Color::from_rgb(0.3, 0.7, 1.0),
                Side::Enemy => Color::from_rgb(0.9, 0.3, 0.3),
            };
            let margin = 8.0;
            let rect = Rect2::new(
                cell_screen_pos(pos) + Vector2::new(margin, margin),
                Vector2::new(CELL - margin * 2.0, CELL - margin * 2.0),
            );
            self.base_mut().draw_rect(rect, color);
        }
    }

    /// Prévisualisation des cases touchées (specs, section 3.3), calculée
    /// dans l'espace combiné des deux plateaux : elle montre donc aussi le
    /// débordement d'un sort de zone ancré près de la frontière commune.
    fn draw_hover_preview(&mut self) {
        let cells: Vec<BoardPos> = match &self.phase {
            Phase::Fight { battle, pending_spell: Some(spell_id), hover: Some(hover), .. } => battle
                .spell(spell_id)
                .map(|spell| {
                    pattern::cells(engine::to_combined(*hover), spell.pattern, engine::COMBINED_BOUNDS)
                        .into_iter()
                        .map(engine::from_combined)
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let overlay = Color::from_rgba(1.0, 1.0, 0.3, 0.4);
        for pos in cells {
            let rect = Rect2::new(cell_screen_pos(pos), Vector2::new(CELL, CELL));
            self.base_mut().draw_rect(rect, overlay);
        }
    }

    // --- Combat coop (specs section 6, étape 8) ---
    //
    // Contrairement au solo, il n'y a pas de `BattleState` local ici : tout
    // vient de `CoopSession::remote_battle()` (dernière diffusion reçue du
    // serveur, autoritaire) et tous les clics passent par des requêtes RPC
    // (`send_placement`/`send_battle_action`) plutôt que d'appeler le
    // moteur directement — même principe que `WorldScene` pour la position.

    /// Accès à l'autoload `CoopSession`, même pattern que `save_game`/
    /// `WorldScene::coop_session` : chemin absolu depuis la racine de
    /// l'arbre de scène.
    fn coop_session(&self) -> Option<Gd<CoopSession>> {
        let root = self.base().get_tree().get_root()?;
        let node = root.get_node_or_null("CoopSession")?;
        node.try_cast::<CoopSession>().ok()
    }

    fn coop_battle_snapshot(&self) -> Option<CoopBattleWire> {
        self.coop_session()?.bind().remote_battle().cloned()
    }

    /// Appelée chaque frame (`process`) : ne reconstruit l'UI que lorsque
    /// l'état reçu a effectivement changé, pour éviter de reconstruire les
    /// boutons de sort à chaque image.
    fn poll_coop_battle(&mut self) {
        let current = self.coop_battle_snapshot();
        if current != self.coop_last {
            self.coop_last = current;
            self.refresh_ui();
        }
    }

    fn draw_coop_units(&mut self) {
        let Some(wire) = self.coop_last.clone() else {
            return;
        };
        for unit in &wire.units {
            if unit.hp <= 0 {
                continue;
            }
            let side = if unit.side == 0 { Side::Player } else { Side::Enemy };
            let color = match side {
                Side::Player => Color::from_rgb(0.3, 0.7, 1.0),
                Side::Enemy => Color::from_rgb(0.9, 0.3, 0.3),
            };
            let margin = 8.0;
            let pos = BoardPos { side, cell: GridPos::new(unit.x, unit.y) };
            let rect = Rect2::new(
                cell_screen_pos(pos) + Vector2::new(margin, margin),
                Vector2::new(CELL - margin * 2.0, CELL - margin * 2.0),
            );
            self.base_mut().draw_rect(rect, color);
        }
    }

    fn handle_coop_click(&mut self, target: BoardPos) {
        let Some(wire) = self.coop_last.clone() else {
            return;
        };
        let Some(mut coop) = self.coop_session() else {
            return;
        };
        let local_peer = coop.bind().local_peer_id();

        match &wire.phase {
            CoopBattlePhaseWire::Placement { sides, .. } => {
                // Plateau attribué au joueur local (specs section 6, étape
                // 9-10, PvP) : toujours `Side::Player` en coop PvE, peut
                // être `Side::Enemy` pour le défenseur d'un duel PvP.
                let expected = sides.iter().find(|(peer, _)| *peer == local_peer).map(|(_, s)| *s == 1);
                let target_is_enemy_side = target.side == Side::Enemy;
                if expected != Some(target_is_enemy_side) {
                    return;
                }
                coop.bind_mut().send_placement(target.cell);
            }
            CoopBattlePhaseWire::Fight { current_turn_owner, .. } => {
                let Some(spell_id) = self.coop_pending_spell.take() else {
                    return;
                };
                if *current_turn_owner != Some(local_peer) {
                    return;
                }
                coop.bind_mut().send_battle_action(&spell_id, target);
            }
            CoopBattlePhaseWire::End { .. } => {}
        }
    }

    /// Équivalent coop de `refresh_ui` pour la partie solo : construit le
    /// titre et, si c'est le tour d'un des personnages du joueur local, les
    /// boutons de sort — à partir de `coop_last` plutôt que de `self.phase`.
    fn refresh_coop_ui(&mut self) {
        let Some(wire) = self.coop_last.clone() else {
            self.set_title("En attente du combat...");
            return;
        };
        let local_peer = self.coop_session().map(|coop| coop.bind().local_peer_id()).unwrap_or(-1);

        match &wire.phase {
            CoopBattlePhaseWire::Placement { pending_counts, .. } => {
                let mine = pending_counts.iter().find(|(peer, _)| *peer == local_peer).map(|(_, n)| *n).unwrap_or(0);
                let text = if mine > 0 {
                    format!("Placement — clique une case de ton plateau ({mine} restant(s))")
                } else {
                    "Placement — en attente des autres joueurs...".to_string()
                };
                self.set_title(&text);
            }
            CoopBattlePhaseWire::Fight { current_turn_owner, current_unit_name, current_unit_spell_ids } => {
                if *current_turn_owner == Some(local_peer) {
                    self.set_title(&format!("À toi de jouer : {current_unit_name}"));
                    self.build_spell_buttons(current_unit_spell_ids.clone());
                } else if current_turn_owner.is_some() {
                    self.set_title(&format!("Tour de {current_unit_name} (coéquipier)"));
                } else {
                    self.set_title(&format!("Tour de {current_unit_name} (ennemi)"));
                }
            }
            CoopBattlePhaseWire::End { .. } => {
                self.set_title("Fin du combat...");
            }
        }
    }
}
