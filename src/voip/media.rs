use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::rtp::packet::Packet as RTPPacket;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_remote::TrackRemote;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTCRtpCodecCapability};
use webrtc::track::track_local::TrackLocalWriter;
use log::{debug, error};
use std::collections::HashMap;

pub struct MediaBridge {
    // Channels for RTP packets
    rtp_sender: mpsc::Sender<RTPPacket>,
    rtp_receiver: mpsc::Receiver<RTPPacket>,
    
    // WebRTC tracks
    local_track: Option<Arc<TrackLocalStaticRTP>>,
    remote_track: Option<Arc<TrackRemote>>,
}

impl MediaBridge {
    pub fn new() -> (Self, mpsc::Sender<RTPPacket>) {
        let (tx1, rx1) = mpsc::channel(1000); // For SIP -> WebRTC
        let (tx2, rx2) = mpsc::channel(1000); // For WebRTC -> SIP

        (
            Self {
                rtp_sender: tx2,
                rtp_receiver: rx1,
                local_track: None,
                remote_track: None,
            },
            tx1,
        )
    }

    pub async fn set_local_track(&mut self, track: Arc<TrackLocalStaticRTP>) {
        self.local_track = Some(track);
    }

    pub async fn set_remote_track(&mut self, track: Arc<TrackRemote>) {
        self.remote_track = Some(track.clone());
        
        let rtp_sender = self.rtp_sender.clone();
        
        // Handle incoming WebRTC RTP packets
        tokio::spawn(async move {
            while let Ok((rtp_packet, _)) = track.read_rtp().await {
                if let Err(e) = rtp_sender.send(rtp_packet).await {
                    error!("Failed to forward WebRTC RTP packet to SIP: {}", e);
                    break;
                }
            }
        });
    }

    pub async fn start(&mut self) -> Result<()> {
        let mut rtp_receiver = std::mem::replace(&mut self.rtp_receiver, mpsc::channel(1000).1);
        let local_track = self.local_track.clone();

        // Forward SIP RTP packets to WebRTC
        tokio::spawn(async move {
            while let Some(packet) = rtp_receiver.recv().await {
                if let Some(track) = &local_track {
                    if let Err(e) = track.write_rtp(&packet).await {
                        error!("Failed to forward SIP RTP packet to WebRTC: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    pub fn get_rtp_parameters(&self) -> RTCRtpCodecParameters {
        // Default to opus for audio
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: "audio/opus".to_string(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                rtcp_feedback: vec![],
            },
            payload_type: 111,
            stats_id: String::new(),
        }
    }
}

pub struct MediaBridgeManager {
    bridges: tokio::sync::RwLock<HashMap<String, Arc<tokio::sync::RwLock<MediaBridge>>>>,
}

impl MediaBridgeManager {
    pub fn new() -> Self {
        Self {
            bridges: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    pub async fn create_bridge(&self, session_id: &str) -> (Arc<tokio::sync::RwLock<MediaBridge>>, mpsc::Sender<RTPPacket>) {
        let (bridge, sender) = MediaBridge::new();
        let bridge = Arc::new(tokio::sync::RwLock::new(bridge));
        self.bridges.write().await.insert(session_id.to_string(), bridge.clone());
        (bridge, sender)
    }

    pub async fn get_bridge(&self, session_id: &str) -> Option<Arc<tokio::sync::RwLock<MediaBridge>>> {
        self.bridges.read().await.get(session_id).cloned()
    }

    pub async fn remove_bridge(&self, session_id: &str) {
        self.bridges.write().await.remove(session_id);
    }

    pub async fn get_all_bridge_ids(&self) -> Vec<String> {
        self.bridges.read().await.keys().cloned().collect()
    }
} 