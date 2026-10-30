#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ошибка ввода-вывода: {0}")]
    Io(#[from] std::io::Error),
    #[error("недопустимый путь: {0}")]
    InvalidPath(String),
    #[error("не найдено: {0}")]
    NotFound(String),
    #[error("уже существует: {0}")]
    AlreadyExists(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
