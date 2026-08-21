# État d'avancement — RPG tactique

Dernière mise à jour : 2026-08-21. Ce fichier résume où en est le projet
pour reprendre le travail dans une nouvelle conversation. Les specs
complètes restent dans `specs-jeu-rpg-tactique.md`, les conventions dans
`CLAUDE.md` — les deux à lire avant toute modification importante.

Tout ce qui est décrit ici est **poussé sur `origin/main`** (dernier commit :
`d4a8efd`). Rien en attente localement.

## Ce qui est fait

### Phase 1 — MVP solo (specs section 6, étapes 1 à 5) : complète et validée

1. **Déplacement** sur la carte du monde en grille (clavier + souris),
   logique pure dans `rust/src/world/grid.rs`, affichage dans
   `rust/src/world/scene.rs`.
2. **Point d'intérêt "ville"** basique, écran dédié
   (`rust/src/world/town.rs`).
3. **Déclenchement de combat aléatoire** en marchant (`ENCOUNTER_CHANCE` à
   0.05 dans `world/scene.rs`, ajustable).
4. **Plateau de combat tactique 3x4 par camp** : moteur pur dans
   `rust/src/battle/engine.rs` (ordre de tour par vitesse, résolution de
   dégâts), `pattern.rs` (sorts en zone : Single/Cross/Square), `data.rs`
   (personnages/monstres/sorts en JSON, `rust/data/*.json`). Affichage dans
   `rust/src/battle/scene.rs` : placement, ciblage avec prévisualisation,
   IA ennemie simple. **Particularité notable** : les deux plateaux (joueur
   et ennemi) sont traités comme physiquement accolés — un sort de zone
   ancré près de la frontière peut toucher les deux camps, y compris le
   lanceur (demande explicite de l'utilisateur, voir `engine::to_combined`).
5. **Résolution de victoire** : XP/or/butin calculés, retour immédiat sur
   la carte avec un popup récapitulatif (pas d'écran de fin dans le combat
   lui-même, changé sur retour utilisateur). Chaque combat est enregistré
   dans un historique.

**Au-delà du strict MVP, ajouté à la demande de l'utilisateur :**
- 5 boutons sur la carte du monde : Historique (liste défilante, combats
  séparés par une ligne), Inventaire, Équipe, Statistiques, Sauvegarder —
  tous en popup modal générique (`WorldScene::open_list_popup`).
- **Console de dev façon Source engine** (`rust/src/dev_console.rs`),
  touche **F1**. Autoload Godot (`DevConsole` dans `project.godot`, scène
  `rpg/scenes/dev_console.tscn`) : seul noeud qui survit aux changements de
  scène. Commandes : `help`, `give_xp <n>`, `give_gold <n>`,
  `teleport <x> <y>`, `start_battle`, `heal_team`, `save`, `load`.

### Phase 2 — Persistance (specs section 6 étape 6) : complète et validée

Architecture cible des specs (section 4-5) mise en place directement,
**pas** de fichier de sauvegarde local — l'utilisateur avait déjà un
Postgres 15 et voulait éviter une réécriture au moment du multijoueur :

- **`backend-api/`** : nouveau crate Rust séparé (axum + sqlx/Postgres),
  lancé indépendamment du jeu (`cargo run` dans `backend-api/`). Écoute sur
  `http://127.0.0.1:8080`. Deux routes : `GET /save` (état complet, valeurs
  à zéro si rien sauvegardé) et `PUT /save` (remplace tout, upsert en
  transaction). Connexion via `DATABASE_URL` dans `backend-api/.env`
  (jamais commité, voir `.env.example` pour le format).
- **Schéma** (`backend-api/migrations/0001` à `0004`) : `saves` (une ligne,
  id=1) → FK vers `players` (prépare le multijoueur) ; `inventory_items`
  référence `items` par id plutôt que du texte libre (évite les doublons) ;
  `battle_history` ; `characters` + `player_characters` (colonne
  `selected`, prépare le choix de composition d'équipe — **schéma
  seulement, rien de branché côté jeu pour l'instant**).
- **Côté jeu** : `rust/src/persistence.rs` (construit/lit le JSON), deux
  noeuds `HTTPRequest` portés par `DevConsole` (`save_http`/`load_http`).
  Chargement au boot (avec correction de position si la carte est déjà
  affichée quand la réponse arrive — le chargement est asynchrone,
  contrairement à un fichier local). Sauvegarde automatique après une
  victoire, bouton "Sauvegarder", commandes console `save`/`load`.

## Pour relancer le projet

1. **Backend** : `cd backend-api && cargo run` (nécessite
   `backend-api/.env` avec `DATABASE_URL=postgres://...` déjà en place —
   pas besoin de le recréer, il existe mais n'est pas versionné). Vérifier
   qu'il tourne avec `curl http://127.0.0.1:8080/save` avant de tester le
   jeu — **le process meurt parfois tout seul entre deux sessions, sans
   crash visible**, donc ne jamais supposer qu'un lancement précédent
   tourne encore.
2. **Jeu** : fermer toute fenêtre Godot ouverte avant `cargo build` dans
   `rust/` (sinon `rpg_rust.dll` est verrouillée, erreur "Accès refusé").
   Lancer avec `D:\logiciel\godot\Godot_v4.7.2-stable_win64.exe --path
   D:\dev\rpg\rpg`, ou en headless avec `--headless --quit-after N` pour
   un test de fumée rapide sans interaction.

## Pièges déjà rencontrés (évite de les refaire)

- Un `Control` absorbe les clics par défaut (`mouse_filter = Stop`) :
  penser à le passer en `IGNORE` sur tout ce qui ne doit pas intercepter
  les clics destinés à un plateau/une grille en dessous.
- Un enfant `CanvasItem` se dessine toujours par-dessus le `_draw()` de son
  parent, quel que soit l'ordre d'ajout : un fond plein écran ajouté en
  enfant cache le contenu dessiné par le parent, sauf à lui donner un
  `z_index` négatif.
- Un écran `Control` sans `process()` qui appelle `queue_redraw()` chaque
  frame ne se redessine pas de façon fiable — préférer ce pattern
  systématiquement plutôt que des `queue_redraw()` ponctuels.
- **Jamais** de PowerShell `Get-Content`/`Set-Content` pour des remplacements
  en masse dans les fichiers du repo : ça corrompt l'UTF-8 des accents
  français, même avec `-Encoding utf8`. Utiliser l'outil d'édition dédié.

## Pas encore fait / pistes pour la suite

- **Phase 3-4 des specs (multijoueur, PvP)** : rien commencé. Le schéma
  Postgres est préparé (`players`) mais aucun compte/authentification.
- **Sélection de composition d'équipe** : `player_characters.selected`
  existe en base mais aucune UI pour la modifier (l'équipe reste la liste
  par défaut codée dans `rust/data/characters.json`).
- **Pas de vraie montée de niveau des personnages** : chaque combat repart
  des stats de base, seuls XP/or joueur et historique sont cumulatifs.
- **Défaite sans pénalité** : volontairement laissé tel quel (specs section
  9 flague ce point comme non défini).
- Points encore ouverts listés dans `specs-jeu-rpg-tactique.md` section 9
  (modèle des personnages, fonction précise des PDI, style visuel...).
