-- Table des joueurs, en préparation du multijoueur (Phase 3-4). Une seule
-- entrée pour l'instant (le joueur courant), mais la sauvegarde y référence
-- déjà son propriétaire par FK plutôt que d'avoir un id figé en dur — pas
-- besoin de retoucher le schéma quand les comptes arriveront.
CREATE TABLE players (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO players (id, name) VALUES (1, 'Joueur 1');

-- Le prochain INSERT sans id explicite doit repartir après 1, pas le réutiliser.
SELECT setval('players_id_seq', (SELECT MAX(id) FROM players));

ALTER TABLE saves ADD COLUMN player_id INTEGER NOT NULL DEFAULT 1 REFERENCES players(id);
