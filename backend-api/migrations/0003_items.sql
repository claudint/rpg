-- Catalogue des objets, pour éviter les doublons de noms en texte libre dans
-- inventory_items (un item = une ligne, référencé par id).
CREATE TABLE items (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

-- Reprend la table de butin actuelle du jeu (rust/src/battle/engine.rs,
-- LOOT_TABLE) ; put_save ajoute automatiquement tout nouvel objet inconnu.
INSERT INTO items (name) VALUES
    ('Potion de soin'),
    ('Petite bourse'),
    ('Fragment de minerai'),
    ('Vieille carte');

ALTER TABLE inventory_items ADD COLUMN item_id INTEGER REFERENCES items(id);
UPDATE inventory_items SET item_id = items.id FROM items WHERE inventory_items.item_name = items.name;
ALTER TABLE inventory_items ALTER COLUMN item_id SET NOT NULL;
ALTER TABLE inventory_items DROP COLUMN item_name;
