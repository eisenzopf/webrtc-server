use std::error::Error as StdError;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    WebSocket(tokio_tungstenite::tungstenite::Error),
    Json(serde_json::Error),
    Room(String),
    Peer(String),
    Media(String),
    IO(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::WebSocket(e) => write!(f, "WebSocket error: {}", e),
            Error::Json(e) => write!(f, "JSON error: {}", e),
            Error::Room(e) => write!(f, "Room error: {}", e),
            Error::Peer(e) => write!(f, "Peer error: {}", e),
            Error::Media(e) => write!(f, "Media error: {}", e),
            Error::IO(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl StdError for Error {}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::Json(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>; 