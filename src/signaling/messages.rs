use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use futures_util::stream::SplitSink;
use tokio::net::TcpStream;

pub type WebSocketSender = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "message_type")]
pub enum SignalingMessage {
    Join {
        room_id: String,
        peer_id: String,
        #[serde(skip)]
        sender: Option<Arc<tokio::sync::Mutex<WebSocketSender>>>,
    },
    Disconnect {
        room_id: String,
        peer_id: String,
    },
    PeerList {
        peers: Vec<String>,
    },
    Offer {
        room_id: String,
        sdp: String,
        from_peer: String,
        to_peer: String,
    },
    Answer {
        room_id: String,
        sdp: String,
        from_peer: String,
        to_peer: String,
    },
    IceCandidate {
        room_id: String,
        candidate: String,
        from_peer: String,
        to_peer: String,
    },
    RequestPeerList,
    InitiateCall {
        peer_id: String,
        room_id: String,
    },
    MediaError {
        error_type: String,
        description: String,
        peer_id: String,
    },
    EndCall {
        room_id: String,
        peer_id: String,
    },
    PeerDisconnected {
        room_id: String,
        peer_id: String,
    },
    CallRequest {
        room_id: String,
        from_peer: String,
        to_peers: Vec<String>,
    },
    CallResponse {
        room_id: String,
        from_peer: String,
        to_peer: String,
        accepted: bool,
    }
}

impl Default for SignalingMessage {
    fn default() -> Self {
        SignalingMessage::PeerList { peers: Vec::new() }
    }
} 