use crate::utils::{Error, Result};
use std::sync::Arc;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use log::{debug, info, warn, error};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use webrtc::stats::StatsReportType;
use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;
use webrtc::peer_connection::policy::bundle_policy::RTCBundlePolicy;
use webrtc::peer_connection::policy::rtcp_mux_policy::RTCRtcpMuxPolicy;
use crate::types::SignalingMessage;
use bytes::Bytes;
use tokio::sync::Mutex;
use webrtc::data_channel::RTCDataChannel;
use std::fmt;

pub trait SignalingHandler {
    fn send_to_peer(&self, peer_id: &str, message: &SignalingMessage) -> impl std::future::Future<Output = Result<()>> + Send;
}

#[derive(Clone)]
pub struct MediaRelay {
    pub peer_connection: Arc<RTCPeerConnection>,
    pub audio_track: Option<Arc<TrackLocalStaticRTP>>,
    pub video_track: Option<Arc<TrackLocalStaticRTP>>,
    pub peer_id: String,
    data_channel: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
}

pub struct MediaRelayManager {
    relays: Arc<tokio::sync::RwLock<std::collections::HashMap<String, MediaRelay>>>,
    turn_config: Option<RTCIceServer>,
}

#[derive(Debug, Clone)]
pub struct MediaStats {
    pub packets_received: u64,
    pub packets_sent: u64,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub last_updated: Instant,
}

impl MediaRelay {
    pub fn new(peer_connection: Arc<RTCPeerConnection>, peer_id: String) -> Self {
        Self {
            peer_connection,
            audio_track: None,
            video_track: None,
            peer_id,
            data_channel: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn get_stats(&self) -> Result<MediaStats> {
        let stats = self.peer_connection.get_stats().await;

        let mut media_stats = MediaStats {
            packets_received: 0,
            packets_sent: 0,
            bytes_received: 0,
            bytes_sent: 0,
            last_updated: Instant::now(),
        };

        for stat in stats.reports.iter() {
            match &stat.1 {
                StatsReportType::InboundRTP(inbound) => {
                    media_stats.packets_received += inbound.packets_received;
                    media_stats.bytes_received += inbound.bytes_received;
                }
                StatsReportType::OutboundRTP(outbound) => {
                    media_stats.packets_sent += outbound.packets_sent;
                    media_stats.bytes_sent += outbound.bytes_sent;
                }
                _ => continue,
            }
        }

        Ok(media_stats)
    }

    pub async fn send_signal(&self, message: &SignalingMessage) -> Result<()> {
        if let Ok(msg_str) = serde_json::to_string(message) {
            let data = Bytes::from(msg_str.into_bytes());
            
            let data_channel = self.data_channel.lock().await;
            if let Some(dc) = data_channel.as_ref() {
                dc.send(&data).await?;
                Ok(())
            } else {
                Err(Error::Peer("No data channel available".to_string()))
            }
        } else {
            Err(Error::Peer("Failed to serialize message".to_string()))
        }
    }

    pub async fn set_data_channel(&self, dc: Arc<RTCDataChannel>) {
        let mut data_channel = self.data_channel.lock().await;
        *data_channel = Some(dc);
    }
}

impl MediaRelayManager {
    pub fn new_with_turn(turn_ip: &str, turn_port: u16, username: &str, password: &str) -> Self {
        Self {
            relays: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            turn_config: Some(RTCIceServer {
                urls: vec![
                    format!("stun:{}:{}", turn_ip, turn_port),
                    format!("turn:{}:{}", turn_ip, turn_port),
                ],
                username: username.to_string(),
                credential: password.to_string(),
                credential_type: RTCIceCredentialType::Password,
                ..Default::default()
            }),
        }
    }

    pub fn new() -> Self {
        Self {
            relays: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            turn_config: None,
        }
    }

    pub async fn create_relay(
        &self, 
        peer_id: String,
        room_id: String,
        remote_peer_id: String,
        handler: Arc<impl SignalingHandler + Send + Sync + 'static>
    ) -> Result<MediaRelay> {
        let mut ice_servers = vec![];

        if let Some(turn_config) = &self.turn_config {
            ice_servers.push(turn_config.clone());
        }

        // Create a new MediaEngine
        let mut media_engine = MediaEngine::default();
        
        // Register default codecs
        media_engine.register_default_codecs()?;

        // Create an API with the MediaEngine
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .build();

        // Create ICE servers configuration
        let config = RTCConfiguration {
            ice_servers,
            ice_transport_policy: RTCIceTransportPolicy::All,
            bundle_policy: RTCBundlePolicy::MaxBundle,
            rtcp_mux_policy: RTCRtcpMuxPolicy::Require,
            ice_candidate_pool_size: 10,
            ..Default::default()
        };

        // Create a new PeerConnection
        let peer_connection = api.new_peer_connection(config).await?;

        // Set up handlers for the peer connection
        let pc = Arc::new(peer_connection);
        let pc_clone = pc.clone();
        let peer_id_clone = peer_id.clone();
        let handler_clone = handler.clone();

        // Handle incoming tracks
        pc.on_track(Box::new(move |track, _, _| {
            let pc = pc_clone.clone();
            Box::pin(async move {
                // Create a new local track based on the track type
                let (track_id, track_type) = match track.kind() {
                    webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio => {
                        ("audio-relay", "audio")
                    },
                    webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video => {
                        ("video-relay", "video")
                    },
                    _ => {
                        error!("Unsupported track type: {:?}", track.kind());
                        return;
                    }
                };

                let local_track = Arc::new(TrackLocalStaticRTP::new(
                    track.codec().capability,
                    format!("{}-{}", track_id, track.stream_id()),
                    track.stream_id(),
                ));

                // Add the track to the peer connection
                if let Err(e) = pc.add_track(local_track.clone()).await {
                    error!("Failed to add {} track: {}", track_type, e);
                } else {
                    debug!("Successfully added {} track to peer connection", track_type);
                }
            })
        }));

        // Clone values that will be moved into the closure
        let room_id_clone = room_id.clone();
        let remote_peer_id_clone = remote_peer_id.clone();
        let peer_id_clone = peer_id.clone();
        let handler_clone = handler.clone();

        pc.on_ice_candidate(Box::new(move |c| {
            let peer_id = peer_id_clone.clone();
            let room_id = room_id_clone.clone();
            let remote_peer_id = remote_peer_id_clone.clone();
            let handler = handler_clone.clone();
            
            Box::pin(async move {
                if let Some(c) = c {
                    let remote_peer_id_ref = remote_peer_id.clone(); // Clone for the message
                    // Send the ICE candidate through signaling
                    let candidate_msg = SignalingMessage::IceCandidate {
                        room_id,
                        candidate: serde_json::to_string(&c).unwrap(),
                        from_peer: peer_id,
                        to_peer: remote_peer_id.clone(),
                    };
                    
                    if let Err(e) = handler.send_to_peer(&remote_peer_id_ref, &candidate_msg).await {
                        error!("Failed to send ICE candidate: {}", e);
                    }
                }
            })
        }));

        // Create data channel
        let dc = pc.create_data_channel("signaling", None).await?;
        
        // Create the MediaRelay instance
        let relay = MediaRelay::new(pc.clone(), peer_id.clone());
        
        // Store the data channel
        relay.set_data_channel(dc).await;

        // Store the relay
        let mut relays = self.relays.write().await;
        relays.insert(peer_id, relay.clone());

        Ok(relay)
    }

    pub async fn forward_track(&self, from_peer: &str, to_peer: &str, track: Arc<TrackLocalStaticRTP>) -> Result<()> {
        let relays = self.relays.read().await;
        if let Some(relay) = relays.get(to_peer) {
            relay.peer_connection.add_track(track).await?;
        }
        Ok(())
    }

    pub async fn broadcast_track(&self, from_peer: &str, track: Arc<TrackLocalStaticRTP>) -> Result<()> {
        let relays = self.relays.read().await;
        for (peer_id, relay) in relays.iter() {
            if peer_id != from_peer {
                relay.peer_connection.add_track(track.clone()).await?;
            }
        }
        Ok(())
    }

    pub async fn remove_relay(&self, peer_id: &str) -> Result<()> {
        let mut relays = self.relays.write().await;
        if let Some(relay) = relays.remove(peer_id) {
            // Close the peer connection
            relay.peer_connection.close().await?;
            info!("Removed relay for peer {}", peer_id);
            
            // Get current relay count
            let relay_count = relays.len();
            info!("Active relays remaining: {}", relay_count);
        } else {
            warn!("Attempted to remove non-existent relay for peer {}", peer_id);
        }
        Ok(())
    }

    pub async fn handle_peer_disconnect(&self, peer_id: &str, room_id: &str) -> Result<()> {
        // Remove the relay
        self.remove_relay(peer_id).await?;
        
        // Get remaining peers in the room to notify them
        let relays = self.relays.read().await;
        let room_peers: Vec<String> = relays
            .iter()
            .filter_map(|(id, _)| {
                if id != peer_id {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        info!("Peer {} disconnected from room {}. Remaining peers: {:?}", 
            peer_id, room_id, room_peers);

        // Notify remaining peers about the disconnection
        for remaining_peer in &room_peers {
            if let Some(relay) = relays.get(remaining_peer) {
                let disconnect_msg = SignalingMessage::PeerList {
                    peers: room_peers.clone(),
                    room_id: room_id.to_string(),
                };
                
                if let Err(e) = relay.send_signal(&disconnect_msg).await {
                    error!("Failed to send peer list update to {}: {}", remaining_peer, e);
                }
            }
        }

        Ok(())
    }

    pub async fn cleanup_stale_relays(&self) {
        let monitor_interval = Duration::from_secs(30); // Check every 30 seconds
        loop {
            let mut relays = self.relays.write().await;
            let mut stale_peers = Vec::new();

            // Check each peer connection's state
            for (peer_id, relay) in relays.iter() {
                let connection_state = relay.peer_connection.connection_state();
                match connection_state {
                    webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Failed |
                    webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed |
                    webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Disconnected => {
                        stale_peers.push(peer_id.clone());
                        warn!("Found stale peer connection for {}: {:?}", peer_id, connection_state);
                    }
                    _ => {}
                }
            }

            // Remove stale peers
            for peer_id in stale_peers {
                if let Some(relay) = relays.remove(&peer_id) {
                    if let Err(e) = relay.peer_connection.close().await {
                        error!("Error closing connection for stale peer {}: {}", peer_id, e);
                    }
                    info!("Removed stale relay for peer {}", peer_id);
                }
            }

            drop(relays); // Release the write lock
            tokio::time::sleep(monitor_interval).await;
        }
    }

    pub async fn get_active_peer_count(&self) -> usize {
        self.relays.read().await.len()
    }

    pub async fn get_relay(&self, peer_id: &str) -> Option<MediaRelay> {
        let relays = self.relays.read().await;
        relays.get(peer_id).cloned()
    }

    // Add monitoring methods
    pub async fn monitor_relays(&self) {
        let monitor_interval = Duration::from_secs(5);
        loop {
            let relays = self.relays.read().await;
            for (peer_id, relay) in relays.iter() {
                match relay.get_stats().await {
                    Ok(stats) => {
                        info!(
                            "Media relay stats for peer {}: rx_packets={}, tx_packets={}, rx_bytes={}, tx_bytes={}",
                            peer_id,
                            stats.packets_received,
                            stats.packets_sent,
                            stats.bytes_received,
                            stats.bytes_sent
                        );
                    }
                    Err(e) => {
                        warn!("Failed to get stats for peer {}: {}", peer_id, e);
                    }
                }
            }
            tokio::time::sleep(monitor_interval).await;
        }
    }

    pub async fn get_relays(&self) -> Result<HashMap<String, MediaRelay>> {
        Ok(self.relays.read().await.clone())
    }
}

impl fmt::Debug for MediaRelay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaRelay")
            .field("peer_id", &self.peer_id)
            .field("audio_track", &self.audio_track)
            .field("video_track", &self.video_track)
            .field("data_channel", &"<RTCDataChannel>")
            .finish()
    }
} 