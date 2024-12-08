use crate::utils::{Error, Result};
use crate::signaling::handler::MessageHandler;
use tokio::net::TcpListener;
use std::sync::Arc;
use tokio_tungstenite::accept_async;
use futures_util::{StreamExt, SinkExt};

pub struct SignalingServer {
    address: String,
    handler: Arc<MessageHandler>,
}

impl SignalingServer {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            handler: Arc::new(MessageHandler::new()),
        }
    }

    pub async fn start(&self) -> Result<()> {
        println!("Starting server on {}", self.address);
        let listener = TcpListener::bind(&self.address).await
            .map_err(|e| Error::IO(e))?;

        while let Ok((stream, addr)) = listener.accept().await {
            println!("New connection from: {}", addr);
            let handler = self.handler.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, handler).await {
                    eprintln!("Connection error: {}", e);
                }
            });
        }

        Ok(())
    }

    async fn handle_connection(
        stream: tokio::net::TcpStream,
        handler: Arc<MessageHandler>,
    ) -> Result<()> {
        let ws_stream = accept_async(stream).await
            .map_err(|e| Error::WebSocket(e))?;
        let (ws_sender, mut ws_receiver) = ws_stream.split();
        let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));
        
        let mut current_peer_id = None;
        let mut current_room_id = None;

        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(msg) => {
                    if let Ok(msg_str) = msg.to_text() {
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