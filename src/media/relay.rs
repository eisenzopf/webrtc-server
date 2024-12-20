use crate::utils::{Error, Result};
use crate::types::SignalingMessage;
use std::sync::Arc;
use std::fmt;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::data_channel::RTCDataChannel;
use webrtc::stats::StatsReportType;
use bytes::Bytes;
use tokio::sync::Mutex;
use log::{debug, info, warn, error};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::TrackLocalWriter;
use webrtc::util::Marshal;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::track::track_local::TrackLocal;

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
    ice_candidate_buffer: Arc<Mutex<Vec<RTCIceCandidateInit>>>,
}

pub struct MediaRelayManager {
    relays: Arc<tokio::sync::RwLock<std::collections::HashMap<String, MediaRelay>>>,
    stun_server: String,
    stun_port: u16,
    turn_server: String,
    turn_port: u16,
    turn_username: String,
    turn_password: String,
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
    pub async fn new(peer_id: String) -> Result<Self> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;

        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .build();

        let peer_connection = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

        // Create audio track
        let audio_track = Arc::new(TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: "audio/opus".to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                ..Default::default()
            },
            "audio".to_owned(),
            "webrtc-rs".to_owned(),
        ));

        // Add transceiver with explicit direction
        peer_connection.add_transceiver_from_track(
            Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>, 
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Sendrecv,
                send_encodings: vec![],
            }),
        ).await?;

        // Set up track handlers with more detailed logging
        let track_clone = audio_track.clone();
        peer_connection.on_track(Box::new(move |track, _, _| {
            let track_clone = track_clone.clone();
            Box::pin(async move {
                debug!("Received track: kind={}, id={}, payload_type={}", 
                    track.kind(), 
                    track.id(),
                    track.payload_type()
                );
                
                while let Ok((rtp, _)) = track.read_rtp().await {
                    debug!("Forwarding RTP packet: ssrc={}, seq={}, ts={}, payload_size={}", 
                        rtp.header.ssrc, 
                        rtp.header.sequence_number,
                        rtp.header.timestamp,
                        rtp.payload.len()
                    );
                    if let Err(e) = track_clone.write_rtp(&rtp).await {
                        error!("Failed to forward RTP packet: {}", e);
                    }
                }
                debug!("Track reading loop ended");
            })
        }));

        Ok(MediaRelay {
            peer_id,
            peer_connection,
            audio_track: Some(audio_track),
            video_track: None,
            data_channel: Arc::new(Mutex::new(None)),
            ice_candidate_buffer: Arc::new(Mutex::new(Vec::new())),
        })
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

    pub async fn buffer_ice_candidate(&self, candidate: RTCIceCandidateInit) {
        let mut buffer = self.ice_candidate_buffer.lock().await;
        buffer.push(candidate);
    }

    pub async fn add_buffered_candidates(&self) -> Result<()> {
        let mut buffer = self.ice_candidate_buffer.lock().await;
        
        while let Some(candidate) = buffer.pop() {
            match self.peer_connection.add_ice_candidate(candidate).await {
                Ok(_) => {
                    debug!("Successfully added buffered ICE candidate");
                }
                Err(e) => {
                    warn!("Failed to add buffered ICE candidate: {}", e);
                }
            }
        }
        
        Ok(())
    }

    // Call this when remote description is set
    pub async fn handle_remote_description_set(&self) -> Result<()> {
        self.add_buffered_candidates().await
    }

    pub async fn set_remote_description(&self, sdp: String) -> Result<()> {
        let desc = RTCSessionDescription::answer(sdp)?;
        self.peer_connection.set_remote_description(desc).await?;
        
        // Apply any buffered candidates now that we have the remote description
        let mut buffer = self.ice_candidate_buffer.lock().await;
        while let Some(candidate) = buffer.pop() {
            if let Err(e) = self.peer_connection.add_ice_candidate(candidate).await {
                warn!("Failed to apply buffered ICE candidate: {}", e);
            }
        }
        Ok(())
    }

    pub async fn add_ice_candidate(&self, candidate: RTCIceCandidateInit) -> Result<()> {
        // If remote description is not set yet, buffer the candidate
        if self.peer_connection.remote_description().await.is_none() {
            let mut buffer = self.ice_candidate_buffer.lock().await;
            buffer.push(candidate);
            return Ok(());
        }

        // Otherwise add it immediately
        self.peer_connection.add_ice_candidate(candidate).await?;
        Ok(())
    }
}

impl MediaRelayManager {
    pub fn new(
        stun_server: String,
        stun_port: u16,
        turn_server: String,
        turn_port: u16,
        turn_username: String,
        turn_password: String,
    ) -> Self {
        Self {
            relays: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            stun_server,
            stun_port,
            turn_server,
            turn_port,
            turn_username,
            turn_password,
        }
    }

    pub async fn create_relay(&self, peer_id: String) -> Result<MediaRelay> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;

        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .build();

        let config = RTCConfiguration {
            ice_servers: vec![
                RTCIceServer {
                    urls: vec![
                        format!("stun:{}:{}", self.stun_server, self.stun_port),
                        format!("turn:{}:{}", self.turn_server, self.turn_port),
                    ],
                    username: self.turn_username.clone(),
                    credential: self.turn_password.clone(),
                    credential_type: RTCIceCredentialType::Password,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);
        let relay = MediaRelay::new(peer_id.clone()).await?;
        
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
        // Get the relay and peer list before removing
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

        // Close the peer connection if it exists
        if let Some(relay) = relays.get(peer_id) {
            relay.peer_connection.close().await?;
        }
        drop(relays); // Release the read lock

        // Now remove the relay
        self.remove_relay(peer_id).await?;

        info!("Peer {} disconnected from room {}. Remaining peers: {:?}", 
            peer_id, room_id, room_peers);

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

    pub async fn add_peer(&self, room_id: &str, peer_id: String) -> Result<()> {
        let mut relays = self.relays.write().await;
        
        // Create API and configuration for the peer connection
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;

        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .build();

        let config = RTCConfiguration {
            ice_servers: vec![
                RTCIceServer {
                    urls: vec![
                        format!("stun:{}:{}", self.stun_server, self.stun_port),
                        format!("turn:{}:{}", self.turn_server, self.turn_port)
                    ],
                    username: self.turn_username.clone(),
                    credential: self.turn_password.clone(),
                    credential_type: RTCIceCredentialType::Password,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);
        let media_relay = MediaRelay::new(peer_id.clone()).await?;

        relays.insert(peer_id, media_relay);
        Ok(())
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