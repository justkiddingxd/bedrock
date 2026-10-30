use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use bedrock_proto::{ApiToken, AuthRequest, AuthResponse, CreateTokenRequest, CreatedToken, MeSettings};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::error::{ApiError, ApiResult};
use crate::{now, AppState, Registration};

/// The account behind the request's bearer token.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = bearer(&parts.headers).ok_or(ApiError::Unauthorized(SESSION_EXPIRED))?;
        user_for_token(state, token).await
    }
}

/// The token of an `Authorization: Bearer <token>` header, forgiving stray
/// spaces, quotes and the scheme's case. An empty token counts as none.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim().trim_matches('"');
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// Looks up a session or API token.
pub async fn user_for_token(st: &AppState, token: &str) -> ApiResult<AuthUser> {
    let row = sqlx::query("SELECT t.user_id, u.username FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?")
        .bind(token_hash(token))
        .fetch_optional(&st.db)
        .await?
        .ok_or(ApiError::Unauthorized(SESSION_EXPIRED))?;
    Ok(AuthUser { id: row.get(0), username: row.get(1) })
}

const SESSION_EXPIRED: &str = "сессия недействительна, войдите заново";

pub async fn register(State(st): State<AppState>, Json(req): Json<AuthRequest>) -> ApiResult<Json<AuthResponse>> {
    validate(&req)?;
    match st.cfg.registration {
        Registration::Closed => {
            return Err(ApiError::Forbidden("регистрация на этом сервере закрыта".into()));
        }
        Registration::FirstOnly if user_count(&st).await? > 0 => {
            return Err(ApiError::Forbidden("регистрация закрыта: у сервера уже есть владелец".into()));
        }
        _ => {}
    }

    let password = req.password.clone();
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await?
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?;

    let username = req.username.trim().to_string();
    let inserted = sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES (?, ?, ?)")
        .bind(&username)
        .bind(&hash)
        .bind(now())
        .execute(&st.db)
        .await;
    let user_id = match inserted {
        Ok(r) => r.last_insert_rowid(),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::Conflict("это имя пользователя уже занято".into()));
        }
        Err(e) => return Err(e.into()),
    };
    let (token, _) = issue_token(&st, user_id, None).await?;
    tracing::info!("registered user {username}");
    Ok(Json(AuthResponse { token, username }))
}

pub async fn login(State(st): State<AppState>, Json(req): Json<AuthRequest>) -> ApiResult<Json<AuthResponse>> {
    const BAD_LOGIN: &str = "неверное имя пользователя или пароль";
    let row = sqlx::query("SELECT id, username, password_hash FROM users WHERE username = ?")
        .bind(req.username.trim())
        .fetch_optional(&st.db)
        .await?
        .ok_or(ApiError::Unauthorized(BAD_LOGIN))?;
    let (id, username, hash): (i64, String, String) = (row.get(0), row.get(1), row.get(2));
    let password = req.password.clone();
    let ok = tokio::task::spawn_blocking(move || verify_password(&password, &hash)).await?;
    if !ok {
        return Err(ApiError::Unauthorized(BAD_LOGIN));
    }
    let (token, _) = issue_token(&st, id, None).await?;
    Ok(Json(AuthResponse { token, username }))
}

pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<()> {
    if let Some(token) = bearer(&headers) {
        // Only the session itself: named API tokens are revoked explicitly.
        sqlx::query("DELETE FROM tokens WHERE token_hash = ? AND name IS NULL").bind(token_hash(token)).execute(&st.db).await?;
    }
    Ok(())
}

pub async fn set_me(State(st): State<AppState>, user: AuthUser, Json(req): Json<MeSettings>) -> ApiResult<()> {
    if !(-12 * 60..=14 * 60).contains(&req.tz_offset_min) {
        return Err(ApiError::BadRequest("часовой пояс вне диапазона".into()));
    }
    sqlx::query("UPDATE users SET tz_offset = ? WHERE id = ?").bind(req.tz_offset_min).bind(user.id).execute(&st.db).await?;
    Ok(())
}

pub async fn list_tokens(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<ApiToken>>> {
    let rows = sqlx::query("SELECT rowid, name, created_at FROM tokens WHERE user_id = ? AND name IS NOT NULL ORDER BY created_at")
        .bind(user.id)
        .fetch_all(&st.db)
        .await?;
    Ok(Json(rows.iter().map(|r| ApiToken { id: r.get(0), name: r.get(1), created_at: r.get(2) }).collect()))
}

pub async fn create_token(
    State(st): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateTokenRequest>,
) -> ApiResult<Json<CreatedToken>> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::BadRequest("название токена: от 1 до 60 символов".into()));
    }
    let (token, id) = issue_token(&st, user.id, Some(name)).await?;
    Ok(Json(CreatedToken { id, name: name.to_string(), token }))
}

pub async fn delete_token(State(st): State<AppState>, user: AuthUser, Path(id): Path<i64>) -> ApiResult<()> {
    let res = sqlx::query("DELETE FROM tokens WHERE rowid = ? AND user_id = ? AND name IS NOT NULL")
        .bind(id)
        .bind(user.id)
        .execute(&st.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn user_count(st: &AppState) -> ApiResult<i64> {
    Ok(sqlx::query("SELECT COUNT(*) FROM users").fetch_one(&st.db).await?.get(0))
}

fn validate(req: &AuthRequest) -> ApiResult<()> {
    let u = req.username.trim();
    let valid_name = (3..=32).contains(&u.len())
        && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !valid_name {
        return Err(ApiError::BadRequest(
            "имя пользователя: 3–32 символа, латиница, цифры и _ - .".into(),
        ));
    }
    if req.password.chars().count() < 8 || req.password.len() > 1024 {
        return Err(ApiError::BadRequest("пароль должен быть не короче 8 символов".into()));
    }
    Ok(())
}

/// Creates a token (a login session when `name` is `None`) and returns it with its row id.
async fn issue_token(st: &AppState, user_id: i64, name: Option<&str>) -> ApiResult<(String, i64)> {
    let token = random_token();
    let res = sqlx::query("INSERT INTO tokens (token_hash, user_id, created_at, name) VALUES (?, ?, ?, ?)")
        .bind(token_hash(&token))
        .bind(user_id)
        .bind(now())
        .bind(name)
        .execute(&st.db)
        .await?;
    Ok((token, res.last_insert_rowid()))
}

pub fn random_token() -> String {
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default().hash_password(password.as_bytes(), &salt)?.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}
