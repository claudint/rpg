//! Ecran de combat. Pour l'instant, juste la transition d'écran (specs,
//! Phase 1 étape 3) : le vrai moteur de combat (plateau 3x4, tour par tour)
//! arrivera à l'étape suivante, comme une crate de logique pure séparée de
//! cet affichage — même principe que `world::grid`.

use godot::classes::{Button, ColorRect, Control, IControl, Label};
use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Control)]
pub struct BattleScene {
    base: Base<Control>,
}

#[godot_api]
impl IControl for BattleScene {
    fn init(base: Base<Control>) -> Self {
        Self { base }
    }

    fn ready(&mut self) {
        let viewport_size = self.base().get_viewport_rect().size;

        let mut background = ColorRect::new_alloc();
        background.set_size(viewport_size);
        background.set_color(Color::from_rgb(0.5, 0.2, 0.2));
        self.base_mut().add_child(&background);

        let mut title = Label::new_alloc();
        title.set_position(Vector2::new(40.0, 40.0));
        title.set_text("Combat !");
        self.base_mut().add_child(&title);

        let this = self.to_gd();

        let mut flee_button = Button::new_alloc();
        flee_button.set_position(Vector2::new(40.0, 120.0));
        flee_button.set_size(Vector2::new(200.0, 50.0));
        flee_button.set_text("Fuir");
        flee_button.connect("pressed", &Callable::from_object_method(&this, "on_flee_pressed"));
        self.base_mut().add_child(&flee_button);
    }
}

#[godot_api]
impl BattleScene {
    #[func]
    fn on_flee_pressed(&mut self) {
        self.base().get_tree().change_scene_to_file("res://scenes/world.tscn");
    }
}
