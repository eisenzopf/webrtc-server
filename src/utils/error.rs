use std::error::Error as StdError;
use std::fmt;
use std::net::AddrParseError;

#[derive(Debug)]
pub enum Error {
    WebSocket(tokio_tungstenite::tungstenite::Error),
    Json(serde_json::Error),
    Room(String),
    Peer(String),
    Media(String),
    IO(std::io::Error),
    Turn(turn::Error),
    AddrParse(AddrParseError),
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
            Error::Turn(e) => write!(f, "TURN error: {}", e),
            Error::AddrParse(e) => write!(f, "Address parse error: {}", e),
        }
    }
}

impl StdError for Error {}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::Json(error)
    }
}

impl From<webrtc::Error> for Error {
    fn from(error: webrtc::Error) -> Self {
        Error::Media(error.to_string())
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for Error {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        Error::WebSocket(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::IO(error)
    }
}

impl From<turn::Error> for Error {
    fn from(error: turn::Error) -> Self {
        Error::Turn(error)
    }
}

impl From<AddrParseError> for Error {
    fn from(error: AddrParseError) -> Self {
        Error::AddrParse(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>; 