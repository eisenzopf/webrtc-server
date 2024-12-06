use crate::types::*;
use crate::room::*;
use crate::metrics::*;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use std::io::{self, Write};

pub struct SignalingServer {
    address: String,
    peers: PeerMap,
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    metrics: Arc<RwLock<HashMap<String, ConnectionMetrics>>>,
}

impl SignalingServer {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            peers: Arc::new(RwLock::new(HashMap::new())),
            rooms: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn log(message: &str) {
        println!("[SERVER DEBUG] {}", message);
        io::stdout().flush().unwrap_or_default();
    }

    pub async fn start(&self) -> Result<()> {
        println!("### SERVER START ###");
        println!("Attempting to bind to {}", self.address);
        let listener = TcpListener::bind(&self.address).await.map_err(|e| {
            eprintln!("Failed to bind to {}: {}", self.address, e);
            anyhow::anyhow!("Failed to bind: {}", e)
        })?;
        println!("### Successfully bound to {} ###", self.address);
        println!("### Signaling server listening on {} ###", self.address);

        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            loop {
                Self::cleanup_stale_metrics(&metrics).await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });

        while let Ok((stream, addr)) = listener.accept().await {
            println!("### New connection from: {} ###", addr);
            let peers = self.peers.clone();
            let rooms = self.rooms.clone();
            let metrics = self.metrics.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, peers, rooms, metrics).await {
                    println!("### Error handling connection: {} ###", e);
                }
            });
        }

        Ok(())
    }

    async fn cleanup_stale_metrics(metrics: &Arc<RwLock<HashMap<String, ConnectionMetrics>>>) {
        let mut metrics = metrics.write().await;
        metrics.retain(|_, v| v.last_seen.elapsed() < Duration::from_secs(300));
    }

    async fn handle_connection(
        stream: TcpStream,
        peers: PeerMap,
        rooms: Arc<RwLock<HashMap<String, Room>>>,
        metrics: Arc<RwLock<HashMap<String, ConnectionMetrics>>>,
    ) -> Result<()> {
        Self::log("Starting new WebSocket connection handling");
        let ws_stream = accept_async(stream).await?;
        Self::log("WebSocket connection accepted");
        
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(Mutex::new(ws_sender));
        let mut current_peer_id: Option<String> = None;
        let mut current_room_id: Option<String> = None;

        while let Some(msg) = ws_receiver.next().await {
            Self::log("------- New message received -------");
            match msg {
                Ok(msg) => {
                    match msg.to_text() {
                        Ok(msg_str) => {
                            Self::log(&format!("Raw message received: {}", msg_str));
                            match serde_json::from_str::<SignalingMessage>(msg_str) {
                                Ok(signal_msg) => {
                                    Self::log(&format!("Successfully parsed message: {:?}", signal_msg));
                                    // Update metrics for the peer
                                    if let Some(ref peer_id) = current_peer_id {
                                        let mut metrics = metrics.write().await;
                                        metrics
                                            .entry(peer_id.clone())
                                            .or_insert_with(|| ConnectionMetrics::new(peer_id.clone()))
                                            .update_last_seen();
                                    }

                                    match signal_msg {
                                        SignalingMessage::Join { room_id, peer_id } => {
                                            Self::log(&format!("=== JOIN REQUEST START ==="));
                                            Self::log(&format!("Received join request - Room: {}, Peer: {}", room_id, peer_id));
                                            
                                            // Get or create room and update its peer list
                                            {
                                                let mut rooms = rooms.write().await;
                                                let room = rooms.entry(room_id.clone())
                                                    .or_insert_with(|| Room {
                                                        id: room_id.clone(),
                                                        peers: Vec::new(),
                                                        media_settings: MediaSettings::default(),
                                                    });
                                                
                                                // Add peer to room's peer list if not already present
                                                if !room.peers.iter().any(|(id, _)| id == &peer_id) {
                                                    room.peers.push((peer_id.clone(), ws_sender.clone()));
                                                }
                                                Self::log(&format!("Room status - Current peers: {}", room.peers.len()));
                                            }

                                            current_peer_id = Some(peer_id.clone());
                                            current_room_id = Some(room_id.clone());
                                            Self::log(&format!("=== JOIN REQUEST END ==="));

                                            // Update peer map and broadcast to all peers
                                            {
                                                let mut peers_write = peers.write().await;
                                                let room_peers = peers_write.entry(room_id.clone()).or_insert_with(Vec::new);
                                                
                                                // Add the new peer if not already present
                                                if !room_peers.iter().any(|(id, _)| id == &peer_id) {
                                                    room_peers.push((peer_id.clone(), ws_sender.clone()));
                                                }

                                                // Get updated peer list
                                                let updated_peer_list = room_peers
                                                    .iter()
                                                    .map(|(id, _)| id.clone())
                                                    .collect::<Vec<_>>();

                                                Self::log(&format!("Current peers in room: {:?}", updated_peer_list));
                                                Self::log("Creating peer list message...");

                                                let peer_list_msg = SignalingMessage::PeerList {
                                                    peers: updated_peer_list.clone(),
                                                };

                                                // First send to the new peer
                                                {
                                                    let mut sender = ws_sender.lock().await;
                                                    Self::log(&format!("Sending direct message to new peer {}", peer_id));
                                                    sender.send(Message::Text(serde_json::to_string(&peer_list_msg)?)).await?;
                                                }

                                                // Then broadcast to all peers in the room
                                                Self::log(&format!("Broadcasting updated peer list to all peers in room {}", room_id));
                                                for (peer_id, sender) in room_peers.iter() {
                                                    let mut sender = sender.lock().await;
                                                    Self::log(&format!("Sending peer list to {}", peer_id));
                                                    sender.send(Message::Text(serde_json::to_string(&peer_list_msg)?)).await?;
                                                }
                                            }
                                        },
                                        SignalingMessage::Offer { ref room_id, ref sdp, ref from_peer, ref to_peer } => {
                                            Self::broadcast_to_room(&peers, &room_id, &signal_msg).await?;
                                        },
                                        SignalingMessage::Answer { ref room_id, ref sdp, ref from_peer, ref to_peer } => {
                                            Self::broadcast_to_room(&peers, &room_id, &signal_msg).await?;
                                        },
                                        SignalingMessage::IceCandidate { ref room_id, ref candidate, ref from_peer, ref to_peer } => {
                                            Self::broadcast_to_room(&peers, &room_id, &signal_msg).await?;
                                        },
                                        SignalingMessage::Disconnect { room_id, peer_id } => {
                                            Self::log(&format!("=== DISCONNECT REQUEST START ==="));
                                            Self::log(&format!("Received disconnect request - Room: {}, Peer: {}", room_id, peer_id));
                                            
                                            // Clean up room and peer mappings
                                            {
                                                let mut rooms = rooms.write().await;
                                                if let Some(room) = rooms.get_mut(&room_id) {
                                                    room.peers.retain(|(id, _)| id != &peer_id);
                                                    Self::log(&format!("Updated room peers - Remaining: {}", room.peers.len()));
                                                    
                                                    // Remove room if empty
                                                    if room.peers.is_empty() {
                                                        rooms.remove(&room_id);
                                                        Self::log("Room removed as it's now empty");
                                                    }
                                                }
                                            }
                                            
                                            // Update peer map
                                            {
                                                let mut peers_write = peers.write().await;
                                                if let Some(room_peers) = peers_write.get_mut(&room_id) {
                                                    room_peers.retain(|(id, _)| id != &peer_id);
                                                    
                                                    // Broadcast updated peer list to remaining peers
                                                    let updated_peer_list = room_peers
                                                        .iter()
                                                        .map(|(id, _)| id.clone())
                                                        .collect::<Vec<_>>();
                                                    
                                                    let peer_list_msg = SignalingMessage::PeerList {
                                                        peers: updated_peer_list.clone(),
                                                    };
                                                    
                                                    Self::log(&format!("Broadcasting updated peer list: {:?}", updated_peer_list));
                                                    Self::broadcast_to_room(&peers, &room_id, &peer_list_msg).await?;
                                                }
                                            }
                                            
                                            Self::log("=== DISCONNECT REQUEST END ===");
                                        },
                                        _ => {
                                            Self::log(&format!("Received unhandled message type: {:?}", signal_msg));
                                            if let Some(ref room_id) = current_room_id {
                                                Self::broadcast_to_room(&peers, room_id, &signal_msg).await?;
                                            }
                                        }
                                    }
                                },
                                Err(e) => {
                                    eprintln!("Failed to parse message as SignalingMessage: {}", e);
                                    eprintln!("Problematic message: {}", msg_str);
                                }
                            }
                        },
                        Err(e) => {
                            eprintln!("Failed to convert message to text: {}", e);
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Error receiving WebSocket message: {}", e);
                }
            }
        }

        Self::log("WebSocket connection closed");
        // Cleanup when connection closes
        if let (Some(room_id), Some(peer_id)) = (current_room_id, current_peer_id) {
            let mut peers_write = peers.write().await;
            if let Some(room_peers) = peers_write.get_mut(&room_id) {
                room_peers.retain(|(id, _)| id != &peer_id);
                let peer_list = room_peers
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();

                let peer_list_msg = SignalingMessage::PeerList {
                    peers: peer_list,
                };

                Self::broadcast_to_room(&peers, &room_id, &peer_list_msg).await?;
            }
        }

        Ok(())
    }

    async fn broadcast_to_room(
        peers: &PeerMap,
        room_id: &str,
        message: &SignalingMessage,
    ) -> Result<()> {
        let peers_read = peers.read().await;
        if let Some(room_peers) = peers_read.get(room_id) {
            Self::log(&format!("Broadcasting to {} peers in room {}", room_peers.len(), room_id));
            let msg = Message::Text(serde_json::to_string(message)?);
            for (peer_id, sender) in room_peers {
                Self::log(&format!("Sending to peer: {}", peer_id));
                let mut guard = sender.lock().await;
                if let Err(e) = guard.send(msg.clone()).await {
                    eprintln!("Error broadcasting message to peer {}: {}", peer_id, e);
                }
            }
        } else {
            Self::log(&format!("No peers found in room {}", room_id));
        }
        Ok(())
    }

    async fn get_or_create_room(
        rooms: &Arc<RwLock<HashMap<String, Room>>>,
        room_id: &str,
    ) -> Result<Room> {
        let mut rooms = rooms.write().await;
        if !rooms.contains_key(room_id) {
            rooms.insert(
                room_id.to_string(),
                Room {
                    id: room_id.to_string(),
                    peers: Vec::new(),
                    media_settings: MediaSettings::default(),
                },
            );
        }
        Ok(rooms.get(room_id).unwrap().clone())
    }
}