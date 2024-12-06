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

    pub async fn start(&self) -> Result<()> {
        println!("Attempting to bind to {}", self.address);
        let listener = TcpListener::bind(&self.address).await.map_err(|e| {
            eprintln!("Failed to bind to {}: {}", self.address, e);
            anyhow::anyhow!("Failed to bind: {}", e)
        })?;
        println!("Successfully bound to {}", self.address);
        println!("Signaling server listening on {}", self.address);

        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            loop {
                Self::cleanup_stale_metrics(&metrics).await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });

        while let Ok((stream, addr)) = listener.accept().await {
            println!("New connection from: {}", addr);
            let peers = self.peers.clone();
            let rooms = self.rooms.clone();
            let metrics = self.metrics.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, peers, rooms, metrics).await {
                    eprintln!("Error handling connection: {}", e);
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
        let ws_stream = accept_async(stream).await?;
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(Mutex::new(ws_sender));
        let mut current_peer_id: Option<String> = None;
        let mut current_room_id: Option<String> = None;

        while let Some(msg) = ws_receiver.next().await {
            let msg = msg?;
            let msg_str = msg.to_text()?;
            println!("Received message: {}", msg_str);
            let signal_msg: SignalingMessage = serde_json::from_str(msg_str)?;

            // Update metrics for the peer
            if let Some(ref peer_id) = current_peer_id {
                let mut metrics = metrics.write().await;
                metrics
                    .entry(peer_id.clone())
                    .or_insert_with(|| ConnectionMetrics::new(peer_id.clone()))
                    .update_last_seen();
            }

            match signal_msg.message_type.as_str() {
                "Join" => {
                    if let (Some(room_id), Some(peer_id)) = (signal_msg.room_id, signal_msg.peer_id) {
                        println!("Received join request - Room: {}, Peer: {}", room_id, peer_id);
                        let room = Self::get_or_create_room(&rooms, &room_id).await?;
                        if room.peers.len() >= room.media_settings.max_participants {
                            let error_msg = SignalingMessage {
                                message_type: "MediaError".to_string(),
                                error_type: Some("room_full".to_string()),
                                description: Some("Room has reached maximum capacity".to_string()),
                                peer_id: Some(peer_id.clone()),
                                ..Default::default()
                            };
                            let mut sender = ws_sender.lock().await;
                            sender.send(Message::Text(serde_json::to_string(&error_msg)?)).await?;
                            continue;
                        }

                        current_peer_id = Some(peer_id.clone());
                        current_room_id = Some(room_id.clone());

                        let mut peers_write = peers.write().await;
                        let room_peers = peers_write.entry(room_id.clone()).or_insert_with(Vec::new);
                        
                        // Get list of peer IDs before adding the new peer
                        let peer_list = room_peers
                            .iter()
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>();

                        // Add the new peer
                        room_peers.push((peer_id.clone(), ws_sender.clone()));

                        // Create peer list message with all peers (including the new one)
                        let updated_peer_list = room_peers
                            .iter()
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>();

                        println!("Current peers in room: {:?}", updated_peer_list);

                        let peer_list_msg = SignalingMessage {
                            message_type: "PeerList".to_string(),
                            peers: Some(updated_peer_list),
                            room_id: Some(room_id.clone()),
                            ..Default::default()
                        };

                        println!("Sending peer list message: {}", serde_json::to_string(&peer_list_msg)?);

                        // Send directly to the new peer first
                        {
                            let mut sender = ws_sender.lock().await;
                            sender.send(Message::Text(serde_json::to_string(&peer_list_msg)?)).await?;
                        }

                        // Then broadcast to all peers
                        Self::broadcast_to_room(&peers, &room_id, &peer_list_msg).await?;
                    }
                }
                "MediaOffer" => {
                    if let (Some(room_id), Some(from_peer), Some(to_peer), Some(sdp), Some(media_types)) = (
                        signal_msg.room_id.as_ref(),
                        signal_msg.from_peer.as_ref(),
                        signal_msg.to_peer.as_ref(),
                        signal_msg.sdp.as_ref(),
                        signal_msg.media_types.as_ref(),
                    ) {
                        Self::broadcast_to_room(
                            &peers,
                            &room_id,
                            &signal_msg,
                        ).await?;
                    }
                }
                "MediaAnswer" => {
                    if let (Some(room_id), Some(from_peer), Some(to_peer), Some(sdp)) = (
                        signal_msg.room_id.as_ref(),
                        signal_msg.from_peer.as_ref(),
                        signal_msg.to_peer.as_ref(),
                        signal_msg.sdp.as_ref(),
                    ) {
                        Self::broadcast_to_room(
                            &peers,
                            &room_id,
                            &signal_msg,
                        ).await?;
                    }
                }
                "IceCandidate" => {
                    if let (Some(room_id), Some(from_peer), Some(to_peer), Some(candidate)) = (
                        signal_msg.room_id.as_ref(),
                        signal_msg.from_peer.as_ref(),
                        signal_msg.to_peer.as_ref(),
                        signal_msg.candidate.as_ref(),
                    ) {
                        Self::broadcast_to_room(
                            &peers,
                            &room_id,
                            &signal_msg,
                        ).await?;
                    }
                }
                _ => {
                    println!("Received unhandled message type: {}", signal_msg.message_type);
                    // Optionally broadcast unhandled messages to room
                    if let Some(ref room_id) = current_room_id {
                        Self::broadcast_to_room(&peers, room_id, &signal_msg).await?;
                    }
                }
            }
        }

        // Cleanup when connection closes
        if let (Some(room_id), Some(peer_id)) = (current_room_id, current_peer_id) {
            let mut peers_write = peers.write().await;
            if let Some(room_peers) = peers_write.get_mut(&room_id) {
                room_peers.retain(|(id, _)| id != &peer_id);
                let peer_list = room_peers
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();

                let peer_list_msg = SignalingMessage {
                    message_type: "PeerList".to_string(),
                    peers: Some(peer_list),
                    ..Default::default()
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
            let msg = Message::Text(serde_json::to_string(message)?);
            for (_, sender) in room_peers {
                let mut guard = sender.lock().await;
                if let Err(e) = guard.send(msg.clone()).await {
                    eprintln!("Error broadcasting message: {}", e);
                }
            }
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