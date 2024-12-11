use crate::utils::{Error, Result};
use crate::room::Room;
use crate::metrics::ConnectionMetrics;
use crate::signaling::messages::SignalingMessage;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use std::collections::HashMap;
use tokio_tungstenite::tungstenite::Message;
use futures_util::SinkExt;
use webrtc::{
    api::{
        media_engine::MediaEngine,
        APIBuilder,
    },
    peer_connection::{
        configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        RTCPeerConnection,
        sdp::session_description::RTCSessionDescription,
    },
    ice_transport::ice_server::RTCIceServer,
    rtp_transceiver::{
        rtp_codec::RTCRtpCodecCapability,
        rtp_receiver::RTCRtpReceiver,
        RTCRtpTransceiver,
    },
    track::{
        track_remote::TrackRemote,
        track_local::track_local_static_rtp::TrackLocalStaticRTP,
    },
};
use crate::media::relay::MediaRelay;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::track::track_local::TrackLocal;

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
                    let end_call_msg = SignalingMessage::EndCall {
                        room_id: room_id.clone(),
                        peer_id: peer_id.clone()
                    };
                    
                    if let Ok(msg) = serde_json::to_string(&end_call_msg) {
                        for (other_peer_id, sender) in room_peers {
                            if other_peer_id != &peer_id {
                                let mut sender = sender.lock().await;
                                let _ = sender.send(Message::Text(msg.clone())).await;
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
        // Initialize WebRTC
        let peer_connection = self.create_peer_connection(&peer_id).await?;
        
        // Set up media handlers
        self.setup_media_handlers(peer_connection.clone(), &peer_id, &room_id).await?;
        
        // Store in room
        {
            let mut rooms = self.rooms.write().await;
            if let Some(room) = rooms.get_mut(&room_id) {
                room.media_relays.insert(peer_id.clone(), MediaRelay {
                    peer_connection: peer_connection.clone(),
                    audio_track: None,
                    peer_id: peer_id.clone(),
                });
            }
        }

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
        let mut rooms = self.rooms.write().await;
        if let Some(room) = rooms.get_mut(&room_id) {
            // Set up media tracks for the calling peer
            if let Some(relay) = room.media_relays.get(&from_peer) {
                // Create and add audio track
                let track_local: Arc<TrackLocalStaticRTP> = Arc::new(TrackLocalStaticRTP::new(
                    RTCRtpCodecCapability {
                        mime_type: "audio/opus".to_owned(),
                        ..Default::default()
                    },
                    format!("audio_{}", from_peer),
                    "webrtc-rs".to_owned(),
                ));

                let track = relay.peer_connection.add_track(track_local.clone()).await?;

                // Store track reference
                room.media_tracks.entry(from_peer.clone())
                    .or_insert_with(Vec::new)
                    .push(track_local);
            }
        }
        
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
        to_peer: &str,
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

    async fn setup_peer_connection_handlers(
        peer_connection: Arc<RTCPeerConnection>,
        peer_id: String,
        room_id: String,
        peers: PeerMap,
    ) {
        let pc_clone = peer_connection.clone();
        let peer_id_clone = peer_id.clone();
        let room_id_clone = room_id.clone();
        let peers_clone = peers.clone();

        peer_connection.on_track(Box::new(move |track: Arc<TrackRemote>, 
                                       receiver: Arc<RTCRtpReceiver>, 
                                       transceiver: Arc<RTCRtpTransceiver>| {
            let pc = pc_clone.clone();
            let peer_id = peer_id_clone.clone();
            let room_id = room_id_clone.clone();
            let peers = peers_clone.clone();

            Box::pin(async move {
                // Forward track to other peers in room
                let peers_read = peers.read().await;
                if let Some(room_peers) = peers_read.get(&room_id) {
                    for (other_peer_id, _) in room_peers {
                        if other_peer_id != &peer_id {
                            // Create new track for forwarding
                            let local_track = TrackLocalStaticRTP::new(
                                track.codec().capability,
                                format!("forwarded_{}", track.id()),
                                "webrtc-rs".to_owned(),
                            );
                            let track_local = Arc::new(local_track);
                            if let Ok(rtp_sender) = pc.add_track(track_local.clone()).await {
                                // Store track if needed
                            }
                        }
                    }
                }
            })
        }));
    }

    async fn create_peer_connection(&self, peer_id: &str) -> Result<Arc<RTCPeerConnection>> {
        let config = RTCConfiguration {
            ice_servers: vec![
                RTCIceServer {
                    urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                    ..Default::default()
                }
            ],
            ..Default::default()
        };

        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let api = APIBuilder::new()
            .with_media_engine(m)
            .build();

        Ok(Arc::new(api.new_peer_connection(config).await?))
    }

    async fn setup_media_handlers(
        &self,
        peer_connection: Arc<RTCPeerConnection>,
        peer_id: &str,
        room_id: &str,
    ) -> Result<()> {
        let pc_clone = peer_connection.clone();
        let peer_id_clone = peer_id.to_string();
        let room_id_clone = room_id.to_string();
        let peers = self.peers.clone();

        peer_connection.on_track(Box::new(move |track: Arc<TrackRemote>, 
                                       receiver: Arc<RTCRtpReceiver>, 
                                       transceiver: Arc<RTCRtpTransceiver>| {
            let pc = pc_clone.clone();
            let peer_id = peer_id_clone.clone();
            let room_id = room_id_clone.clone();
            let peers = peers.clone();

            Box::pin(async move {
                // Forward track to other peers in room
                let peers_read = peers.read().await;
                if let Some(room_peers) = peers_read.get(&room_id) {
                    for (other_peer_id, _) in room_peers {
                        if other_peer_id != &peer_id {
                            // Create new track for forwarding
                            let local_track = TrackLocalStaticRTP::new(
                                track.codec().capability,
                                format!("forwarded_{}", track.id()),
                                "webrtc-rs".to_owned(),
                            );
                            let track_local = Arc::new(local_track);
                            if let Ok(rtp_sender) = pc.add_track(track_local.clone()).await {
                                // Store track if needed
                            }
                        }
                    }
                }
            })
        }));

        // Handle connection state changes
        let peer_id_clone = peer_id.to_string();
        peer_connection.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
            let peer_id = peer_id_clone.clone();
            Box::pin(async move {
                match state {
                    RTCPeerConnectionState::Failed => {
                        log::error!("Peer connection failed for {}", peer_id);
                        // Implement reconnection logic here
                    },
                    RTCPeerConnectionState::Disconnected => {
                        log::warn!("Peer {} disconnected", peer_id);
                    },
                    RTCPeerConnectionState::Connected => {
                        log::info!("Peer {} connected", peer_id);
                    },
                    _ => {}
                }
            })
        }));

        Ok(())
    }

    async fn handle_offer(
        &self,
        room_id: &str,
        sdp: &str,
        from_peer: &str,
        to_peer: &str,
    ) -> Result<()> {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(room_id) {
            if let Some(relay) = room.media_relays.get(to_peer) {
                let desc = RTCSessionDescription::offer(sdp.to_owned())?;
                relay.peer_connection.set_remote_description(desc).await?;

                // Create answer
                let answer = relay.peer_connection.create_answer(None).await?;
                relay.peer_connection.set_local_description(answer.clone()).await?;

                // Send answer back
                let answer_msg = SignalingMessage::Answer {
                    room_id: room_id.to_owned(),
                    sdp: answer.sdp,
                    from_peer: to_peer.to_owned(),
                    to_peer: from_peer.to_owned(),
                };

                self.send_to_peer(from_peer, &answer_msg).await?;
            }
        }
        Ok(())
    }

    async fn send_to_peer(&self, peer_id: &str, message: &SignalingMessage) -> Result<()> {
        let peers = self.peers.read().await;
        for (room_peers) in peers.values() {
            if let Some((_, sender)) = room_peers.iter().find(|(id, _)| id == peer_id) {
                let mut sender = sender.lock().await;
                let msg = serde_json::to_string(message)?;
                sender.send(Message::Text(msg)).await
                    .map_err(|e| Error::WebSocket(e))?;
                return Ok(());
            }
        }
        Err(Error::Peer(format!("Peer {} not found", peer_id)))
    }

    // Add other handler methods...
} 