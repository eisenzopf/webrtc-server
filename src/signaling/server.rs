use crate::utils::{Error, Result};
use crate::signaling::handler::MessageHandler;
use tokio::net::TcpListener;
use std::sync::Arc;
use tokio_tungstenite::accept_async;
use futures_util::{StreamExt, SinkExt};
use log::{info, warn, error, debug};

pub struct SignalingServer {
    address: String,
    handler: Arc<MessageHandler>,
}

impl SignalingServer {
    pub fn new(address: &str) -> Self {
        info!("Creating new SignalingServer instance on {}", address);
        Self {
            address: address.to_string(),
            handler: Arc::new(MessageHandler::new()),
        }
    }

    pub async fn start(&self) -> Result<()> {
        info!("Starting server on {}", self.address);
        let listener = TcpListener::bind(&self.address).await
            .map_err(|e| {
                error!("Failed to bind to address {}: {}", self.address, e);
                Error::IO(e)
            })?;

        info!("Server successfully bound to {}", self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            info!("New connection from: {}", addr);
            let handler = self.handler.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, handler).await {
                    error!("Connection error from {}: {}", addr, e);
                }
            });
        }

        warn!("Server accept loop terminated");
        Ok(())
    }

    async fn handle_connection(
        stream: tokio::net::TcpStream,
        handler: Arc<MessageHandler>,
    ) -> Result<()> {
        let peer_addr = stream.peer_addr().map_err(|e| {
            error!("Failed to get peer address: {}", e);
            Error::IO(e)
        })?;
        
        debug!("Upgrading connection to WebSocket for peer {}", peer_addr);
        let ws_stream = accept_async(stream).await
            .map_err(|e| {
                error!("WebSocket upgrade failed for {}: {}", peer_addr, e);
                Error::WebSocket(e)
            })?;
        
        info!("Successfully established WebSocket connection with {}", peer_addr);
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));
        
        let mut current_peer_id = None;
        let mut current_room_id = None;

        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(msg) => {
                    if let Ok(msg_str) = msg.to_text() {
                        debug!("Received message from {}: {}", peer_addr, msg_str);
                        if let Ok(signal_msg) = serde_json::from_str(msg_str) {
                            handler.handle_message(
                                signal_msg,
                                ws_sender.clone(),
                                &mut current_peer_id,
                                &mut current_room_id,
                            ).await?;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("WebSocket error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }
} 