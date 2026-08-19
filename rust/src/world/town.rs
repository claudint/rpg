//! Ecran d'un point d'intérêt "ville". Pour l'instant, un menu minimal sans
//! fonctionnalité derrière (juste "Boutique" et "Sortir") : la fonction
//! précise de chaque PDI reste à définir (specs, section 9).

use godot::classes::{Button, ColorRect, Control, IControl, Label};
use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Control)]
pub struct TownScene {
    base: Base<Control>,
    status_label: Option<Gd<Label>>,
}

#[godot_api]
impl IControl for TownScene {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            status_label: None,
        }
    }

    fn ready(&mut self) {
        let viewport_size = self.base().get_viewport_rect().size;

        let mut background = ColorRect::new_alloc();
        background.set_size(viewport_size);
        background.set_color(Color::from_rgb(0.3, 0.5, 0.35));
        self.base_mut().add_child(&background);

        let mut title = Label::new_alloc();
        title.set_position(Vector2::new(40.0, 40.0));
        title.set_text("Ville");
        self.base_mut().add_child(&title);

        // Callables ciblent des méthodes #[func] déclarées plus bas, sur
        // `self` : il faut donc un Gd<Self> (via `to_gd`) pour les créer.
        let this = self.to_gd();

        let mut shop_button = Button::new_alloc();
        shop_button.set_position(Vector2::new(40.0, 120.0));
        shop_button.set_size(Vector2::new(200.0, 50.0));
        shop_button.set_text("Boutique");
        shop_button.connect("pressed", &Callable::from_object_method(&this, "on_shop_pressed"));
        self.base_mut().add_child(&shop_button);

        let mut leave_button = Button::new_alloc();
        leave_button.set_position(Vector2::new(40.0, 190.0));
        leave_button.set_size(Vector2::new(200.0, 50.0));
        leave_button.set_text("Sortir");
        leave_button.connect("pressed", &Callable::from_object_method(&this, "on_leave_pressed"));
        self.base_mut().add_child(&leave_button);

        let mut status_label = Label::new_alloc();
        status_label.set_position(Vector2::new(40.0, 260.0));
        self.base_mut().add_child(&status_label);
        self.status_label = Some(status_label);
    }
}

#[godot_api]
impl TownScene {
    #[func]
    fn on_shop_pressed(&mut self) {
        if let Some(label) = self.status_label.as_mut() {
            label.set_text("La boutique n'est pas encore implémentée.");
        }
    }

    #[func]
    fn on_leave_pressed(&mut self) {
        self.base().get_tree().change_scene_to_file("res://scenes/world.tscn");
    }
}
