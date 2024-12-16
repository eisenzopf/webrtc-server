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
    active_recordings: Mutex<HashMap<String, Recording>>,
}

struct Recording {
    file: File,
    participants: Vec<String>,
    call_id: String,
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

    pub async fn start_call_recording(&self, room_id: &str, participants: Vec<String>) -> Result<()> {
        let mut recordings = self.active_recordings.lock().await;
        
        if recordings.contains_key(room_id) {
            return Ok(());  // Recording already exists
        }

        let call_id = Uuid::new_v4();
        let filename = format!(
            "call_{}_{}.rtp",
            Utc::now().format("%Y%m%d_%H%M%S"),
            call_id
        );
        
        let filepath = self.recording_path.join(filename);
        let file = File::create(&filepath)?;
        
        let metadata_filename = format!("call_{}_{}.json", Utc::now().format("%Y%m%d_%H%M%S"), call_id);
        let metadata_path = self.recording_path.join(metadata_filename);
        let metadata = serde_json::json!({
            "room_id": room_id,
            "participants": participants,
            "start_time": Utc::now().to_rfc3339(),
            "call_id": call_id.to_string()
        });
        std::fs::write(metadata_path, serde_json::to_string_pretty(&metadata)?)?;
        
        recordings.insert(room_id.to_string(), Recording {
            file,
            participants,
            call_id: call_id.to_string(),
        });
        
        info!("Started recording for room {} with call_id {}", room_id, call_id);
        Ok(())
    }

    pub async fn write_rtp_packet(&self, room_id: &str, packet: &RTPPacket) -> Result<()> {
        let mut recordings = self.active_recordings.lock().await;
        if let Some(recording) = recordings.get_mut(room_id) {
            let mut buf = Vec::new();
            packet.marshal_to(&mut buf)?;
            recording.file.write_all(&buf)?;
        }
        Ok(())
    }

    pub async fn stop_call_recording(&self, room_id: &str) -> Result<()> {
        let mut recordings = self.active_recordings.lock().await;
        recordings.remove(room_id);
        Ok(())
    }
} 