#![allow(warnings)]
mod metrics;
mod room;
mod server;
mod types;

use anyhow::Result;
use crate::server::SignalingServer;

#[tokio::main]
async fn main() -> Result<()> {
    let server = SignalingServer::new("127.0.0.1:8080");
    server.start().await?;
    Ok(())
}