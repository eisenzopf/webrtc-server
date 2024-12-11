#![allow(warnings)]
use anyhow::Result;
use webrtc_server::signaling::server::SignalingServer;
use webrtc_server::signaling::stun::StunService;
use std::error::Error;
use env_logger::Env;
use tokio::select;
use tokio::signal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging with default level INFO
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    log::info!("Starting WebRTC servers...");

    // Create both servers
    let signaling_server = SignalingServer::new("0.0.0.0:8080");
    let stun_server = StunService::new("0.0.0.0:3478").await?;

    // Run both servers concurrently with shutdown handling
    select! {
        result = signaling_server.start() => {
            if let Err(e) = result {
                log::error!("Signaling server error: {}", e);
            }
        }
        result = stun_server.run() => {
            if let Err(e) = result {
                log::error!("STUN server error: {}", e);
            }
        }
        _ = signal::ctrl_c() => {
            log::info!("Received shutdown signal");
        }
    }

    log::info!("Servers stopped normally");
    Ok(())
}