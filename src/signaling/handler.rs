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
            SignalingMessage::CallRequest { room_id, from_peer, to_peers } => {
                println!("Handling CallRequest: from={}, to={:?}, room={}", 
                    from_peer, to_peers, room_id);
                self.handle_call_request(room_id, from_peer, to_peers).await
            },
            SignalingMessage::CallResponse { room_id, from_peer, to_peer, accepted } => {
                self.handle_call_response(room_id, from_peer, to_peer, accepted).await
            },
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
                self.handle_ice_candidate(&room_id, &candidate, &from_peer, &to_peer).await
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

    async fn handle_call_request(
        &self,
        room_id: String,
        from_peer: String,
        to_peers: Vec<String>
    ) -> Result<()> {
        println!("Entering handle_call_request: room={}, from={}, to={:?}", 
            room_id, from_peer, to_peers);
        
        let peers = self.peers.read().await;
        println!("Current peers in room: {:?}", 
            peers.get(&room_id).map(|p| p.len()).unwrap_or(0));
        
        if let Some(room_peers) = peers.get(&room_id) {
            let call_request = SignalingMessage::CallRequest {
                room_id: room_id.clone(),
                from_peer: from_peer.clone(),
                to_peers: to_peers.clone(),
            };
            
            // Send to each target peer
            for (peer_id, sender) in room_peers {
                if to_peers.contains(peer_id) {
                    println!("Attempting to send call request to peer: {}", peer_id);
                    if let Ok(msg) = serde_json::to_string(&call_request) {
                        let mut sender = sender.lock().await;
                        match sender.send(Message::Text(msg)).await {
                            Ok(_) => println!("Successfully sent call request to peer {}", peer_id),
                            Err(e) => eprintln!("Failed to send call request to peer {}: {}", peer_id, e),
                        }
                    }
                }
            }
        } else {
            println!("No peers found in room {}", room_id);
        }
        Ok(())
    }

    async fn handle_call_response(
        &self,
        room_id: String,
        from_peer: String,
        to_peer: String,
        accepted: bool,
    ) -> Result<()> {
        let peers = self.peers.read().await;
        if let Some(room_peers) = peers.get(&room_id) {
            if let Some((_, target_sender)) = room_peers.iter().find(|(id, _)| id == &to_peer) {
                let response = SignalingMessage::CallResponse {
                    room_id,
                    from_peer,
                    to_peer,
                    accepted,
                };
                
                if let Ok(msg) = serde_json::to_string(&response) {
                    let mut sender = target_sender.lock().await;
                    let _ = sender.send(Message::Text(msg)).await;
                }
            }
        }
        Ok(())
    }

    async fn handle_ice_candidate(
        &self,
        room_id: &str,
        candidate: &str,
        from_peer: &str,
        to_peer: &str
    ) -> Result<()> {
        println!("Handling ICE candidate from {} to {} in room {}", from_peer, to_peer, room_id);
        
        let peers = self.peers.read().await;
        if let Some(room_peers) = peers.get(room_id) {
            if let Some((_, target_sender)) = room_peers.iter().find(|(id, _)| id == to_peer) {
                let ice_message = SignalingMessage::IceCandidate {
                    room_id: room_id.to_string(),
                    candidate: candidate.to_string(),
                    from_peer: from_peer.to_string(),
                    to_peer: to_peer.to_string(),
                };
                
                let mut sender = target_sender.lock().await;
                match serde_json::to_string(&ice_message) {
                    Ok(msg) => {
                        if let Err(e) = sender.send(Message::Text(msg)).await {
                            println!("Failed to send ICE candidate: {}", e);
                            return Err(Error::WebSocket(e));
                        }
                        println!("Successfully forwarded ICE candidate");
                    },
                    Err(e) => {
                        println!("Failed to serialize ICE candidate: {}", e);
                        return Err(Error::Json(e));
                    }
                }
            } else {
                println!("Target peer {} not found", to_peer);
            }
        } else {
            println!("Room {} not found", room_id);
        }
        Ok(())
    }

    async fn update_connection_state(
        &self,
        room_id: &str,
        peer1: &str,
        peer2: &str,
        connected: bool
    ) -> Result<()> {
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(room_id) {
            let pair = if peer1 < peer2 {
                (peer1.to_string(), peer2.to_string())
            } else {
                (peer2.to_string(), peer1.to_string())
            };
            
            if connected {
                room.connected_pairs.insert(pair);
            } else {
                room.connected_pairs.remove(&pair);
            }
        }
        Ok(())
    }

    // Add other handler methods...
} 