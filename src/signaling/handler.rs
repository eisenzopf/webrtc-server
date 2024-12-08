use crate::utils::{Error, Result};
use crate::room::Room;
use crate::metrics::ConnectionMetrics;
use crate::signaling::messages::SignalingMessage;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use std::collections::HashMap;
use tokio_tungstenite::tungstenite::Message;
use futures_util::SinkExt;

pub type WebSocketSender = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    Message
>;
pub type PeerConnection = (String, Arc<Mutex<WebSocketSender>>);
pub type PeerMap = Arc<RwLock<HashMap<String, Vec<PeerConnection>>>>;

pub struct MessageHandler {
    peers: PeerMap,
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    metrics: Arc<RwLock<HashMap<String, ConnectionMetrics>>>,
}

impl MessageHandler {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            rooms: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn broadcast_to_room(&self, room_id: &str, message: &SignalingMessage) -> Result<()> {
        let peers_read = self.peers.read().await;
        if let Some(room_peers) = peers_read.get(room_id) {
            if let Ok(msg_str) = serde_json::to_string(message) {
                for (peer_id, sender) in room_peers {
                    let mut sender = sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(msg_str.clone())).await {
                        eprintln!("Failed to send message to peer {}: {}", peer_id, e);
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn handle_message(
        &self,
        message: SignalingMessage,
        sender: Arc<Mutex<WebSocketSender>>,
        current_peer_id: &mut Option<String>,
        current_room_id: &mut Option<String>,
    ) -> Result<()> {
        match message {
            SignalingMessage::Join { room_id, peer_id } => {
                self.handle_join(room_id, peer_id, sender, current_peer_id, current_room_id).await
            },
            SignalingMessage::Disconnect { room_id, peer_id } => {
                self.handle_disconnect(room_id, peer_id).await
            },
            SignalingMessage::Offer { ref room_id, ref sdp, ref from_peer, ref to_peer } => {
                // Forward the offer to the target peer
                let peers = self.peers.read().await;
                if let Some(room_peers) = peers.get(room_id) {
                    if let Some((_, target_sender)) = room_peers.iter().find(|(id, _)| id == to_peer) {
                        let mut sender = target_sender.lock().await;
                        let msg = serde_json::to_string(&message)?;
                        sender.send(Message::Text(msg)).await
                            .map_err(|e| Error::WebSocket(e))?;
                    }
                }
                Ok(())
            },
            SignalingMessage::Answer { ref room_id, ref sdp, ref from_peer, ref to_peer } => {
                // Forward the answer to the target peer
                let peers = self.peers.read().await;
                if let Some(room_peers) = peers.get(room_id) {
                    if let Some((_, target_sender)) = room_peers.iter().find(|(id, _)| id == to_peer) {
                        let mut sender = target_sender.lock().await;
                        let msg = serde_json::to_string(&message)?;
                        sender.send(Message::Text(msg)).await
                            .map_err(|e| Error::WebSocket(e))?;
                    }
                }
                Ok(())
            },
            SignalingMessage::IceCandidate { ref room_id, ref candidate, ref from_peer, ref to_peer } => {
                // Forward ICE candidates to the target peer
                let peers = self.peers.read().await;
                if let Some(room_peers) = peers.get(room_id) {
                    if let Some((_, target_sender)) = room_peers.iter().find(|(id, _)| id == to_peer) {
                        let mut sender = target_sender.lock().await;
                        let msg = serde_json::to_string(&message)?;
                        sender.send(Message::Text(msg)).await
                            .map_err(|e| Error::WebSocket(e))?;
                    }
                }
                Ok(())
            },
            SignalingMessage::EndCall { room_id, peer_id } => {
                // Notify other peers in the room about the ended call
                let peers = self.peers.read().await;
                if let Some(room_peers) = peers.get(&room_id) {
                    for (other_peer_id, sender) in room_peers {
                        if other_peer_id != &peer_id {
                            let end_call_msg = SignalingMessage::PeerList {
                                peers: room_peers.iter()
                                    .map(|(id, _)| id.clone())
                                    .collect()
                            };
                            let mut sender = sender.lock().await;
                            if let Ok(msg) = serde_json::to_string(&end_call_msg) {
                                let _ = sender.send(Message::Text(msg)).await;
                            }
                        }
                    }
                }
                Ok(())
            },
            _ => Ok(()),
        }
    }

    async fn handle_join(
        &self,
        room_id: String,
        peer_id: String,
        sender: Arc<Mutex<WebSocketSender>>,
        current_peer_id: &mut Option<String>,
        current_room_id: &mut Option<String>,
    ) -> Result<()> {
        *current_peer_id = Some(peer_id.clone());
        *current_room_id = Some(room_id.clone());

        // Add peer to room
        {
            let mut rooms = self.rooms.write().await;
            let room = rooms.entry(room_id.clone()).or_insert_with(Room::default);
            room.peers.push((peer_id.clone(), sender.clone()));
        }

        // Update peer map
        {
            let mut peers = self.peers.write().await;
            peers.entry(room_id.clone())
                .or_insert_with(Vec::new)
                .push((peer_id.clone(), sender));
        }

        // Get updated peer list and broadcast to all peers in the room
        let peers_read = self.peers.read().await;
        if let Some(room_peers) = peers_read.get(&room_id) {
            let peer_list = room_peers.iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<String>>();
            
            let peer_list_msg = SignalingMessage::PeerList { peers: peer_list };
            self.broadcast_to_room(&room_id, &peer_list_msg).await?;
        }

        Ok(())
    }

    async fn handle_disconnect(&self, room_id: String, peer_id: String) -> Result<()> {
        // Remove peer from room
        {
            let mut rooms = self.rooms.write().await;
            if let Some(room) = rooms.get_mut(&room_id) {
                room.peers.retain(|(id, _)| id != &peer_id);
            }
        }

        // Remove peer from peer map
        {
            let mut peers = self.peers.write().await;
            if let Some(room_peers) = peers.get_mut(&room_id) {
                room_peers.retain(|(id, _)| id != &peer_id);
            }
        }

        // Broadcast updated peer list to remaining peers in the room
        let peers_read = self.peers.read().await;
        if let Some(room_peers) = peers_read.get(&room_id) {
            let peer_list = room_peers.iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<String>>();
            
            let peer_list_msg = SignalingMessage::PeerList { peers: peer_list };
            self.broadcast_to_room(&room_id, &peer_list_msg).await?;
        }

        Ok(())
    }

    // Add other handler methods...
} 