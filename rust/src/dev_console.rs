//! Console de dev façon Source engine : F1 pour ouvrir/fermer, tape une
//! commande, Entrée pour l'exécuter. Autoload Godot (voir project.godot) :
//! contrairement aux popups des écrans de jeu, ce noeud n'est jamais détruit
//! quand la scène change, donc la console reste ouvrable partout.

use godot::classes::{
    CanvasLayer, ColorRect, Control, ICanvasLayer, InputEvent, InputEventKey, Label, LineEdit, ScrollContainer,
    VBoxContainer,
};
use godot::global::Key;
use godot::prelude::*;

use crate::battle::scene::BattleScene;
use crate::session;
use crate::world::scene::WorldScene;

#[derive(GodotClass)]
#[class(base=CanvasLayer)]
pub struct DevConsole {
    base: Base<CanvasLayer>,
    panel: Option<Gd<Control>>,
    input: Option<Gd<LineEdit>>,
    log: Option<Gd<VBoxContainer>>,
}

#[godot_api]
impl ICanvasLayer for DevConsole {
    fn init(base: Base<CanvasLayer>) -> Self {
        Self { base, panel: None, input: None, log: None }
    }

    fn ready(&mut self) {
        let mut panel = Control::new_alloc();
        panel.set_position(Vector2::new(20.0, 20.0));
        panel.set_size(Vector2::new(520.0, 320.0));
        panel.set_visible(false);

        let mut background = ColorRect::new_alloc();
        background.set_size(Vector2::new(520.0, 320.0));
        background.set_color(Color::from_rgba(0.05, 0.05, 0.08, 0.9));
        panel.add_child(&background);

        let mut scroll = ScrollContainer::new_alloc();
        scroll.set_position(Vector2::new(10.0, 10.0));
        scroll.set_size(Vector2::new(500.0, 260.0));
        panel.add_child(&scroll);

        let mut log = VBoxContainer::new_alloc();
        log.set_custom_minimum_size(Vector2::new(480.0, 0.0));
        scroll.add_child(&log);

        let mut input = LineEdit::new_alloc();
        input.set_position(Vector2::new(10.0, 280.0));
        input.set_size(Vector2::new(500.0, 30.0));
        input.set_placeholder("Commande... (help)");
        panel.add_child(&input);

        self.base_mut().add_child(&panel);

        let mut target = self.to_gd();
        let callable = Callable::from_fn("submit_console_command", move |args: &[&Variant]| {
            let text = args[0].to::<GString>().to_string();
            target.bind_mut().submit(text);
            Variant::nil()
        });
        input.connect("text_submitted", &callable);

        self.panel = Some(panel);
        self.input = Some(input);
        self.log = Some(log);

        self.print_line("Console de dev prête. Tape 'help' pour la liste des commandes.");
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        let Ok(key_event) = event.try_cast::<InputEventKey>() else {
            return;
        };
        if key_event.is_pressed() && !key_event.is_echo() && key_event.get_keycode() == Key::F1 {
            self.toggle();
        }
    }
}

impl DevConsole {
    fn toggle(&mut self) {
        let Some(panel) = &mut self.panel else {
            return;
        };
        let now_visible = !panel.is_visible();
        panel.set_visible(now_visible);

        if now_visible {
            if let Some(input) = &mut self.input {
                input.clear();
                input.grab_focus();
            }
        }

        let _ = self.with_world_scene(|world| {
            world.set_input_blocked(now_visible);
            Ok(())
        });
    }

    fn print_line(&mut self, text: &str) {
        let Some(log) = &mut self.log else {
            return;
        };
        let mut label = Label::new_alloc();
        label.set_text(text);
        log.add_child(&label);
    }

    fn submit(&mut self, text: String) {
        if let Some(input) = &mut self.input {
            input.clear();
        }

        let command_line = text.trim().to_string();
        if command_line.is_empty() {
            return;
        }

        self.print_line(&format!("> {command_line}"));
        match self.execute(&command_line) {
            Ok(message) => self.print_line(&message),
            Err(message) => self.print_line(&format!("Erreur : {message}")),
        }
    }

    fn execute(&self, command_line: &str) -> Result<String, String> {
        let mut parts = command_line.split_whitespace();
        let command = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        match command {
            "help" => Ok(
                "Commandes : help, give_xp <n>, give_gold <n>, teleport <x> <y>, start_battle, heal_team"
                    .to_string(),
            ),
            "give_xp" => {
                let amount = parse_i32(&args, 0)?;
                session::add_rewards(amount, 0);
                Ok(format!("+{amount} XP"))
            }
            "give_gold" => {
                let amount = parse_i32(&args, 0)?;
                session::add_rewards(0, amount);
                Ok(format!("+{amount} or"))
            }
            "teleport" => {
                let x = parse_i32(&args, 0)?;
                let y = parse_i32(&args, 1)?;
                self.with_world_scene(|world| world.teleport(x, y))?;
                Ok(format!("téléporté en ({x}, {y})"))
            }
            "start_battle" => {
                self.with_world_scene(|world| {
                    world.force_start_battle();
                    Ok(())
                })?;
                Ok("combat forcé".to_string())
            }
            "heal_team" => {
                self.with_battle_scene(|battle| battle.heal_player_team())?;
                Ok("équipe soignée".to_string())
            }
            "" => Ok(String::new()),
            other => Err(format!("commande inconnue : {other} (essaie 'help')")),
        }
    }

    fn current_scene(&self) -> Option<Gd<Node>> {
        self.base().get_tree().get_current_scene()
    }

    fn with_world_scene(&self, f: impl FnOnce(&mut WorldScene) -> Result<(), &'static str>) -> Result<(), String> {
        let Some(scene) = self.current_scene() else {
            return Err("pas de scène active".to_string());
        };
        let Ok(mut world) = scene.try_cast::<WorldScene>() else {
            return Err("disponible seulement sur la carte du monde".to_string());
        };
        f(&mut world.bind_mut()).map_err(|e| e.to_string())
    }

    fn with_battle_scene(&self, f: impl FnOnce(&mut BattleScene) -> Result<(), &'static str>) -> Result<(), String> {
        let Some(scene) = self.current_scene() else {
            return Err("pas de scène active".to_string());
        };
        let Ok(mut battle) = scene.try_cast::<BattleScene>() else {
            return Err("disponible seulement en combat".to_string());
        };
        f(&mut battle.bind_mut()).map_err(|e| e.to_string())
    }
}

fn parse_i32(args: &[&str], index: usize) -> Result<i32, String> {
    let raw = args.get(index).ok_or_else(|| "argument manquant".to_string())?;
    raw.parse::<i32>().map_err(|_| "argument invalide, attendu un nombre entier".to_string())
}
