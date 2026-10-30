use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bedrock_proto::ErrorBody;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized(&'static str),
    Forbidden(String),
    NotFound,
    Conflict(String),
    Internal(anyhow::Error),
}

impl ApiError {
    /// The text a client sees.
    pub fn message(&self) -> String {
        match self {
            ApiError::BadRequest(m) | ApiError::Forbidden(m) | ApiError::Conflict(m) => m.clone(),
            ApiError::Unauthorized(m) => m.to_string(),
            ApiError::NotFound => "не найдено".into(),
            ApiError::Internal(_) => "внутренняя ошибка сервера".into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.to_string()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m),
            ApiError::Internal(e) => {
                tracing::error!("internal error: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError::Internal(e.into())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
