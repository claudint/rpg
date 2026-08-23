//! API de persistance (specs section 4-5) : reçoit les requêtes HTTP du jeu
//! Godot et parle à Postgres. Comptes joueurs (Phase 3) : chaque sauvegarde
//! est désormais rattachée au joueur authentifié via un jeton de session
//! (`Authorization: Bearer <token>`, voir `AuthedPlayer`), plus à l'id 1 en
//! dur.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct SaveData {
    xp: i32,
    gold: i32,
    pos_x: i32,
    pos_y: i32,
    inventory: Vec<String>,
    history: Vec<BattleRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BattleRecord {
    victory: bool,
    xp: i32,
    gold: i32,
    loot: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SaveRow {
    id: i32,
    xp: i32,
    gold: i32,
    pos_x: i32,
    pos_y: i32,
}

#[derive(sqlx::FromRow)]
struct HistoryRow {
    victory: bool,
    xp: i32,
    gold: i32,
    loot: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthRequest {
    name: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    token: String,
    player_id: i32,
}

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

/// Joueur authentifié : extrait de l'en-tête `Authorization: Bearer <token>`
/// par recherche dans `sessions`. Tout handler qui prend ce type en
/// paramètre exige un jeton valide, sans code de vérification répété.
struct AuthedPlayer(i32);

#[axum::async_trait]
impl FromRequestParts<AppState> for AuthedPlayer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;

        let row: Option<(i32,)> = sqlx::query_as("SELECT player_id FROM sessions WHERE token = $1")
            .bind(token)
            .fetch_optional(&state.pool)
            .await?;

        row.map(|(player_id,)| AuthedPlayer(player_id)).ok_or(ApiError::Unauthorized)
    }
}

enum ApiError {
    Db(sqlx::Error),
    Unauthorized,
    Conflict(String),
    BadRequest(String),
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Db(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "identifiants invalides").into_response(),
            Self::Conflict(message) => (StatusCode::CONFLICT, message).into_response(),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
        }
    }
}

/// Hache un mot de passe en clair avec un sel aléatoire (argon2, format
/// PHC — le sel et les paramètres sont encodés dans la chaîne retournée).
fn hash_password(password: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| ApiError::BadRequest("mot de passe invalide".to_string()))
}

/// Vérifie un mot de passe en clair contre un hash PHC stocké en base.
fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

/// Crée une session pour un joueur et renvoie le jeton généré, prêt à être
/// renvoyé au client (`register`/`login` traitent l'inscription comme une
/// connexion automatique).
async fn create_session(pool: &PgPool, player_id: i32) -> Result<String, ApiError> {
    let token = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (token, player_id) VALUES ($1, $2)")
        .bind(&token)
        .bind(player_id)
        .execute(pool)
        .await?;
    Ok(token)
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL manquant : crée backend-api/.env (voir .env.example)");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connexion à Postgres échouée");

    sqlx::migrate!("./migrations").run(&pool).await.expect("migration échouée");

    let state = AppState { pool };
    let app = Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/save", get(get_save).put(put_save))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    println!("backend-api écoute sur http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("impossible d'écouter le port 8080");
    axum::serve(listener, app).await.expect("le serveur a planté");
}

async fn register(
    State(state): State<AppState>,
    Json(data): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    if data.name.trim().is_empty() || data.password.is_empty() {
        return Err(ApiError::BadRequest("nom et mot de passe requis".to_string()));
    }
    let hash = hash_password(&data.password)?;

    // ON CONFLICT DO NOTHING plutôt qu'un SELECT préalable : évite une
    // course entre la vérification et l'insertion si deux inscriptions au
    // même nom arrivent en même temps.
    let row: Option<(i32,)> = sqlx::query_as(
        "INSERT INTO players (name, password_hash) VALUES ($1, $2)
         ON CONFLICT (name) DO NOTHING
         RETURNING id",
    )
    .bind(&data.name)
    .bind(&hash)
    .fetch_optional(&state.pool)
    .await?;

    let Some((player_id,)) = row else {
        return Err(ApiError::Conflict("ce nom est déjà pris".to_string()));
    };

    let token = create_session(&state.pool, player_id).await?;
    Ok(Json(AuthResponse { token, player_id }))
}

async fn login(State(state): State<AppState>, Json(data): Json<AuthRequest>) -> Result<Json<AuthResponse>, ApiError> {
    let row: Option<(i32, String)> = sqlx::query_as("SELECT id, password_hash FROM players WHERE name = $1")
        .bind(&data.name)
        .fetch_optional(&state.pool)
        .await?;

    let Some((player_id, hash)) = row else {
        return Err(ApiError::Unauthorized);
    };
    if !verify_password(&data.password, &hash) {
        return Err(ApiError::Unauthorized);
    }

    let token = create_session(&state.pool, player_id).await?;
    Ok(Json(AuthResponse { token, player_id }))
}

async fn get_save(
    State(state): State<AppState>,
    AuthedPlayer(player_id): AuthedPlayer,
) -> Result<Json<SaveData>, ApiError> {
    let row: Option<SaveRow> = sqlx::query_as("SELECT id, xp, gold, pos_x, pos_y FROM saves WHERE player_id = $1")
        .bind(player_id)
        .fetch_optional(&state.pool)
        .await?;

    // Pas encore de sauvegarde pour ce joueur (compte tout juste créé) :
    // valeurs par défaut, sans requêter inventaire/historique (aucun
    // save_id à joindre).
    let Some(save) = row else {
        return Ok(Json(SaveData { xp: 0, gold: 0, pos_x: 0, pos_y: 0, inventory: vec![], history: vec![] }));
    };

    let inventory_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT items.name FROM inventory_items
         JOIN items ON items.id = inventory_items.item_id
         WHERE inventory_items.save_id = $1",
    )
    .bind(save.id)
    .fetch_all(&state.pool)
    .await?;
    let inventory = inventory_rows.into_iter().map(|(item,)| item).collect();

    let history_rows: Vec<HistoryRow> = sqlx::query_as(
        "SELECT victory, xp, gold, loot FROM battle_history WHERE save_id = $1 ORDER BY id",
    )
    .bind(save.id)
    .fetch_all(&state.pool)
    .await?;
    let history = history_rows
        .into_iter()
        .map(|r| BattleRecord { victory: r.victory, xp: r.xp, gold: r.gold, loot: r.loot })
        .collect();

    Ok(Json(SaveData { xp: save.xp, gold: save.gold, pos_x: save.pos_x, pos_y: save.pos_y, inventory, history }))
}

async fn put_save(
    State(state): State<AppState>,
    AuthedPlayer(player_id): AuthedPlayer,
    Json(data): Json<SaveData>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;

    let (save_id,): (i32,) = sqlx::query_as(
        "INSERT INTO saves (player_id, xp, gold, pos_x, pos_y, updated_at) VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (player_id) DO UPDATE SET xp = $2, gold = $3, pos_x = $4, pos_y = $5, updated_at = now()
         RETURNING id",
    )
    .bind(player_id)
    .bind(data.xp)
    .bind(data.gold)
    .bind(data.pos_x)
    .bind(data.pos_y)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM inventory_items WHERE save_id = $1").bind(save_id).execute(&mut *tx).await?;
    for item in &data.inventory {
        // Upsert dans le catalogue d'objets : RETURNING id fonctionne aussi
        // bien à la création qu'en cas de conflit sur le nom (déjà connu).
        let (item_id,): (i32,) = sqlx::query_as(
            "INSERT INTO items (name) VALUES ($1)
             ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
             RETURNING id",
        )
        .bind(item)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query("INSERT INTO inventory_items (save_id, item_id) VALUES ($1, $2)")
            .bind(save_id)
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query("DELETE FROM battle_history WHERE save_id = $1").bind(save_id).execute(&mut *tx).await?;
    for record in &data.history {
        sqlx::query("INSERT INTO battle_history (save_id, victory, xp, gold, loot) VALUES ($1, $2, $3, $4, $5)")
            .bind(save_id)
            .bind(record.victory)
            .bind(record.xp)
            .bind(record.gold)
            .bind(&record.loot)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(StatusCode::OK)
}
