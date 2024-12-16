use anyhow::Result;
use rsip::{
    prelude::*,
    SipMessage,
    Request,
    Response,
    Headers,
    typed::{Via, From, To, Contact, CSeq},
    Version,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use uuid::Uuid;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp::packet::Packet as RTPPacket;
use log::{info, debug, error};

pub struct SipSession {
    call_id: String,
    peer_id: String,
    dialog_state: DialogState,
    peer_connection: Option<Arc<RTCPeerConnection>>,
    local_tag: String,
    remote_tag: Option<String>,
    cseq: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DialogState {
    Initial,
    Early,
    Confirmed,
    Terminated,
}

impl SipSession {
    pub fn new(call_id: String) -> Self {
        Self {
            call_id,
            peer_id: Uuid::new_v4().to_string(),
            dialog_state: DialogState::Initial,
            peer_connection: None,
            local_tag: Uuid::new_v4().to_string(),
            remote_tag: None,
            cseq: 1,
        }
    }

    pub fn from_request(request: &Request) -> Result<Self> {
        let call_id = request.call_id_header()?.typed()?.value.to_string();
        let mut session = Self::new(call_id);
        
        if let Ok(to) = request.to_header()?.typed() {
            if let Some(tag) = to.tag.clone() {
                session.remote_tag = Some(tag.to_string());
            }
        }

        Ok(session)
    }

    pub async fn set_peer_connection(&mut self, pc: Arc<RTCPeerConnection>) {
        self.peer_connection = Some(pc);
    }

    pub fn generate_response(&mut self, request: &Request, status_code: u16) -> Result<Response> {
        let mut headers = Headers::default();
        
        // Copy Via headers
        headers.push(request.via_header()?.clone().into());
        
        // Copy From header
        headers.push(request.from_header()?.clone().into());
        
        // Create To header with our tag
        let mut to = request.to_header()?.typed()?;
        to.with_tag(self.local_tag.clone());
        headers.push(to.into());
        
        // Copy Call-ID
        headers.push(request.call_id_header()?.clone().into());
        
        // Copy CSeq
        headers.push(request.cseq_header()?.clone().into());
        
        // Add Contact header
        let contact = Contact::new(self.local_tag.clone());
        headers.push(contact.into());

        Ok(Response {
            status_code: status_code.into(),
            headers,
            version: Version::V2,
            body: Default::default(),
        })
    }

    pub fn update_state(&mut self, new_state: DialogState) {
        debug!("Session {} state change: {:?} -> {:?}", 
            self.call_id, self.dialog_state, new_state);
        self.dialog_state = new_state;
    }

    pub fn get_peer_id(&self) -> &str {
        &self.peer_id
    }
}

pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, Arc<RwLock<SipSession>>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_session(&self, call_id: String) -> Arc<RwLock<SipSession>> {
        let session = Arc::new(RwLock::new(SipSession::new(call_id.clone())));
        self.sessions.write().await.insert(call_id, session.clone());
        session
    }

    pub async fn get_session(&self, call_id: &str) -> Option<Arc<RwLock<SipSession>>> {
        self.sessions.read().await.get(call_id).cloned()
    }

    pub async fn remove_session(&self, call_id: &str) {
        self.sessions.write().await.remove(call_id);
    }

    pub async fn get_all_session_ids(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }
} 