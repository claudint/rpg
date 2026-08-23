//! Écran de connexion/inscription (Phase 3, section 6) : premier écran
//! affiché au lancement (voir `project.godot`, `run/main_scene`). Tant
//! qu'aucun compte n'est authentifié, `session::auth_token()` est vide et
//! toute requête `/save` échoue côté backend — voir `persistence.rs`.

use godot::classes::http_client::Method;
use godot::classes::{Button, ColorRect, Control, HttpRequest, IControl, Label, LineEdit};
use godot::prelude::*;
use serde::{Deserialize, Serialize};

use crate::dev_console::DevConsole;
use crate::persistence::BASE_URL;
use crate::session;

#[derive(Serialize)]
struct AuthRequest<'a> {
    name: &'a str,
    password: &'a str,
}

#[derive(Deserialize)]
struct AuthResponse {
    token: String,
    player_id: i32,
}

#[derive(GodotClass)]
#[class(base=Control)]
pub struct LoginScene {
    base: Base<Control>,
    name_input: Option<Gd<LineEdit>>,
    password_input: Option<Gd<LineEdit>>,
    status_label: Option<Gd<Label>>,
    auth_http: Option<Gd<HttpRequest>>,
}

#[godot_api]
impl IControl for LoginScene {
    fn init(base: Base<Control>) -> Self {
        Self { base, name_input: None, password_input: None, status_label: None, auth_http: None }
    }

    fn ready(&mut self) {
        let viewport_size = self.base().get_viewport_rect().size;

        let mut background = ColorRect::new_alloc();
        background.set_size(viewport_size);
        background.set_color(Color::from_rgb(0.12, 0.12, 0.16));
        self.base_mut().add_child(&background);

        let mut title = Label::new_alloc();
        title.set_position(Vector2::new(60.0, 60.0));
        title.set_text("Connexion");
        self.base_mut().add_child(&title);

        let mut name_label = Label::new_alloc();
        name_label.set_position(Vector2::new(60.0, 110.0));
        name_label.set_text("Pseudo");
        self.base_mut().add_child(&name_label);

        let mut name_input = LineEdit::new_alloc();
        name_input.set_position(Vector2::new(60.0, 140.0));
        name_input.set_size(Vector2::new(280.0, 34.0));
        name_input.set_placeholder("Pseudo");
        self.base_mut().add_child(&name_input);

        let mut password_label = Label::new_alloc();
        password_label.set_position(Vector2::new(60.0, 190.0));
        password_label.set_text("Mot de passe");
        self.base_mut().add_child(&password_label);

        let mut password_input = LineEdit::new_alloc();
        password_input.set_position(Vector2::new(60.0, 220.0));
        password_input.set_size(Vector2::new(280.0, 34.0));
        password_input.set_placeholder("Mot de passe");
        password_input.set_secret(true);
        self.base_mut().add_child(&password_input);

        let this = self.to_gd();

        let mut login_button = Button::new_alloc();
        login_button.set_position(Vector2::new(60.0, 270.0));
        login_button.set_size(Vector2::new(130.0, 40.0));
        login_button.set_text("Se connecter");
        login_button.connect("pressed", &Callable::from_object_method(&this, "on_login_pressed"));
        self.base_mut().add_child(&login_button);

        let mut register_button = Button::new_alloc();
        register_button.set_position(Vector2::new(210.0, 270.0));
        register_button.set_size(Vector2::new(160.0, 40.0));
        register_button.set_text("Créer un compte");
        register_button.connect("pressed", &Callable::from_object_method(&this, "on_register_pressed"));
        self.base_mut().add_child(&register_button);

        let mut status_label = Label::new_alloc();
        status_label.set_position(Vector2::new(60.0, 330.0));
        status_label.set_modulate(Color::from_rgb(1.0, 0.6, 0.6));
        self.base_mut().add_child(&status_label);

        let mut auth_http = HttpRequest::new_alloc();
        let mut response_target = self.to_gd();
        let callable = Callable::from_fn("on_auth_response", move |args: &[&Variant]| {
            let response_code = args[1].to::<i64>();
            let body = args[3].to::<PackedByteArray>();
            response_target.bind_mut().on_auth_response(response_code, body);
            Variant::nil()
        });
        auth_http.connect("request_completed", &callable);
        self.base_mut().add_child(&auth_http);

        self.name_input = Some(name_input);
        self.password_input = Some(password_input);
        self.status_label = Some(status_label);
        self.auth_http = Some(auth_http);
    }
}

#[godot_api]
impl LoginScene {
    #[func]
    fn on_login_pressed(&mut self) {
        self.submit("/login");
    }

    #[func]
    fn on_register_pressed(&mut self) {
        self.submit("/register");
    }

    /// Lance `POST {path}` (`/login` ou `/register`, même forme de requête
    /// et de réponse côté backend) avec le contenu des deux champs.
    fn submit(&mut self, path: &str) {
        let name = self.name_input.as_ref().map(|input| input.get_text().to_string()).unwrap_or_default();
        let password = self.password_input.as_ref().map(|input| input.get_text().to_string()).unwrap_or_default();
        if name.trim().is_empty() || password.is_empty() {
            self.set_status("Pseudo et mot de passe requis.");
            return;
        }

        let payload = AuthRequest { name: &name, password: &password };
        let Ok(body) = serde_json::to_string(&payload) else {
            return;
        };

        let Some(mut http) = self.auth_http.clone() else {
            return;
        };
        let mut headers = PackedStringArray::new();
        headers.push("Content-Type: application/json");

        let url = format!("{BASE_URL}{path}");
        let _ = http.request_ex(&url).method(Method::POST).custom_headers(&headers).request_data(&body).done();
        self.set_status("Connexion en cours...");
    }

    /// Réponse à `/login` ou `/register`. Sur succès : mémorise le jeton
    /// dans `session.rs`, déclenche le chargement de la sauvegarde via
    /// `DevConsole` (seul noeud qui porte les requêtes `save`/`load`, voir
    /// `dev_console.rs`), puis bascule sur la carte du monde.
    fn on_auth_response(&mut self, response_code: i64, body: PackedByteArray) {
        if !(200..300).contains(&response_code) {
            self.set_status("Identifiants invalides ou pseudo déjà pris.");
            return;
        }

        let text = body.get_string_from_utf8().to_string();
        let Ok(response) = serde_json::from_str::<AuthResponse>(&text) else {
            self.set_status("Réponse du serveur invalide.");
            return;
        };

        session::set_auth(response.token, response.player_id);

        if let Some(root) = self.base().get_tree().get_root() {
            if let Some(node) = root.get_node_or_null("DevConsole") {
                if let Ok(console) = node.try_cast::<DevConsole>() {
                    console.bind().load();
                }
            }
        }

        self.base().get_tree().change_scene_to_file("res://scenes/world.tscn");
    }

    fn set_status(&mut self, text: &str) {
        if let Some(label) = self.status_label.as_mut() {
            label.set_text(text);
        }
    }
}
