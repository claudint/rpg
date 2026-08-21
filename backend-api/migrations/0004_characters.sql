-- Catalogue des personnages (reprend l'équipe par défaut du jeu, voir
-- rust/data/characters.json) et table de liaison joueur <-> personnage, en
-- préparation de la sélection de composition d'équipe (pas encore
-- implémentée côté jeu : juste le schéma pour l'instant).
CREATE TABLE characters (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    hp INTEGER NOT NULL,
    speed INTEGER NOT NULL
);

INSERT INTO characters (name, hp, speed) VALUES
    ('Chevalier', 30, 8),
    ('Archère', 20, 12),
    ('Mage', 18, 9);

-- "selected" : fait partie de la composition d'équipe active du joueur.
CREATE TABLE player_characters (
    id SERIAL PRIMARY KEY,
    player_id INTEGER NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    selected BOOLEAN NOT NULL DEFAULT true,
    UNIQUE (player_id, character_id)
);

-- Le joueur courant possède déjà les 3 personnages par défaut, tous sélectionnés.
INSERT INTO player_characters (player_id, character_id, selected)
SELECT 1, id, true FROM characters;
