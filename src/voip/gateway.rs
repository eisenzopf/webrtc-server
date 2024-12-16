use anyhow::Result;
use std::sync::Arc;
use tokio::net::UdpSocket;
use crate::signaling::handler::MessageHandler;
use super::{VoIPHandler, session::SessionManager, media::MediaBridgeManager};
use log::{info, error};

pub struct VoipGateway {
    handler: Arc<VoIPHandler>,
    session_manager: Arc<SessionManager>,
    media_manager: Arc<MediaBridgeManager>,
}

impl VoipGateway {
    pub async fn new(
        sip_server: &str,
        sip_port: u16,
        username: &str,
        password: &str,
        message_handler: Arc<MessageHandler>,
    ) -> Result<Self> {
        // Bind to a local address for SIP communication
        let bind_addr = format!("0.0.0.0:{}", sip_port);
        
        let session_manager = Arc::new(SessionManager::new());
        let media_manager = Arc::new(MediaBridgeManager::new());
        
        let handler = Arc::new(VoIPHandler::new(
            message_handler,
            &bind_addr,
            sip_server.to_string(),
            username.to_string(),
            password.to_string(),
        ).await?);

        Ok(Self {
            handler,
            session_manager,
            media_manager,
        })
    }

    pub async fn start(self) -> Result<()> {
        info!("Starting VoIP gateway...");
        self.handler.run().await
    }

    // Helper method to get active sessions
    pub async fn get_active_sessions(&self) -> Vec<String> {
        self.session_manager.get_all_session_ids().await
    }

    // Helper method to get active media bridges
    pub async fn get_active_bridges(&self) -> Vec<String> {
        self.media_manager.get_all_bridge_ids().await
    }
} 