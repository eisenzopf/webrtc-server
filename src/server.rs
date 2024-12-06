use crate::types::*;
use crate::room::*;
use crate::metrics::*;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
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
            let signal_msg: SignalingMessage = serde_json::from_str(msg_str)?;

            // Update metrics for the peer
            if let Some(ref peer_id) = current_peer_id {
                let mut metrics = metrics.write().await;
                metrics
                    .entry(peer_id.clone())
                    .or_insert_with(|| ConnectionMetrics::new(peer_id.clone()))
                    .last_seen = Instant::now();
            }

            match signal_msg {
                SignalingMessage::Join { room_id, peer_id } => {
                    let room = Self::get_or_create_room(&rooms, &room_id).await?;
                    if room.peers.len() >= room.media_settings.max_participants {
                        let error_msg = SignalingMessage::MediaError {
                            error_type: "room_full".to_string(),
                            description: "Room has reached maximum capacity".to_string(),
                            peer_id: peer_id.clone(),
                        };
                        let mut sender = ws_sender.lock().await;
                        sender.send(Message::Text(serde_json::to_string(&error_msg)?)).await?;
                        continue;
                    }

                    current_peer_id = Some(peer_id.clone());
                    current_room_id = Some(room_id.clone());

                    let mut peers_write = peers.write().await;
                    let room_peers = peers_write.entry(room_id.clone()).or_insert_with(Vec::new);
                    let peer_list = room_peers
                        .iter()
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>();

                    room_peers.push((peer_id.clone(), ws_sender.clone()));

                    Self::broadcast_to_room(&peers, &room_id, &SignalingMessage::PeerList {
                        peers: peer_list,
                    })
                    .await?;
                }
                SignalingMessage::MediaOffer {
                    room_id,
                    sdp,
                    from_peer,
                    to_peer,
                    media_types,
                } => {
                    let rooms = rooms.read().await;
                    if let Some(room) = rooms.get(&room_id) {
                        let allowed_types = &room.media_settings.allowed_media_types;
                        if !media_types.iter().any(|mt| allowed_types.contains(mt)) {
                            let error_msg = SignalingMessage::MediaError {
                                error_type: "unsupported_media".to_string(),
                                description: "One or more media types not allowed in this room"
                                    .to_string(),
                                peer_id: from_peer.clone(),
                            };
                            let mut sender = ws_sender.lock().await;
                            sender
                                .send(Message::Text(serde_json::to_string(&error_msg)?))
                                .await?;
                            continue;
                        }
                    }

                    Self::broadcast_to_room(
                        &peers,
                        &room_id,
                        &SignalingMessage::MediaOffer {
                            room_id: room_id.clone(),
                            sdp,
                            from_peer,
                            to_peer,
                            media_types,
                        },
                    )
                    .await?;
                }
                SignalingMessage::LeaveRoom { room_id, peer_id } => {
                    let mut peers_write = peers.write().await;
                    if let Some(room_peers) = peers_write.get_mut(&room_id) {
                        room_peers.retain(|(id, _)| id != &peer_id);
                        let peer_list = room_peers
                            .iter()
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>();
                        Self::broadcast_to_room(&peers, &room_id, &SignalingMessage::PeerList {
                            peers: peer_list,
                        })
                        .await?;
                    }
                }
                _ => {
                    // Handle other message types by broadcasting to room
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
                Self::broadcast_to_room(&peers, &room_id, &SignalingMessage::PeerList {
                    peers: peer_list,
                })
                .await?;
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