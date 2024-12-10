use std::net::SocketAddr;
use tokio::net::UdpSocket;
use stun::message::{Message, MessageType, BINDING_REQUEST, Setter};
use stun::agent::TransactionId;
use stun::xoraddr::XorMappedAddress;
use stun::attributes::{ATTR_XORMAPPED_ADDRESS, RawAttribute};
use crate::utils::{Error, Result};
use log::{info, error, debug};

pub struct StunService {
    addr: SocketAddr,
}

impl StunService {
    pub async fn new(addr: &str) -> Result<Self> {
        let addr = addr.parse()
            .map_err(|e| {
                error!("Failed to parse address {}: {}", addr, e);
                Error::IO(std::io::Error::new(std::io::ErrorKind::Other, e))
            })?;

        Ok(Self { addr })
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting STUN server on {}", self.addr);
        
        let socket = UdpSocket::bind(self.addr).await
            .map_err(|e| {
                error!("Failed to bind UDP socket: {}", e);
                Error::IO(e)
            })?;

        let mut buf = vec![0u8; 1024];

        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, src)) => {
                    debug!("Received {} bytes from {}", len, src);
                    
                    let mut msg = Message::new();
                    msg.raw.extend_from_slice(&buf[..len]);
                    
                    match msg.decode() {
                        Ok(()) if msg.typ == BINDING_REQUEST => {
                            let mut response = Message::new();
                            let mut txid = TransactionId::new();
                            txid.0.copy_from_slice(&msg.transaction_id.0);
                            
                            response.build(&[
                                Box::new(txid),
                                Box::new(BINDING_REQUEST)
                            ]).map_err(|e| Error::IO(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
                            
                            let addr = XorMappedAddress {
                                ip: src.ip(),
                                port: src.port(),
                            };
                            
                            addr.add_to(&mut response)
                                .map_err(|e| Error::IO(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
                            
                            if let Err(e) = socket.send_to(&response.raw, src).await {
                                error!("Failed to send response: {}", e);
                            }
                        }
                        Ok(()) => debug!("Received non-binding request"),
                        Err(e) => error!("Failed to decode STUN message: {}", e),
                    }
                }
                Err(e) => {
                    error!("Socket receive error: {}", e);
                    return Err(Error::IO(e));
                }
            }
        }
    }
}