-- Sauvegarde unique (pas de comptes/multi-joueurs pour l'instant, specs
-- section 9) : une seule ligne, id fixé à 1.
CREATE TABLE saves (
    id INTEGER PRIMARY KEY,
    xp INTEGER NOT NULL DEFAULT 0,
    gold INTEGER NOT NULL DEFAULT 0,
    pos_x INTEGER NOT NULL DEFAULT 0,
    pos_y INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE inventory_items (
    id SERIAL PRIMARY KEY,
    save_id INTEGER NOT NULL REFERENCES saves(id) ON DELETE CASCADE,
    item_name TEXT NOT NULL
);

CREATE TABLE battle_history (
    id SERIAL PRIMARY KEY,
    save_id INTEGER NOT NULL REFERENCES saves(id) ON DELETE CASCADE,
    victory BOOLEAN NOT NULL,
    xp INTEGER NOT NULL,
    gold INTEGER NOT NULL,
    loot TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
