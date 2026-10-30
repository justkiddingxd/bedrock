use crate::crypto::CryptoError;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("сеть: {0}")]
    Http(#[from] reqwest::Error),
    /// 4xx answers carry a message meant for the user; 5xx ones get the status too.
    #[error("{}", if *status < 500 { message.clone() } else { format!("сервер ответил {status}: {message}") })]
    Server { status: u16, message: String },
    #[error("{0}")]
    Unauthorized(String),
    #[error("неверный пароль хранилища")]
    WrongPassword,
    #[error("шифрование: {0}")]
    Crypto(String),
    #[error(transparent)]
    Core(#[from] bedrock_core::Error),
    #[error("ввод-вывод: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("синхронизация не настроена")]
    NotConfigured,
    #[error("{0}")]
    Other(String),
}

impl From<CryptoError> for SyncError {
    fn from(e: CryptoError) -> Self {
        match e {
            CryptoError::WrongPassword => SyncError::WrongPassword,
            other => SyncError::Crypto(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;
