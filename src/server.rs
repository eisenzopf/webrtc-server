use crate::types::{MediaRelay, MediaSettings, Room as TypesRoom, SignalingMessage};
use crate::room::Room as RoomConfig;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use std::io::Write;
use rtp::packet::Packet as RtpPacket;
use crate::types::PeerMap;
use crate::room::Room;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use crate::metrics::ConnectionMetrics;
use tokio::sync::Mutex;
use tokio_tungstenite::accept_async;
use tokio::net::TcpStream;
use std::time::Duration;
use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio_tungstenite::tungstenite::Message;
use webrtc::track::track_local::TrackLocalWriter;
use std::any::Any;
use webrtc::util::Marshal;

#[derive(Clone)]
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

    async fn log(message: &str) {
        println!("[SERVER DEBUG] {}", message);
        let mut stdout = tokio::io::stdout();
        if let Err(e) = stdout.flush().await {
            eprintln!("Failed to flush stdout: {}", e);
        }
    }

    pub async fn start(&self) -> Result<(), anyhow::Error> {
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
            
            let server = self.clone();
            tokio::spawn(async move {
                if let Err(e) = server.handle_connection(stream, peers, rooms, metrics).await {
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
        &self,
        stream: TcpStream,
        peers: PeerMap,
        rooms: Arc<RwLock<HashMap<String, Room>>>,
        metrics: Arc<RwLock<HashMap<String, ConnectionMetrics>>>,
    ) -> Result<(), anyhow::Error> {
        Self::log("Starting new WebSocket connection handling").await;
        let ws_stream = accept_async(stream).await?;
        Self::log("WebSocket connection accepted").await;
        
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(Mutex::new(ws_sender));
        let mut current_peer_id: Option<String> = None;
        let mut current_room_id: Option<String> = None;

        // Create a heartbeat channel
        let (heartbeat_tx, mut heartbeat_rx) = tokio::sync::mpsc::channel::<()>(1);
        let heartbeat_ws_sender = ws_sender.clone();
        
        // Spawn heartbeat task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let mut sender = heartbeat_ws_sender.lock().await;
                if sender.send(Message::Ping(vec![])).await.is_err() {
                    let _ = heartbeat_tx.send(()).await;
                    break;
                }
            }
        });

        while let Some(msg) = tokio::select! {
            msg = ws_receiver.next() => msg,
            _ = heartbeat_rx.recv() => {
                // Heartbeat failed, clean up and exit
                Self::cleanup_peer(&current_peer_id, &current_room_id, &peers, &rooms).await?;
                return Ok(());
            }
        } {
            Self::log("------- New message received -------").await;
            match msg {
                Ok(msg) => {
                    match msg.to_text() {
                        Ok(msg_str) => {
                            // Add check for empty messages
                            if msg_str.trim().is_empty() {
                                Self::log("Received empty message - ignoring").await;
                                continue;
                            }

                            Self::log(&format!("Raw message received: {}", msg_str)).await;
                            match serde_json::from_str::<SignalingMessage>(msg_str) {
                                Ok(signal_msg) => {
                                    Self::log(&format!("Successfully parsed message: {:?}", signal_msg)).await;
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
                                            Self::log("=== JOIN REQUEST START ===").await;
                                            Self::log(&format!("Received join request - Room: {}, Peer: {}", room_id, peer_id)).await;
                                            
                                            // Store the current peer's information
                                            current_peer_id = Some(peer_id.clone());
                                            current_room_id = Some(room_id.clone());
                                            
                                            // Clone the Arc<Mutex> before locking
                                            let peer_sender = ws_sender.clone();
                                            
                                            // Add peer to room and get updated peer list
                                            let peer_list = {
                                                let mut rooms = rooms.write().await;
                                                let room = rooms.entry(room_id.clone()).or_insert_with(|| Room::default());
                                                room.id = room_id.clone();
                                                
                                                // Add the new peer
                                                room.peers.push((peer_id.clone(), peer_sender));
                                                
                                                // Get list of all peer IDs in the room
                                                room.peers.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()
                                            };

                                            Self::log(&format!("Current peers in room: {:?}", peer_list)).await;
                                            
                                            // Update the peers map
                                            {
                                                let mut peers_write = peers.write().await;
                                                peers_write
                                                    .entry(room_id.clone())
                                                    .or_insert_with(Vec::new)
                                                    .push((peer_id.clone(), ws_sender.clone()));
                                            }
                                            
                                            // Broadcast the updated peer list to all peers in the room
                                            let peer_list_msg = SignalingMessage::PeerList {
                                                peers: peer_list.clone(),
                                            };
                                            
                                            // Broadcast to all peers including the new one
                                            Self::broadcast_to_room(&peers, &room_id, &peer_list_msg).await?;
                                            
                                            Self::log("=== JOIN REQUEST END ===").await;
                                        },
                                        SignalingMessage::Offer { room_id, sdp, from_peer, to_peer } => {
                                            let rooms = self.rooms.read().await;
                                            if let Some(room) = rooms.get(&room_id) {
                                                // Forward the offer to the target peer
                                                for (peer_id, sender) in &room.peers {
                                                    if peer_id == &to_peer {
                                                        let mut sender = sender.lock().await;
                                                        sender.send(Message::Text(serde_json::to_string(&SignalingMessage::Offer {
                                                            room_id: room_id.clone(),
                                                            sdp,
                                                            from_peer,
                                                            to_peer,
                                                        })?)).await?;
                                                        break;
                                                    }
                                                }
                                            }
                                        },
                                        SignalingMessage::Answer { room_id, sdp, from_peer, to_peer } => {
                                            let rooms = self.rooms.read().await;
                                            if let Some(room) = rooms.get(&room_id) {
                                                // Forward the answer only to the peer that sent the offer
                                                for (peer_id, sender) in &room.peers {
                                                    if peer_id == &to_peer {
                                                        let mut sender = sender.lock().await;
                                                        sender.send(Message::Text(serde_json::to_string(&SignalingMessage::Answer {
                                                            room_id: room_id.clone(),
                                                            sdp,
                                                            from_peer,
                                                            to_peer,
                                                        })?)).await?;
                                                        break;
                                                    }
                                                }
                                            }
                                        },
                                        SignalingMessage::IceCandidate { ref room_id, ref candidate, ref from_peer, ref to_peer } => {
                                            Self::broadcast_to_room(&peers, &room_id, &signal_msg).await?;
                                        },
                                        SignalingMessage::Disconnect { room_id, peer_id } => {
                                            Self::log(&format!("=== DISCONNECT REQUEST START ===")).await;
                                            Self::cleanup_peer(&Some(peer_id), &Some(room_id), &peers, &rooms).await?;
                                            Self::log("=== DISCONNECT REQUEST END ===").await;
                                            
                                            // Clear current peer state
                                            current_peer_id = None;
                                            current_room_id = None;
                                        },
                                        _ => {
                                            Self::log(&format!("Received unhandled message type: {:?}", signal_msg)).await;
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

        // Clean up when connection closes
        Self::cleanup_peer(&current_peer_id, &current_room_id, &peers, &rooms).await?;
        Ok(())
    }

    async fn cleanup_peer(
        peer_id: &Option<String>,
        room_id: &Option<String>,
        peers: &PeerMap,
        rooms: &Arc<RwLock<HashMap<String, Room>>>,
    ) -> Result<(), anyhow::Error> {
        if let (Some(peer_id), Some(room_id)) = (peer_id, room_id) {
            Self::log(&format!("Cleaning up peer {} from room {}", peer_id, room_id)).await;
            
            // Remove from rooms
            {
                let mut rooms = rooms.write().await;
                if let Some(room) = rooms.get_mut(room_id) {
                    room.peers.retain(|(id, _)| id != peer_id);
                    
                    // Remove empty room
                    if room.peers.is_empty() {
                        rooms.remove(room_id);
                    }
                }
            }

            // Remove from peers and broadcast update
            {
                let mut peers_write = peers.write().await;
                if let Some(room_peers) = peers_write.get_mut(room_id) {
                    room_peers.retain(|(id, _)| id != peer_id);
                    
                    // Get updated peer list
                    let peer_list = room_peers
                        .iter()
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>();

                    // Broadcast update
                    let peer_list_msg = SignalingMessage::PeerList {
                        peers: peer_list,
                    };
                    Self::broadcast_to_room(peers, room_id, &peer_list_msg).await?;
                }
            }
        }
        Ok(())
    }

    async fn broadcast_to_room(
        peers: &PeerMap,
        room_id: &str,
        message: &SignalingMessage,
    ) -> anyhow::Result<()> {
        let peers_read = peers.read().await;
        if let Some(room_peers) = peers_read.get(room_id) {
            Self::log(&format!("Broadcasting to {} peers in room {}", room_peers.len(), room_id)).await;
            let msg = Message::Text(serde_json::to_string(message)?);
            for (peer_id, sender) in room_peers {
                Self::log(&format!("Sending to peer: {}", peer_id)).await;
                let mut guard = sender.lock().await;
                if let Err(e) = guard.send(msg.clone()).await {
                    eprintln!("Error broadcasting message to peer {}: {}", peer_id, e);
                }
            }
        } else {
            Self::log(&format!("No peers found in room {}", room_id)).await;
        }
        Ok(())
    }

    async fn get_or_create_room(
        rooms: &Arc<RwLock<HashMap<String, Room>>>,
        room_id: &str,
    ) -> Result<Room, anyhow::Error> {
        let mut rooms = rooms.write().await;
        if !rooms.contains_key(room_id) {
            rooms.insert(
                room_id.to_string(),
                Room {
                    id: room_id.to_string(),
                    peers: Vec::new(),
                    media_settings: MediaSettings::default(),
                    media_relays: HashMap::new(),
                    recording_enabled: false,
                },
            );
        }
        Ok(rooms.get(room_id).unwrap().clone())
    }

    async fn setup_media_relay(&self, room_id: &str, peer_id: &str) -> Result<MediaRelay, anyhow::Error> {
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let api = APIBuilder::new()
            .with_media_engine(m)
            .build();

        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);
        let pc = Arc::clone(&peer_connection);
        
        // Clone peer_id before moving into closure
        let peer_id_owned = peer_id.to_string();

        // Create an audio track for this peer
        let audio_track = Arc::new(TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: "audio/opus".to_owned(),
                ..Default::default()
            },
            format!("audio-{}", peer_id),
            format!("audio-stream-{}", peer_id),
        ));

        // Add the track to the peer connection
        peer_connection.add_track(audio_track.clone()).await?;

        // Set up handlers for the peer connection
        peer_connection.on_track(Box::new(move |track, _, _| {
            println!("Received track from peer: {}", peer_id_owned);
            let pc = pc.clone();
            Box::pin(async move {
                while let Ok((rtp, _)) = track.read_rtp().await {
                    // Forward the RTP packet to other peers
                    if let Some(sender) = pc.get_senders().await.get(0) {
                        if let Some(track) = sender.track().await {
                            if let Some(writer) = track.as_any().downcast_ref::<TrackLocalStaticRTP>() {
                                let _ = writer.write_rtp(&rtp).await;
                            }
                        }
                    }
                }
            })
        }));

        Ok(MediaRelay {
            peer_connection,
            audio_track: Some(audio_track),
            peer_id: peer_id.to_string(),
        })
    }

    async fn handle_media_packet(&self, room_id: &str, from_peer: &str, packet: RtpPacket) -> Result<(), anyhow::Error> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            // Serialize the RTP packet to bytes
            let packet_bytes = packet.marshal()?;
            
            // Forward the packet to all other peers in the room
            for (peer_id, relay) in &room.media_relays {
                if peer_id != from_peer {
                    if let Some(track) = &relay.audio_track {
                        track.write(&packet_bytes).await?;
                    }
                }
            }

            // If recording is enabled, save the packet
            if room.recording_enabled {
                self.record_audio(room_id, &packet).await?;
            }
        }
        Ok(())
    }

    async fn record_audio(&self, room_id: &str, packet: &RtpPacket) -> Result<(), anyhow::Error> {
        // Create recordings directory if it doesn't exist
        std::fs::create_dir_all("recordings")?;
        
        let recording_path = format!("recordings/{}.raw", room_id);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(recording_path)?;
        
        file.write_all(&packet.payload)?;
        Ok(())
    }

    async fn broadcast_peer_list(&self, room_id: &str, peer_list: &[String]) -> anyhow::Result<()> {
        let peer_list_msg = SignalingMessage::PeerList {
            peers: peer_list.to_vec(),
        };
        
        Self::broadcast_to_room(&self.peers, room_id, &peer_list_msg).await
    }
}
