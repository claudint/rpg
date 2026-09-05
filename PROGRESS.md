# État d'avancement — RPG tactique

Dernière mise à jour : 2026-08-23. Ce fichier résume où en est le projet
pour reprendre le travail dans une nouvelle conversation. Les specs
complètes restent dans `specs-jeu-rpg-tactique.md`, les conventions dans
`CLAUDE.md` — les deux à lire avant toute modification importante.

Tout ce qui est décrit ici jusqu'à la synchro coop de la carte (Phase 3,
étape 7 incluse) est **poussé sur `origin/main`** (dernier commit :
`1cac652`). Le combat coop (Phase 3, étape 8, section suivante) est fait et
testé localement (deux clients + serveur headless, combat complet jusqu'à
la victoire), mais **pas encore commité** — à valider avec l'utilisateur
avant de committer/pousser.

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

### Phase 3 — Multijoueur, comptes joueurs (specs section 6 étape 7, début) : fait, pas commité

Première brique avant tout déplacement/combat synchronisé : le jeu sait
maintenant distinguer les joueurs (avant, tout était câblé en dur sur
`player_id = 1`). Décisions prises avec l'utilisateur : base Postgres locale
= données de test jetables (reset propre plutôt que migration de
l'existant), un seul écran gérant inscription et connexion, contexte
local/LAN uniquement pour l'instant (pas de TLS/durcissement). Plan complet
dans `C:\Users\maxen\.claude\plans\linear-soaring-bird.md` si besoin de
retrouver le détail des décisions.

- **`backend-api/migrations/0005_auth.sql`** : `players.password_hash`
  (argon2) + `players.name` unique, `saves.player_id` devient la clé
  d'upsert (unique + `saves.id` auto-généré), nouvelle table `sessions`
  (jeton opaque → player_id, pas d'expiration pour l'instant). Reset des
  tables `players`/`saves`/`inventory_items`/`battle_history`/
  `player_characters` (données de test jetables).
- **Routes** `POST /register` / `POST /login` (`backend-api/src/main.rs`) :
  mot de passe haché avec `argon2`, jeton de session en `Uuid`. `GET/PUT
  /save` exigent désormais un extracteur `AuthedPlayer` (en-tête
  `Authorization: Bearer <token>`) et sont paramétrés par joueur au lieu de
  l'id 1 en dur. Testé de bout en bout au `curl` (inscription, doublon de
  nom, mauvais mot de passe, isolation des sauvegardes entre deux comptes).
- **Écran de connexion** (`rust/src/login.rs`, nouvelle scène
  `rpg/scenes/login.tscn`, devenue `run/main_scene`) : pseudo + mot de
  passe (`LineEdit` masqué), boutons Se connecter/Créer un compte. Sur
  succès : jeton mémorisé dans `session.rs` (`AuthInfo`), déclenche
  `DevConsole::load()` (devenu `pub`, plus appelé automatiquement au boot
  comme avant), puis bascule sur `world.tscn`. `persistence.rs` ajoute
  l'en-tête `Authorization` sur les requêtes save/load.
- **Testé manuellement dans l'éditeur** : écran de connexion au lancement,
  création de compte (`testeur1`) via l'UI réelle, transition vers la carte
  du monde, compte confirmé fonctionnel côté backend au `curl` après coup.
- Limitation acceptée : la console F1 reste accessible sur l'écran de
  connexion (autoload global) ; `save`/`load` lancés avant connexion
  échouent silencieusement (401), comme "backend éteint" aujourd'hui.

### Phase 3 (suite) — Synchronisation coop de la carte (specs section 6 étape 7) : fait, pas commité

Plusieurs joueurs se voient bouger en temps réel sur la carte d'exploration.
Identité réseau volontairement **séparée** des comptes pour cette itération
(simple pseudo, sans vérification de jeton — décision utilisateur, à relier
plus tard). Hors scope ici (repoussé à l'étape 8 "combat coop") : combats/
villes partagés, collision entre joueurs, réconciliation stricte serveur→
client. Plan complet dans
`C:\Users\maxen\.claude\plans\linear-soaring-bird.md`.

- **`rust/src/network.rs`** (nouveau, autoload `CoopSession`, présent à
  l'identique côté client et côté serveur headless) : héberge une partie
  (`ENetMultiplayerPeer::create_server`, détecté via l'argument `--server`
  au lancement, ex. `Godot --headless --path rpg -- --server`, port 9000 par
  défaut) ou en rejoint une (`connect_to`, popup "Coop" dans `WorldScene`).
  RPC `#[rpc]` : `request_join`/`request_move` (client → serveur, rejoue
  `world::grid::step` — même logique pure que le solo, aucune duplication)
  et `sync_state` (serveur → clients, broadcast JSON du roster). Le serveur
  bascule sur une scène vide (`rpg/scenes/server_root.tscn`) pour ne jamais
  exécuter `login.tscn` en mode headless.
- **`rust/src/world/scene.rs`** : bouton de menu "Coop" (popup adresse/port/
  pseudo), dessin des joueurs distants (rect orange, sans interpolation —
  simplification volontaire pour cette itération), notifie `CoopSession`
  après chaque déplacement local.
- **`rust/src/world/grid.rs`** : `WORLD_BOUNDS` (constante partagée
  client/serveur, avant codée en dur dans `WorldScene::init`),
  `Direction::to_code`/`from_code` pour le transport réseau.
- **Bugs rencontrés et corrigés** :
  - Appeler `get_unique_id()` sur un `ENetMultiplayerPeer` pas encore
    connecté fait planter le client en boucle d'erreurs (une par frame).
    Corrigé en gardant une référence Rust explicite sur le peer
    (`CoopSession.peer`, sinon gdext pouvait le libérer trop tôt) et en ne
    touchant plus l'API multiplayer tant que `connected`/`is_server` est
    faux.
  - `request_join` initialisait tout nouveau joueur à `(0, 0)` côté serveur
    quelle que soit sa vraie position (restaurée depuis la sauvegarde) :
    les joueurs déjà présents le voyaient donc "téléporter" depuis
    l'origine de la grille jusqu'à sa vraie case au premier déplacement,
    au lieu de le voir directement à la bonne case en rejoignant. Corrigé
    en transmettant la position courante du joueur (`WorldScene::logical_pos`)
    dans `connect_to`/`request_join`, au lieu de la coder en dur.
- **Testé manuellement** : serveur headless + deux clients fenêtrés
  (`alice`/`bob`), jointure coop des deux côtés, déplacement d'un joueur
  visible en temps réel dans l'autre fenêtre, aucune erreur en log.

### Phase 3 (fin) — Combat coop (specs section 6, étape 8) : fait, pas commité

Plusieurs joueurs placent et jouent chacun leurs propres personnages dans
un même combat partagé. Décisions utilisateur : chaque joueur ne contrôle
que le tour de son/ses personnage(s) (pas de contrôle partagé), initiative
= la stat `speed` déjà existante (rien de nouveau côté moteur), tous les
joueurs connectés embarquent dans le même combat dès qu'une rencontre se
déclenche pour l'un d'eux. Le catalogue de personnages reste les 3 mêmes
(pas d'UI de sélection d'équipe, specs section 9) : répartis en
round-robin entre les joueurs connectés au moment où le combat démarre
("un ou plusieurs personnages chacun", specs 3.4 — 2 joueurs → 2+1, 1 seul
joueur → comportement solo inchangé). Plan complet dans
`C:\Users\maxen\.claude\plans\linear-soaring-bird.md`.

- **`rust/src/battle/coop.rs`** (nouveau) : `CoopBattle`, orchestration
  côté serveur — répartition des personnages, file de placement par
  joueur, validation qu'un joueur n'agit que pour son propre personnage
  (`act`), relance l'IA (`resolve_ai_turn`) pour les tours ennemis ou
  d'un joueur déconnecté (`on_disconnect`, ses personnages passent à
  l'IA). Testable en pur Rust (aucun appel direct à `randf()` : le tirage
  est un paramètre, comme `world::encounter::should_trigger`).
- **`rust/src/battle/engine.rs`** : extraction de `choose_ai_action`
  (déplacée hors de `BattleScene::run_ai_turn`, désormais réutilisée par
  le solo et le coop — même comportement, aucune duplication).
- **`rust/src/network.rs`** (`CoopSession` étendu) : le jet de rencontre
  (`ENCOUNTER_CHANCE`, désormais `pub(crate)` dans `world/scene.rs`) est
  rejoué côté serveur dans le traitement de `request_move` — évite que
  deux joueurs déclenchent chacun leur propre combat. Nouveaux RPC
  `request_place`/`request_battle_action` (client → serveur) et
  `sync_battle` (serveur → clients, un seul message réutilisé pour
  placement/combat/fin, même idée que `sync_state` pour la carte). Fin de
  combat : chaque client applique la même récompense à son propre état
  local (`session.rs`), pas de logique serveur de répartition — puis
  retour immédiat à la carte pour tout le monde (pas d'écran de
  victoire/défaite dédié en coop, contrairement au solo).
- **`rust/src/battle/scene.rs`** : `BattleScene` distingue solo/coop via
  `CoopSession` — en solo, comportement **strictement inchangé** ; en
  coop, l'affichage vient de la dernière diffusion reçue
  (`CoopSession::remote_battle`) et les clics passent par des requêtes
  RPC plutôt que d'appeler le moteur localement. Simplifications
  volontaires pour cette itération : pas d'étiquettes de PV flottantes en
  coop (juste les rectangles colorés), pas de prévisualisation de zone au
  survol, bouton "Fuir" masqué (abandon partiel d'un combat partagé non
  géré).
- **Limites acceptées** : rejoindre la session coop pendant qu'un combat
  est déjà en cours n'intègre pas ce joueur au combat (il attend la fin) ;
  un 4e joueur connecté n'aurait aucun personnage à contrôler (seulement 3
  fiches existent) ; l'identité réseau coop reste non reliée aux comptes
  (déjà noté à l'étape précédente).
- **Testé manuellement de bout en bout** : serveur headless + deux
  clients (`alice`/`bob`), combat déclenché naturellement (jet aléatoire)
  pour les deux à la fois, répartition 2/1 des personnages, placement
  partagé synchronisé, tours alternés avec le bon propriétaire à chaque
  fois (vérifié y compris le refus d'une tentative de bob hors de son
  tour), IA ennemie automatique entre les tours, victoire → retour
  synchronisé des deux clients sur la carte du monde.

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
3. **Serveur coop** (optionnel, pour tester la synchro multijoueur) :
   `D:\logiciel\godot\Godot_v4.7.2-stable_win64.exe --headless --path
   D:\dev\rpg\rpg -- --server` (port 9000 par défaut). Rejoindre depuis le
   bouton "Coop" de la carte du monde, dans deux instances du jeu lancées
   séparément.

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

- **Phase 4 des specs (PvP)** : la Phase 3 (déplacement + combat coop) est
  entièrement faite (voir ci-dessus), mais rien du PvP (matchmaking, combat
  entre équipes de joueurs humains).
- **Identité réseau coop non reliée aux comptes** : le pseudo saisi dans le
  popup "Coop" n'est pas vérifié contre `backend-api` (décision délibérée
  pour cette itération) — à relier avec le combat coop ou une itération
  dédiée.
- **Sélection de composition d'équipe** : `player_characters.selected`
  existe en base mais aucune UI pour la modifier (l'équipe reste la liste
  par défaut codée dans `rust/data/characters.json`).
- **Pas de vraie montée de niveau des personnages** : chaque combat repart
  des stats de base, seuls XP/or joueur et historique sont cumulatifs.
- **Défaite sans pénalité** : volontairement laissé tel quel (specs section
  9 flague ce point comme non défini).
- Points encore ouverts listés dans `specs-jeu-rpg-tactique.md` section 9
  (modèle des personnages, fonction précise des PDI, style visuel...).
