use anyhow::Result;
use rsip::{
    self,
    prelude::*,
    SipMessage, 
    Request,
    Response,
    Headers,
    typed::{Via, From, To, Contact, CSeq},
    Domain,
    Transport,
    Version,
    Method,
};
use tokio::net::UdpSocket;
use std::sync::Arc;
use webrtc::rtp::packet::Packet as RTPPacket;
use crate::signaling::handler::MessageHandler;
use crate::types::SignalingMessage;
use uuid::Uuid;
use tokio::sync::RwLock;
use std::collections::HashMap;
use log::{info, error, debug};
use std::net::SocketAddr;

mod gateway;
mod session;
mod media;

pub use gateway::VoipGateway;
use session::{SessionManager, DialogState};
use media::MediaBridgeManager;

// Core VoIP handler for SIP signaling
pub struct VoIPHandler {
    message_handler: Arc<MessageHandler>,
    active_calls: Arc<RwLock<HashMap<String, String>>>, // call_id -> peer_id mapping
    socket: Arc<UdpSocket>,
    session_manager: Arc<SessionManager>,
    media_manager: Arc<MediaBridgeManager>,
    sip_domain: String,
    username: String,
    password: String,
}

impl VoIPHandler {
    pub async fn new(
        message_handler: Arc<MessageHandler>,
        bind_addr: &str,
        sip_domain: String,
        username: String,
        password: String,
    ) -> Result<Self> {
        let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
        
        Ok(Self {
            message_handler,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            socket,
            session_manager: Arc::new(SessionManager::new()),
            media_manager: Arc::new(MediaBridgeManager::new()),
            sip_domain,
            username,
            password,
        })
    }

    // Main method to start listening for SIP messages
    pub async fn run(&self) -> Result<()> {
        let mut buf = vec![0u8; 8192];
        
        loop {
            let (len, addr) = self.socket.recv_from(&mut buf).await?;
            let data = &buf[..len];
            
            match rsip::Message::try_from(data) {
                Ok(message) => {
                    self.handle_sip_message(message, addr).await?;
                }
                Err(e) => {
                    error!("Failed to parse SIP message: {}", e);
                }
            }
        }
    }

    async fn handle_invite(&self, request: Request, addr: SocketAddr) -> Result<()> {
        let call_id = request.call_id_header()?.typed()?.value.to_string();
        let peer_id = Uuid::new_v4().to_string();
        
        // Create new session
        let session = self.session_manager.create_session(call_id.clone()).await;
        
        // Create media bridge
        let (bridge, rtp_sender) = self.media_manager.create_bridge(&call_id).await;
        
        // Create WebRTC peer through message handler
        self.message_handler.handle_connect(&peer_id, "default").await?;
        
        // Store mapping
        self.active_calls.write().await.insert(call_id.clone(), peer_id.clone());
        
        // Send 200 OK with SDP
        let mut session = session.write().await;
        let response = session.generate_response(&request, 200)?;
        
        self.socket.send_to(
            response.to_string().as_bytes(),
            addr
        ).await?;

        Ok(())
    }

    async fn handle_bye(&self, request: Request, addr: SocketAddr) -> Result<()> {
        let call_id = request.call_id_header()?.typed()?.value.to_string();
        
        // Clean up session and media bridge
        if let Some(peer_id) = self.active_calls.write().await.remove(&call_id) {
            self.message_handler.handle_disconnect(&peer_id, "default").await?;
            self.session_manager.remove_session(&call_id).await;
            self.media_manager.remove_bridge(&call_id).await;
        }
        
        // Send 200 OK
        if let Some(session) = self.session_manager.get_session(&call_id).await {
            let mut session = session.write().await;
            let response = session.generate_response(&request, 200)?;
            
            self.socket.send_to(
                response.to_string().as_bytes(),
                addr
            ).await?;
        }

        Ok(())
    }

    async fn handle_sip_message(&self, message: SipMessage, addr: SocketAddr) -> Result<()> {
        match message {
            SipMessage::Request(request) => {
                match request.method() {
                    Method::Invite => self.handle_invite(request, addr).await?,
                    Method::Bye => self.handle_bye(request, addr).await?,
                    _ => debug!("Unhandled SIP request method: {}", request.method()),
                }
            },
            SipMessage::Response(response) => {
                // Handle responses
                debug!("Received SIP response: {}", response.status_code());
            }
        }
        Ok(())
    }
} 