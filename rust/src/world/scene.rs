//! Noeud Godot qui affiche la grille d'exploration et déplace le joueur.
//!
//! Toute la logique de déplacement (calcul de case d'arrivée, chemin) vit
//! dans `world::grid`, qui ne connaît rien de Godot. Ce fichier ne fait que
//! lire les entrées, appeler cette logique, et dessiner le résultat.

use std::collections::VecDeque;

use godot::classes::{InputEvent, InputEventKey, InputEventMouseButton};
use godot::global::{randf, Key, MouseButton};
use godot::prelude::*;

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
        }
    }

    fn process(&mut self, delta: f64) {
        self.advance_visual_position(delta as f32);
        self.base_mut().queue_redraw();
    }

    fn unhandled_input(&mut self, event: Gd<InputEvent>) {
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
        crate::session::set_return_point(TOWN_POS);
        self.base().get_tree().change_scene_to_file("res://scenes/town.tscn");
    }

    fn enter_battle(&mut self, pos: GridPos) {
        crate::session::set_return_point(pos);
        self.base().get_tree().change_scene_to_file("res://scenes/battle.tscn");
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
