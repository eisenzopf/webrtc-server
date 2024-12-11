use crate::utils::{Error, Result};
use std::sync::Arc;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::ice_transport::ice_server::RTCIceServer;
use log::{debug, info, warn, error};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use webrtc::stats::StatsReportType;
use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;
use webrtc::peer_connection::policy::bundle_policy::RTCBundlePolicy;
use webrtc::peer_connection::policy::rtcp_mux_policy::RTCRtcpMuxPolicy;
use crate::signaling::SignalingMessage;

#[derive(Debug, Clone)]
pub struct MediaRelay {
    pub peer_connection: Arc<RTCPeerConnection>,
    pub audio_track: Option<Arc<TrackLocalStaticRTP>>,
    pub video_track: Option<Arc<TrackLocalStaticRTP>>,
    pub peer_id: String,
}

pub struct MediaRelayManager {
    relays: Arc<tokio::sync::RwLock<std::collections::HashMap<String, MediaRelay>>>,
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
}

impl MediaRelayManager {
    pub fn new() -> Self {
        Self {
            relays: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn create_relay(&self, peer_id: String) -> Result<MediaRelay> {
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
            ice_servers: vec![
                RTCIceServer {
                    urls: vec![format!("stun:{}:{}", "192.168.1.68", "3478")],
                    ..Default::default()
                },
            ],
            ice_transport_policy: RTCIceTransportPolicy::Relay,
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

        pc.on_ice_candidate(Box::new(move |c| {
            let peer_id = peer_id_clone.clone();
            let handler = handler_clone.clone();
            
            Box::pin(async move {
                if let Some(c) = c {
                    // Send the ICE candidate through signaling
                    let candidate_msg = SignalingMessage::IceCandidate {
                        room_id: room_id.clone(),
                        candidate: serde_json::to_string(&c).unwrap(),
                        from_peer: peer_id.clone(),
                        to_peer: remote_peer_id.clone(),
                    };
                    
                    if let Err(e) = handler.send_to_peer(&remote_peer_id, &candidate_msg).await {
                        error!("Failed to send ICE candidate: {}", e);
                    }
                }
            })
        }));

        // Create the MediaRelay instance
        let relay = MediaRelay {
            peer_connection: pc,
            audio_track: None,
            video_track: None,
            peer_id: peer_id.clone(),
        };

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
        }
        Ok(())
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