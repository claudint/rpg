//! API de persistance (specs section 4-5) : reçoit les requêtes HTTP du jeu
//! Godot et parle à Postgres. Sauvegarde unique pour l'instant (pas de
//! comptes/multi-joueurs, specs section 9) : une seule ligne en base, id=1.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;

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

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

struct ApiError(sqlx::Error);

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
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
    let app = Router::new().route("/save", get(get_save).put(put_save)).with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    println!("backend-api écoute sur http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("impossible d'écouter le port 8080");
    axum::serve(listener, app).await.expect("le serveur a planté");
}

async fn get_save(State(state): State<AppState>) -> Result<Json<SaveData>, ApiError> {
    let row: Option<SaveRow> = sqlx::query_as("SELECT xp, gold, pos_x, pos_y FROM saves WHERE id = 1")
        .fetch_optional(&state.pool)
        .await?;
    let (xp, gold, pos_x, pos_y) = row.map(|r| (r.xp, r.gold, r.pos_x, r.pos_y)).unwrap_or((0, 0, 0, 0));

    let inventory_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT items.name FROM inventory_items
         JOIN items ON items.id = inventory_items.item_id
         WHERE inventory_items.save_id = 1",
    )
    .fetch_all(&state.pool)
    .await?;
    let inventory = inventory_rows.into_iter().map(|(item,)| item).collect();

    let history_rows: Vec<HistoryRow> = sqlx::query_as(
        "SELECT victory, xp, gold, loot FROM battle_history WHERE save_id = 1 ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    let history = history_rows
        .into_iter()
        .map(|r| BattleRecord { victory: r.victory, xp: r.xp, gold: r.gold, loot: r.loot })
        .collect();

    Ok(Json(SaveData { xp, gold, pos_x, pos_y, inventory, history }))
}

async fn put_save(State(state): State<AppState>, Json(data): Json<SaveData>) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;

    sqlx::query(
        "INSERT INTO saves (id, xp, gold, pos_x, pos_y, updated_at) VALUES (1, $1, $2, $3, $4, now())
         ON CONFLICT (id) DO UPDATE SET xp = $1, gold = $2, pos_x = $3, pos_y = $4, updated_at = now()",
    )
    .bind(data.xp)
    .bind(data.gold)
    .bind(data.pos_x)
    .bind(data.pos_y)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM inventory_items WHERE save_id = 1").execute(&mut *tx).await?;
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

        sqlx::query("INSERT INTO inventory_items (save_id, item_id) VALUES (1, $1)")
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query("DELETE FROM battle_history WHERE save_id = 1").execute(&mut *tx).await?;
    for record in &data.history {
        sqlx::query("INSERT INTO battle_history (save_id, victory, xp, gold, loot) VALUES (1, $1, $2, $3, $4)")
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
