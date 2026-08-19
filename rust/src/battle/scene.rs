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

use crate::geometry::GridPos;

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

fn opposite(side: Side) -> Side {
    match side {
        Side::Player => Side::Enemy,
        Side::Enemy => Side::Player,
    }
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
    End(Outcome),
}

#[derive(GodotClass)]
#[class(base=Control)]
pub struct BattleScene {
    base: Base<Control>,
    phase: Phase,
    title: Option<Gd<Label>>,
    ui_container: Option<Gd<Control>>,
}

#[godot_api]
impl IControl for BattleScene {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            phase: Phase::Placement { pending: data::default_player_roster(), placed: Vec::new() },
            title: None,
            ui_container: None,
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

        let this = self.to_gd();
        let mut flee_button = Button::new_alloc();
        flee_button.set_position(Vector2::new(600.0, 20.0));
        flee_button.set_size(Vector2::new(120.0, 40.0));
        flee_button.set_text("Fuir");
        flee_button.connect("pressed", &Callable::from_object_method(&this, "on_flee_pressed"));
        self.base_mut().add_child(&flee_button);

        self.refresh_ui();
    }

    fn process(&mut self, _delta: f64) {
        self.base_mut().queue_redraw();
    }

    fn draw(&mut self) {
        self.draw_board(Side::Player);
        self.draw_board(Side::Enemy);
        self.draw_units();
        self.draw_hover_preview();
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
                TargetKind::Enemy => opposite(caster.side),
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
            let caster_id = battle.current_unit_id();
            let caster = battle.unit(caster_id).clone();

            caster.spell_ids.first().cloned().and_then(|spell_id| {
                let spell = battle.spell(&spell_id)?.clone();
                let target_side = match spell.target {
                    TargetKind::Enemy => opposite(caster.side),
                    TargetKind::Ally => caster.side,
                };
                let candidates: Vec<GridPos> = battle
                    .units
                    .iter()
                    .filter(|u| u.is_alive() && u.pos.side == target_side)
                    .map(|u| u.pos.cell)
                    .collect();
                if candidates.is_empty() {
                    return None;
                }
                let index = ((randf() * candidates.len() as f64) as usize).min(candidates.len() - 1);
                Some((spell_id, BoardPos { side: target_side, cell: candidates[index] }))
            })
        };

        match action {
            Some((spell_id, target)) => self.resolve_and_advance(&spell_id, target),
            None => {
                self.advance_turn_only();
                self.update_unit_labels();
            }
        }
    }

    fn enter_end_state(&mut self, outcome: Outcome) {
        self.phase = Phase::End(outcome);
        self.refresh_ui();
        self.base_mut().queue_redraw();
    }

    fn select_spell(&mut self, spell_id: String) {
        if let Phase::Fight { pending_spell, .. } = &mut self.phase {
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

    fn build_spell_buttons(&mut self, spell_ids: Vec<String>) {
        let spell_names: Vec<(String, String)> = match &self.phase {
            Phase::Fight { battle, .. } => spell_ids
                .iter()
                .filter_map(|id| battle.spell(id).map(|s| (id.clone(), s.name.clone())))
                .collect(),
            _ => Vec::new(),
        };

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
}
