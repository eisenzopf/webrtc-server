use anyhow::Result;
use std::sync::Arc;
use tokio::net::UdpSocket;
use crate::signaling::handler::MessageHandler;
use super::{VoIPHandler, session::SessionManager, media::MediaBridgeManager};
use log::{info, error};
use super::sip_server::SipServer;

pub struct VoipGateway {
    handler: Arc<VoIPHandler>,
    session_manager: Arc<SessionManager>,
    media_manager: Arc<MediaBridgeManager>,
}

impl VoipGateway {
    pub async fn new(
        bind_addr: &str,
        domain: &str,
        message_handler: Arc<MessageHandler>,
    ) -> Result<Self> {
        let session_manager = Arc::new(SessionManager::new());
        let media_manager = Arc::new(MediaBridgeManager::new());
        let sip_server = Arc::new(SipServer::new(bind_addr, domain).await?);
        
        let handler = VoIPHandler::new(
            message_handler,
            bind_addr,
            domain.to_string(),
            "user".to_string(),  // Default username
            "pass".to_string(),  // Default password
        ).await?;

        Ok(Self {
            handler: Arc::new(handler),
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