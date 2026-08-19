mod data;
mod engine;
mod pattern;
mod scene;

/// Résumé de l'équipe par défaut, pour l'écran "Équipe" de la carte du monde
/// (`world::scene`). Pas de vraie gestion d'équipe pour l'instant (le modèle
/// de recrutement reste à définir, specs section 9).
pub fn team_summary() -> Vec<String> {
    data::default_player_roster()
        .into_iter()
        .map(|u| format!("{}\nPV {}, Vitesse {}\nSorts : {}", u.name, u.hp, u.speed, u.spell_ids.join(", ")))
        .collect()
}
