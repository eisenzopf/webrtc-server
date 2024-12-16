use std::error::Error as StdError;
use std::fmt;
use std::net::AddrParseError;
use webrtc::Error as WebRTCError;
use tokio_tungstenite::tungstenite::Error as WsError;
use std::io::Error as IoError;
use turn::Error as TurnError;
use serde_json::Error as SerdeError;
use webrtc::util::Error as WebRTCUtilError;

#[derive(Debug)]
pub enum Error {
    WebRTCError(String),
    WebSocketError(String),
    ConnectionError(String),
    SerializationError(String),
    InvalidMessage(String),
    Room(String),
    Peer(String),
    Media(String),
    IO(String),
    Turn(String),
    AddrParse(String),
    WarpError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::WebRTCError(msg) => write!(f, "WebRTC error: {}", msg),
            Error::WebSocketError(msg) => write!(f, "WebSocket error: {}", msg),
            Error::ConnectionError(msg) => write!(f, "Connection error: {}", msg),
            Error::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
            Error::InvalidMessage(msg) => write!(f, "Invalid message: {}", msg),
            Error::Room(msg) => write!(f, "Room error: {}", msg),
            Error::Peer(msg) => write!(f, "Peer error: {}", msg),
            Error::Media(msg) => write!(f, "Media error: {}", msg),
            Error::IO(msg) => write!(f, "IO error: {}", msg),
            Error::Turn(msg) => write!(f, "TURN error: {}", msg),
            Error::AddrParse(msg) => write!(f, "Address parse error: {}", msg),
            Error::WarpError(msg) => write!(f, "Warp error: {}", msg),
        }
    }
}

impl StdError for Error {}

impl From<WebRTCError> for Error {
    fn from(err: WebRTCError) -> Self {
        Error::WebRTCError(err.to_string())
    }
}

impl From<WsError> for Error {
    fn from(err: WsError) -> Self {
        Error::WebSocketError(err.to_string())
    }
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Self {
        Error::IO(err.to_string())
    }
}

impl From<TurnError> for Error {
    fn from(err: TurnError) -> Self {
        Error::Turn(err.to_string())
    }
}

impl From<SerdeError> for Error {
    fn from(err: SerdeError) -> Self {
        Error::SerializationError(err.to_string())
    }
}

impl From<AddrParseError> for Error {
    fn from(err: AddrParseError) -> Self {
        Error::AddrParse(err.to_string())
    }
}

impl From<warp::Error> for Error {
    fn from(err: warp::Error) -> Self {
        Error::WarpError(err.to_string())
    }
}

impl From<WebRTCUtilError> for Error {
    fn from(err: WebRTCUtilError) -> Self {
        Error::WebRTCError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>; 