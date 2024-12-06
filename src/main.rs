use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, WebSocketStream};
use futures_util::{StreamExt, SinkExt};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use anyhow::Result;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum SignalingMessage {
    Join {
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
}

type PeerConnection = (String, WebSocketStream<TcpStream>);
type PeerMap = Arc<RwLock<HashMap<String, Vec<PeerConnection>>>>;

pub struct SignalingServer {
    address: String,
    peers: PeerMap,
}

impl SignalingServer {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn start(&self) -> Result<()> {
        println!("Attempting to bind to {}", self.address);
        let listener = TcpListener::bind(&self.address).await.map_err(|e| {
            eprintln!("Failed to bind to {}: {}", self.address, e);
            anyhow::anyhow!("Failed to bind: {}", e)
        })?;
        println!("Successfully bound to {}", self.address);
        println!("Signaling server listening on {}", self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            println!("New connection from: {}", addr);
            let peers = self.peers.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, peers).await {
                    eprintln!("Error handling connection: {}", e);
                }
            });
        }

        Ok(())
    }

    async fn handle_connection(stream: TcpStream, peers: PeerMap) -> Result<()> {
        let ws_stream = accept_async(stream).await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        while let Some(msg) = ws_receiver.next().await {
            let msg = msg?;
            let msg_str = msg.to_text()?;
            let signal_msg: SignalingMessage = serde_json::from_str(msg_str)?;

            match signal_msg {
                SignalingMessage::Join { room_id, peer_id } => {
                    let mut peers_write = peers.write().await;
                    let room_peers = peers_write.entry(room_id.clone()).or_insert_with(Vec::new);
                    let ws_stream = ws_sender.reunite(ws_receiver)?;
                    room_peers.push((peer_id.clone(), ws_stream));
                    
                    // Send current peer list to all peers in room
                    let peer_list = room_peers.iter()
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>();
                    
                    Self::broadcast_to_room(&peers, &room_id, &SignalingMessage::PeerList {
                        peers: peer_list,
                    }).await?;
                    break;
                },
                SignalingMessage::RequestPeerList => {
                    let peers_read = peers.read().await;
                    let all_peers = peers_read.values()
                        .flat_map(|room| room.iter().map(|(id, _)| id.clone()))
                        .collect::<Vec<_>>();
                    
                    ws_sender.send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::to_string(&SignalingMessage::PeerList {
                            peers: all_peers,
                        })?,
                    )).await?;
                },
                SignalingMessage::Offer { room_id, sdp, from_peer, to_peer } => {
                    Self::broadcast_to_room(&peers, &room_id, &SignalingMessage::Offer { 
                        room_id: room_id.clone(), 
                        sdp,
                        from_peer,
                        to_peer
                    }).await?;
                },
                SignalingMessage::Answer { room_id, sdp, from_peer, to_peer } => {
                    Self::broadcast_to_room(&peers, &room_id, &SignalingMessage::Answer { 
                        room_id: room_id.clone(), 
                        sdp,
                        from_peer,
                        to_peer
                    }).await?;
                },
                SignalingMessage::IceCandidate { room_id, candidate, from_peer, to_peer } => {
                    Self::broadcast_to_room(&peers, &room_id, &SignalingMessage::IceCandidate { 
                        room_id: room_id.clone(), 
                        candidate,
                        from_peer,
                        to_peer
                    }).await?;
                },
                _ => {
                    // Optionally log unhandled message types
                    println!("Received unhandled message type");
                }
            }
        }

        Ok(())
    }

    async fn broadcast_to_room(peers: &PeerMap, room_id: &str, message: &SignalingMessage) -> Result<()> {
        let mut peers_write = peers.write().await;
        if let Some(room_peers) = peers_write.get_mut(room_id) {
            let msg = tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&message)?
            );
            for (_, ws_stream) in room_peers.iter_mut() {
                if let Err(e) = ws_stream.send(msg.clone()).await {
                    eprintln!("Error broadcasting message: {}", e);
                }
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = SignalingServer::new("127.0.0.1:8080");
    server.start().await?;
    Ok(())
}
