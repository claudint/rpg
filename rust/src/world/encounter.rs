//! Décide si un déplacement déclenche une rencontre aléatoire. Le tirage
//! aléatoire lui-même reste côté Godot (RNG globale du moteur) ; ce module ne
//! fait que la comparaison, pour rester pur et testable.

pub fn should_trigger(roll: f64, chance: f64) -> bool {
    roll < chance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_below_chance() {
        assert!(should_trigger(0.1, 0.2));
    }

    #[test]
    fn does_not_trigger_above_chance() {
        assert!(!should_trigger(0.5, 0.2));
    }
}
