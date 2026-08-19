//! Noeud Godot qui affiche la grille d'exploration et déplace le joueur.
//!
//! Toute la logique de déplacement (calcul de case d'arrivée, chemin) vit
//! dans `world::grid`, qui ne connaît rien de Godot. Ce fichier ne fait que
//! lire les entrées, appeler cette logique, et dessiner le résultat.

use std::collections::VecDeque;

use godot::classes::{
    Button, ColorRect, HSeparator, InputEvent, InputEventKey, InputEventMouseButton, Label, ScrollContainer,
    VBoxContainer,
};
use godot::global::{randf, Key, MouseButton};
use godot::prelude::*;

use crate::session;

use super::encounter;
use super::grid::{self, Direction, GridBounds, GridPos};

const TILE_SIZE: f32 = 64.0;
/// Vitesse de déplacement du joueur, en cases par seconde.
const MOVE_SPEED: f32 = 8.0;
/// Case où se trouve le point d'intérêt "ville" (Phase 1 étape 2).
const TOWN_POS: GridPos = GridPos { x: 6, y: 3 };
/// Probabilité de déclencher un combat à chaque case parcourue (Phase 1 étape 3).
const ENCOUNTER_CHANCE: f64 = 0.05;

fn grid_to_pixels(pos: GridPos) -> Vector2 {
    Vector2::new(pos.x as f32 * TILE_SIZE, pos.y as f32 * TILE_SIZE)
}

#[derive(GodotClass)]
#[class(base=Node2D)]
pub struct WorldScene {
    base: Base<Node2D>,
    bounds: GridBounds,
    /// Case actuelle du joueur (état logique, la seule source de vérité).
    logical_pos: GridPos,
    /// Position affichée à l'écran, qui glisse progressivement vers
    /// `logical_pos` pour donner l'impression d'un mouvement fluide.
    visual_pos: Vector2,
    /// Cases restant à parcourir pour un déplacement à la souris.
    move_queue: VecDeque<GridPos>,
    /// Vrai pendant qu'un popup (ex. récompense de victoire) est affiché :
    /// on ignore alors les entrées de déplacement.
    has_open_popup: bool,
}

#[godot_api]
impl INode2D for WorldScene {
    fn init(base: Base<Node2D>) -> Self {
        let bounds = GridBounds { width: 10, height: 8 };
        // Au premier lancement, ça vaut (0, 0) (valeur par défaut de
        // `session::return_point`). En revenant d'un écran secondaire (ville,
        // combat...), ça replace le joueur sur la case qui l'y a envoyé.
        let start = crate::session::return_point();

        Self {
            base,
            bounds,
            logical_pos: start,
            visual_pos: grid_to_pixels(start),
            move_queue: VecDeque::new(),
            has_open_popup: false,
        }
    }

    fn ready(&mut self) {
        // Les boutons de menu doivent être ajoutés avant un éventuel popup
        // de récompense initial, pour que son fond plein écran (ajouté après
        // et donc par-dessus) les bloque bien tant qu'il est affiché.
        self.add_menu_button(Vector2::new(660.0, 20.0), "Historique", WorldScene::open_history_popup);
        self.add_menu_button(Vector2::new(660.0, 70.0), "Inventaire", WorldScene::open_inventory_popup);
        self.add_menu_button(Vector2::new(660.0, 120.0), "Équipe", WorldScene::open_team_popup);
        self.add_menu_button(Vector2::new(660.0, 170.0), "Statistiques", WorldScene::open_stats_popup);

        if let Some(reward) = session::take_pending_reward() {
            self.show_reward_popup(reward);
        }
    }

    fn process(&mut self, delta: f64) {
        self.advance_visual_position(delta as f32);
        self.base_mut().queue_redraw();
    }

    fn unhandled_input(&mut self, event: Gd<InputEvent>) {
        if self.has_open_popup {
            return;
        }

        if let Some(dir) = keyboard_direction(&event) {
            self.try_step(dir);
            return;
        }

        if let Some(target) = self.mouse_click_target(&event) {
            self.queue_path_to(target);
        }
    }

    fn draw(&mut self) {
        self.draw_grid();
        self.draw_player();
    }
}

impl WorldScene {
    /// Vrai tant que le joueur glisse encore vers sa case logique : on
    /// ignore les nouvelles entrées clavier pendant ce temps pour garder un
    /// déplacement case par case bien lisible (façon Pokémon).
    fn is_moving(&self) -> bool {
        self.visual_pos.distance_to(grid_to_pixels(self.logical_pos)) > 0.5
            || !self.move_queue.is_empty()
    }

    fn try_step(&mut self, dir: Direction) {
        if self.is_moving() {
            return;
        }
        if let Some(target) = grid::step(self.logical_pos, dir, self.bounds) {
            self.arrive_at(target);
        }
    }

    fn queue_path_to(&mut self, target: GridPos) {
        if self.is_moving() || !self.bounds.contains(target) {
            return;
        }
        for step in grid::path_to(self.logical_pos, target) {
            self.move_queue.push_back(step);
        }
    }

    fn advance_visual_position(&mut self, delta: f32) {
        let target = grid_to_pixels(self.logical_pos);
        let max_dist = MOVE_SPEED * TILE_SIZE * delta;

        if self.visual_pos.distance_to(target) <= max_dist {
            self.visual_pos = target;
            if let Some(next) = self.move_queue.pop_front() {
                self.arrive_at(next);
            }
        } else {
            let direction = (target - self.visual_pos).normalized();
            self.visual_pos += direction * max_dist;
        }
    }

    /// Met à jour la case logique du joueur et déclenche l'entrée dans un
    /// point d'intérêt si la case d'arrivée en contient un.
    fn arrive_at(&mut self, pos: GridPos) {
        self.logical_pos = pos;
        if pos == TOWN_POS {
            self.enter_town();
        } else if encounter::should_trigger(randf(), ENCOUNTER_CHANCE) {
            self.enter_battle(pos);
        }
    }

    fn enter_town(&mut self) {
        session::set_return_point(TOWN_POS);
        self.base().get_tree().change_scene_to_file("res://scenes/town.tscn");
    }

    fn enter_battle(&mut self, pos: GridPos) {
        session::set_return_point(pos);
        self.base().get_tree().change_scene_to_file("res://scenes/battle.tscn");
    }

    /// Popup de récompense après un combat gagné (specs, section 6 étape 5).
    /// `backdrop` couvre tout l'écran et absorbe les clics (mouse_filter par
    /// défaut = Stop) pour empêcher de bouger le joueur pendant qu'il est
    /// affiché ; `has_open_popup` fait pareil côté clavier.
    fn show_reward_popup(&mut self, reward: session::PendingReward) {
        let viewport_size = self.base().get_viewport_rect().size;

        let mut backdrop = ColorRect::new_alloc();
        backdrop.set_size(viewport_size);
        backdrop.set_color(Color::from_rgba(0.0, 0.0, 0.0, 0.55));

        let mut panel = ColorRect::new_alloc();
        panel.set_position(Vector2::new(120.0, 120.0));
        panel.set_size(Vector2::new(420.0, 320.0));
        panel.set_color(Color::from_rgb(0.15, 0.15, 0.2));
        backdrop.add_child(&panel);

        let progress = session::progress();
        let mut label = Label::new_alloc();
        label.set_position(Vector2::new(140.0, 140.0));
        label.set_size(Vector2::new(380.0, 220.0));
        label.set_text(&format!(
            "Victoire !\n\n+{} XP\n+{} or\nButin : {}\n\nTotal : {} XP, {} or",
            reward.xp, reward.gold, reward.loot, progress.xp, progress.gold
        ));
        backdrop.add_child(&label);

        let mut ok_button = Button::new_alloc();
        ok_button.set_position(Vector2::new(140.0, 370.0));
        ok_button.set_size(Vector2::new(100.0, 40.0));
        ok_button.set_text("OK");
        backdrop.add_child(&ok_button);

        self.base_mut().add_child(&backdrop);
        self.has_open_popup = true;

        let mut this = self.to_gd();
        let mut backdrop_handle = backdrop.clone();
        let callable = Callable::from_fn("close_reward_popup", move |_args: &[&Variant]| {
            backdrop_handle.queue_free();
            this.bind_mut().has_open_popup = false;
            Variant::nil()
        });
        ok_button.connect("pressed", &callable);
    }

    /// Bouton de menu toujours visible sur la carte (Historique, Inventaire,
    /// Équipe, Statistiques). `action` est une méthode de `WorldScene` sans
    /// argument, passée comme simple pointeur de fonction (elle ne capture
    /// rien, donc pas besoin de fermeture ici).
    fn add_menu_button(&mut self, position: Vector2, text: &str, action: fn(&mut WorldScene)) {
        let mut button = Button::new_alloc();
        button.set_position(position);
        button.set_size(Vector2::new(160.0, 40.0));
        button.set_text(text);

        let mut target = self.to_gd();
        let callable = Callable::from_fn("menu_button", move |_args: &[&Variant]| {
            action(&mut target.bind_mut());
            Variant::nil()
        });
        button.connect("pressed", &callable);

        self.base_mut().add_child(&button);
    }

    fn open_history_popup(&mut self) {
        let entries: Vec<String> = session::battle_history()
            .iter()
            .enumerate()
            .map(|(i, record)| {
                let result_text = match record.result {
                    session::BattleResult::Victory => "Victoire",
                    session::BattleResult::Defeat => "Défaite",
                };
                match record.loot {
                    Some(loot) => format!(
                        "Combat {} — {}\n+{} XP, +{} or\nButin : {}",
                        i + 1,
                        result_text,
                        record.xp,
                        record.gold,
                        loot
                    ),
                    None => format!("Combat {} — {}", i + 1, result_text),
                }
            })
            .collect();
        self.open_list_popup("Historique des combats", entries, true);
    }

    fn open_inventory_popup(&mut self) {
        let entries: Vec<String> = session::inventory().into_iter().map(|item| item.to_string()).collect();
        self.open_list_popup("Inventaire", entries, false);
    }

    fn open_team_popup(&mut self) {
        self.open_list_popup("Équipe", crate::battle::team_summary(), true);
    }

    fn open_stats_popup(&mut self) {
        let progress = session::progress();
        let history = session::battle_history();
        let wins = history.iter().filter(|r| r.result == session::BattleResult::Victory).count();
        let losses = history.iter().filter(|r| r.result == session::BattleResult::Defeat).count();
        let entries = vec![format!(
            "XP total : {}\nOr total : {}\nCombats gagnés : {}\nCombats perdus : {}",
            progress.xp, progress.gold, wins, losses
        )];
        self.open_list_popup("Statistiques", entries, false);
    }

    /// Popup générique listant `entries` dans un conteneur défilant (specs
    /// implicites de la demande : scrollbar si beaucoup d'entrées, séparées
    /// par une ligne si `separated`).
    fn open_list_popup(&mut self, title: &str, entries: Vec<String>, separated: bool) {
        let viewport_size = self.base().get_viewport_rect().size;

        let mut backdrop = ColorRect::new_alloc();
        backdrop.set_size(viewport_size);
        backdrop.set_color(Color::from_rgba(0.0, 0.0, 0.0, 0.55));

        let mut panel = ColorRect::new_alloc();
        panel.set_position(Vector2::new(100.0, 60.0));
        panel.set_size(Vector2::new(460.0, 440.0));
        panel.set_color(Color::from_rgb(0.15, 0.15, 0.2));
        backdrop.add_child(&panel);

        let mut title_label = Label::new_alloc();
        title_label.set_position(Vector2::new(120.0, 76.0));
        title_label.set_text(title);
        backdrop.add_child(&title_label);

        let mut scroll = ScrollContainer::new_alloc();
        scroll.set_position(Vector2::new(120.0, 112.0));
        scroll.set_size(Vector2::new(420.0, 320.0));
        backdrop.add_child(&scroll);

        let mut list = VBoxContainer::new_alloc();
        list.set_custom_minimum_size(Vector2::new(400.0, 0.0));
        scroll.add_child(&list);

        if entries.is_empty() {
            let mut empty_label = Label::new_alloc();
            empty_label.set_text("(rien pour l'instant)");
            list.add_child(&empty_label);
        } else {
            for (i, entry) in entries.iter().enumerate() {
                if separated && i > 0 {
                    let separator = HSeparator::new_alloc();
                    list.add_child(&separator);
                }
                let mut entry_label = Label::new_alloc();
                entry_label.set_text(entry);
                list.add_child(&entry_label);
            }
        }

        let mut close_button = Button::new_alloc();
        close_button.set_position(Vector2::new(120.0, 452.0));
        close_button.set_size(Vector2::new(100.0, 36.0));
        close_button.set_text("Fermer");
        backdrop.add_child(&close_button);

        self.base_mut().add_child(&backdrop);
        self.has_open_popup = true;

        let mut this = self.to_gd();
        let mut backdrop_handle = backdrop.clone();
        let callable = Callable::from_fn("close_list_popup", move |_args: &[&Variant]| {
            backdrop_handle.queue_free();
            this.bind_mut().has_open_popup = false;
            Variant::nil()
        });
        close_button.connect("pressed", &callable);
    }

    fn mouse_click_target(&self, event: &Gd<InputEvent>) -> Option<GridPos> {
        let mouse_event = event.clone().try_cast::<InputEventMouseButton>().ok()?;
        if mouse_event.get_button_index() != MouseButton::LEFT || !mouse_event.is_pressed() {
            return None;
        }

        let local = self.base().to_local(mouse_event.get_position());
        let target = GridPos::new(
            (local.x / TILE_SIZE).floor() as i32,
            (local.y / TILE_SIZE).floor() as i32,
        );
        self.bounds.contains(target).then_some(target)
    }

    fn draw_grid(&mut self) {
        let line_color = Color::from_rgb(0.25, 0.25, 0.3);
        let town_color = Color::from_rgb(0.55, 0.4, 0.2);

        for x in 0..self.bounds.width {
            for y in 0..self.bounds.height {
                let rect = Rect2::new(
                    Vector2::new(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE),
                    Vector2::new(TILE_SIZE, TILE_SIZE),
                );
                if GridPos::new(x, y) == TOWN_POS {
                    self.base_mut().draw_rect(rect, town_color);
                }
                self.base_mut()
                    .draw_rect_ex(rect, line_color)
                    .filled(false)
                    .width(1.0)
                    .done();
            }
        }
    }

    fn draw_player(&mut self) {
        let margin = 8.0;
        let rect = Rect2::new(
            self.visual_pos + Vector2::new(margin, margin),
            Vector2::new(TILE_SIZE - margin * 2.0, TILE_SIZE - margin * 2.0),
        );
        self.base_mut().draw_rect(rect, Color::from_rgb(0.3, 0.7, 1.0));
    }
}

fn keyboard_direction(event: &Gd<InputEvent>) -> Option<Direction> {
    let key_event = event.clone().try_cast::<InputEventKey>().ok()?;
    if !key_event.is_pressed() || key_event.is_echo() {
        return None;
    }

    match key_event.get_keycode() {
        Key::UP | Key::W => Some(Direction::Up),
        Key::DOWN | Key::S => Some(Direction::Down),
        Key::LEFT | Key::A => Some(Direction::Left),
        Key::RIGHT | Key::D => Some(Direction::Right),
        _ => None,
    }
}
