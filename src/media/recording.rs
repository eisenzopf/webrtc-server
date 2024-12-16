use crate::types::{RecordingMetadata, ParticipantInfo, RoomRecording, ParticipantRecording};
use crate::utils::{Error, Result};
use std::path::PathBuf;
use std::collections::HashMap;
use tokio::sync::Mutex;
use webrtc::rtp::packet::Packet as RTPPacket;
use std::fs::File;
use std::io::Write;
use chrono::Utc;
use log::{info, error};
use uuid::Uuid;
use serde_json;
use webrtc::util::Marshal;

pub struct RecordingManager {
    recording_path: PathBuf,
    active_recordings: Mutex<HashMap<String, RoomRecording>>,
}

impl RecordingManager {
    pub fn new(recording_path: PathBuf) -> Self {
        std::fs::create_dir_all(&recording_path).unwrap_or_else(|e| {
            error!("Failed to create recording directory: {}", e);
        });
        
        Self {
            recording_path,
            active_recordings: Mutex::new(HashMap::new()),
        }
    }

    pub async fn start_call_recording(&self, room_id: &str, initial_participants: Vec<String>) -> Result<()> {
        let mut recordings = self.active_recordings.lock().await;
        
        if recordings.contains_key(room_id) {
            return Ok(());  // Recording already exists
        }

        let call_id = Uuid::new_v4().to_string();
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let mut participant_files = HashMap::new();
        let mut participant_info = HashMap::new();

        // Create separate recording files for each participant
        for peer_id in initial_participants {
            let filename = format!("call_{}_{}_{}.rtp", timestamp, call_id, peer_id);
            let filepath = self.recording_path.join(&filename);
            let file = File::create(&filepath)?;

            info!("Creating recording file for peer {}: {}", peer_id, filename);

            participant_info.insert(peer_id.clone(), ParticipantInfo {
                peer_id: peer_id.clone(),
                join_time: Utc::now().to_rfc3339(),
                file_path: filename.clone(),
            });

            participant_files.insert(peer_id.clone(), ParticipantRecording {
                file,
                peer_id: peer_id.clone(),
                rtp_file_path: filepath,
            });
        }

        // Create and save metadata
        let metadata = RecordingMetadata {
            call_id: call_id.clone(),
            room_id: room_id.to_string(),
            start_time: Utc::now().to_rfc3339(),
            participants: participant_info,
        };

        let metadata_filename = format!("call_{}_{}_metadata.json", timestamp, call_id);
        let metadata_path = self.recording_path.join(&metadata_filename);
        std::fs::write(
            &metadata_path,
            serde_json::to_string_pretty(&metadata)?
        )?;

        recordings.insert(room_id.to_string(), RoomRecording {
            call_id,
            participant_files,
            metadata,
        });

        Ok(())
    }

    pub async fn add_participant(&self, room_id: &str, peer_id: &str) -> Result<()> {
        let mut recordings = self.active_recordings.lock().await;
        
        if let Some(recording) = recordings.get_mut(room_id) {
            if !recording.participant_files.contains_key(peer_id) {
                let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
                let filename = format!("call_{}_{}_{}.rtp", timestamp, recording.call_id, peer_id);
                let filepath = self.recording_path.join(&filename);
                let file = File::create(&filepath)?;

                info!("Adding new participant to recording: {}", peer_id);

                recording.participant_files.insert(peer_id.to_string(), ParticipantRecording {
                    file,
                    peer_id: peer_id.to_string(),
                    rtp_file_path: filepath,
                });

                recording.metadata.participants.insert(peer_id.to_string(), ParticipantInfo {
                    peer_id: peer_id.to_string(),
                    join_time: Utc::now().to_rfc3339(),
                    file_path: filename,
                });

                // Update metadata file
                let metadata_path = self.recording_path.join(
                    format!("call_{}_metadata.json", recording.call_id)
                );
                std::fs::write(
                    &metadata_path,
                    serde_json::to_string_pretty(&recording.metadata)?
                )?;
            }
        }

        Ok(())
    }

    pub async fn write_rtp_packet(&self, room_id: &str, peer_id: &str, packet: &RTPPacket) -> Result<()> {
        let mut recordings = self.active_recordings.lock().await;
        
        if let Some(recording) = recordings.get_mut(room_id) {
            if let Some(participant_recording) = recording.participant_files.get_mut(peer_id) {
                let buf = packet.marshal()?;
                participant_recording.file.write_all(&buf)?;
            }
        }

        Ok(())
    }
} 