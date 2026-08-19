# Specs — RPG tactique multijoueur (nom à définir)

## 1. Concept général

Jeu de rôle tactique inspiré des premiers Pokémon (exploration sur grille, points
d'intérêt) et des jeux de combat sur grille façon Dofus/Wakfu.

Le joueur explore une carte du monde en se déplaçant case par case entre des points
d'intérêt : villes, mines, temples, donjons, etc. Il constitue une équipe de 4
personnages. En se déplaçant, il peut déclencher des combats aléatoires contre des
ennemis. Gagner un combat rapporte de l'XP joueur, de l'XP personnage, de l'argent et
du loot.

**Multijoueur** : jusqu'à 4 joueurs peuvent jouer ensemble sur la même équipe (chacun
contrôle un ou plusieurs des 4 personnages) en coopératif contre l'IA (donjons, mobs),
ou s'affronter en PvP, équipe contre équipe.

## 2. Boucle de jeu

1. **Exploration** — déplacement sur la carte du monde (grille), visite des points
   d'intérêt, déclenchement aléatoire de combats en se déplaçant.
2. **Combat** — écran dédié, plateau tactique, résolution au tour par tour.
3. **Progression** — XP joueur, XP personnage, argent, loot récupérés après victoire,
   utilisés pour renforcer l'équipe.
4. Retour à l'exploration, avec une équipe plus forte.

## 3. Systèmes de jeu détaillés

### 3.1 Exploration

- Carte du monde en grille (déplacement case par case, façon Pokémon Rouge/Bleu).
- Points d'intérêt : villes (boutiques, recrutement, quêtes), mines, temples, donjons.
  Chaque type de PDI a une fonction de gameplay différente (à détailler plus tard).
- Rencontres aléatoires : probabilité de déclenchement à chaque déplacement, probablement
  variable selon la zone/terrain (ex: plus élevée en forêt ou en mine qu'en ville).

### 3.2 Équipe et personnages

- Équipe de 4 personnages maximum.
- Chaque personnage progresse individuellement (XP personnage, niveau, stats, sorts
  débloqués — modèle à définir : classes fixes ? personnages recrutables avec identité
  propre ?).
- Le joueur (ou l'équipe en multi) progresse aussi via un XP joueur global.

### 3.3 Combat

- Changement d'écran à l'entrée en combat.
- Écran split en 2 zones : zone du/des joueur(s) et zone adverse (IA ou joueurs
  adverses en PvP) — orientation gauche/droite ou haut/bas à trancher visuellement
  plus tard, ça n'a pas d'impact sur la logique.
- Chaque camp dispose d'un plateau de **3x4 cases**. Placement d'un personnage par case
  avant le lancement du combat.
- Résolution **au tour par tour**, personnage par personnage.
- **Ordre des actions déterminé par la vitesse** de chaque personnage (le plus rapide
  agit en premier). Recommandation à valider : c'est le système le plus lisible et le
  plus simple à équilibrer pour ce type de jeu.
- Chaque personnage choisit une action à son tour et cible une case (la sienne, une
  case adverse, ou une case alliée selon le sort).
- Sorts en zone : prévisualisation des cases touchées avant validation de l'action.
- Fin de combat : victoire → XP joueur + XP personnage + argent + loot. Défaite → à
  définir (perte de rien ? retour en ville ? pénalité légère ?).

### 3.4 Modes multijoueur

- **Solo PvE** : un joueur contrôle les 4 personnages de son équipe, combat contre l'IA.
- **Coop PvE** : jusqu'à 4 joueurs rejoignent la même équipe (un ou plusieurs
  personnages chacun), explorent et combattent ensemble contre l'IA (donjons, mobs).
- **PvP** : une équipe de 4 personnages (contrôlée par 1 à 4 joueurs) affronte une
  équipe adverse humaine, sur le même système de plateau 3x4.

## 4. Stack technique recommandée

- **Moteur / Client** : **Godot 4** — éditeur de scènes, gestion native des tilemaps
  et grilles (carte du monde + plateau de combat 3x4), export mobile (Android/iOS)
  mature, API multijoueur haut niveau intégrée (RPCs, modèle autoritaire).
- **Langage principal** : **Rust**, via le binding officiel `gdext` (GDExtension) —
  toute la logique de jeu (mouvement, moteur de combat, calculs de dégâts, IA, état
  réseau) s'écrit en Rust et s'exécute comme du code natif dans Godot, sans le
  ralentissement du GDScript interprété.
- **Serveur temps réel** : instance **Godot headless** (sans interface graphique)
  tournant la même logique Rust côté serveur — un seul et même code de combat/
  déplacement, utilisé aussi bien en local (solo) qu'en autoritaire (multi), pas de
  duplication de logique entre client et serveur.
- **Backend de persistance** : API séparée en Rust (**axum** ou **actix-web**, tous
  deux matures) pour tout ce qui ne nécessite pas de temps réel : comptes, sauvegarde
  d'équipe, inventaire, progression.
- **Base de données** : PostgreSQL (ou SQLite pour démarrer) — tu es à l'aise en SQL,
  donc pas de changement de ce côté, seul le langage qui interroge la BDD change
  (crate `sqlx` ou `diesel` côté Rust).

## 5. Architecture

Deux briques serveur bien séparées, toutes deux en Rust :

1. **API persistante** (axum/actix-web) : création de compte, sauvegarde/chargement
   d'équipe, inventaire, progression. Pas besoin de temps réel ici, une API REST
   classique suffit.
2. **Serveur temps réel (Godot headless + Rust/gdext)** : uniquement pour les sessions
   actives — déplacement synchronisé en coop sur la carte, résolution des tours de
   combat en multi. Le serveur reste **autoritaire** sur les résultats de combat
   (calculs de dégâts, ordre des tours) pour éviter la triche.

```
[Client Godot/Rust] ──(API REST/HTTP)──► [Backend Rust (axum) : comptes, saves] ──► DB
       │
       └──(RPCs Godot / ENet)──► [Serveur Godot headless (Rust) : exploration, combats]
```

**Remarque d'apprentissage** : `gdext` est solide mais plus jeune et moins documenté
que le GDScript natif — attends-toi à des temps de compilation à chaque changement, et
un peu plus de lecture de code source que de tutoriels tout faits. Si le rythme
d'apprentissage de Rust + gdext freine trop la Phase 1, il reste possible de prototyper
en GDScript au départ et migrer la logique vers Rust module par module — mais l'objectif
ici est bien Rust de bout en bout.

## 6. Scope MVP — développement en phases

Vu l'ampleur du projet (exploration + combat tactique + multi + persistance), il est
essentiel de ne pas tout attaquer en même temps. Ordre recommandé :

**Phase 1 — Solo, sans réseau**
1. Déplacement sur une carte en grille (un seul écran/zone pour commencer)
2. Un point d'intérêt fonctionnel (ex: une ville basique)
3. Déclenchement d'un combat aléatoire (juste la transition d'écran)
4. Plateau de combat 3x4 par camp, placement des personnages, tour par tour contre une
   IA simple (1-2 sorts basiques, ciblage, prévisualisation de zone)
5. Résolution de victoire : XP, argent, loot (même basique)

**Phase 2 — Persistance**
6. Sauvegarde de l'équipe et de la progression (même en solo, juste en local ou via une
   petite base de données)

**Phase 3 — Multijoueur coopératif**
7. Plusieurs joueurs sur la même carte d'exploration (positions synchronisées)
8. Combat coop : plusieurs joueurs contrôlent les personnages d'une même équipe pendant
   un combat

**Phase 4 — PvP**
9. Matchmaking / défi entre deux équipes de joueurs
10. Résolution de combat PvP (même logique que PvE, adversaire humain au lieu de l'IA)

Ne pas passer à une phase tant que la précédente n'est pas stable et amusante à jouer.

## 7. Structure de projet suggérée

```
mon-jeu/
├── godot-project/            # Projet Godot 4 (client ET serveur headless)
│   ├── scenes/                # WorldScene, TownScene, BattleScene, MenuScene (.tscn)
│   ├── rust/                  # Crate Rust (gdext) — logique du jeu
│   │   └── src/
│   │       ├── world/          # déplacement, grille, points d'intérêt
│   │       ├── battle/         # moteur de combat pur (sans dépendance à l'affichage)
│   │       ├── characters/     # personnages, stats, sorts
│   │       └── network/        # RPCs, synchronisation, logique côté serveur
│   └── project.godot
├── backend-api/               # Crate Rust séparée (axum/actix-web)
│   ├── src/
│   │   ├── routes/             # comptes, sauvegarde, inventaire
│   │   └── db/                 # modèles + requêtes (sqlx/diesel)
│   └── Cargo.toml
└── shared/                     # crate Rust partagée : types communs (stats, sorts,
                                 # formats de grille) entre le jeu et le backend
```

## 8. Points d'attention pour Claude Code

- Commencer 100% solo (Phase 1) avant même de mentionner le réseau. Le système de
  combat tactique (grille, ciblage, zones, tour par tour) est déjà un projet en soi.
- Écrire le moteur de combat comme une **crate Rust pure**, sans dépendance à Godot ou
  à l'affichage (juste des structs/fonctions qui prennent un état et retournent un
  nouvel état). Cette même crate pourra tourner telle quelle côté client (solo) et
  côté serveur headless (multi) plus tard, sans rien réécrire.
- Modéliser les personnages et les sorts avec des données (fichiers JSON/RON/TOML)
  plutôt que du code en dur, pour pouvoir en ajouter facilement sans toucher au moteur.
- Pour la prévisualisation de zone, prévoir un système générique de "pattern" de cases
  autour d'une cible (ex: croix, carré 3x3, ligne) réutilisable pour tous les sorts.
- Si l'apprentissage de Rust + gdext ralentit trop au début, ne pas hésiter à
  prototyper une mécanique en GDScript pour valider l'idée rapidement, puis la
  migrer en Rust une fois validée — mieux vaut itérer vite sur le game design que de
  bloquer sur le borrow checker en phase d'exploration créative.
- Le passage au coop/PvP (Phase 3-4) sera direct si le moteur de combat est déjà une
  crate Rust isolée et pure dès la Phase 1 : il suffira de la brancher au serveur
  headless au lieu de la dupliquer.

## 9. Points encore à définir (à trancher avant ou pendant le développement)

- Modèle des personnages : classes fixes, personnages recrutables avec identité propre,
  ou création libre ?
- Détail des sorts/actions disponibles par personnage
- Ce qui se passe en cas de défaite (pénalité, retour en ville, perte d'objets ?)
- Fonction précise de chaque type de PDI (mine, temple, donjon)
- Orientation visuelle de l'écran de combat (gauche/droite vs haut/bas)
- Style visuel (pixel art, plus stylisé, etc.)
