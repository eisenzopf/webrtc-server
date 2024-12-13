use std::net::SocketAddr;
use tokio::net::UdpSocket;
use stun::message::{Message, MessageType, Method, BINDING_REQUEST, CLASS_SUCCESS_RESPONSE, METHOD_BINDING};
use stun::xoraddr::XorMappedAddress;
use stun::message::Setter;
use stun::attributes::ATTR_XORMAPPED_ADDRESS;
use log::{info, error, debug, trace};
use crate::utils::{Error, Result};

pub struct StunService {
    addr: SocketAddr,
}

impl StunService {
    pub async fn new(addr: &str) -> Result<Self> {
        let addr = addr.parse()
            .map_err(|e: std::net::AddrParseError| {
                error!("Failed to parse address {}: {}", addr, e);
                Error::IO(e.to_string())
            })?;

        Ok(Self { addr })
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting STUN server on {}", self.addr);
        
        let socket = UdpSocket::bind(self.addr).await
            .map_err(|e| {
                error!("Failed to bind UDP socket: {}", e);
                Error::IO(e.to_string())
            })?;

        let mut buf = vec![0u8; 1024];

        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, src)) => {
                    trace!("Received UDP packet: {} bytes from {}", len, src);
                    debug!("Raw packet data: {:?}", &buf[..len]);
                    
                    let mut msg = Message::new();
                    match msg.unmarshal_binary(&buf[..len]) {
                        Ok(_) => {
                            debug!("Decoded STUN message type: {:?}", msg.typ);
                            if msg.typ == BINDING_REQUEST {
                                info!("Processing STUN binding request from {}", src);
                                
                                let mut response = Message::new();
                                response.transaction_id.0.copy_from_slice(&msg.transaction_id.0);
                                response.typ = MessageType::new(Method::from(METHOD_BINDING), CLASS_SUCCESS_RESPONSE);
                                
                                debug!("Creating XOR-MAPPED-ADDRESS for {}:{}", src.ip(), src.port());
                                let addr = XorMappedAddress {
                                    ip: src.ip(),
                                    port: src.port(),
                                };
                                trace!("XOR-MAPPED-ADDRESS components - IP: {}, Port: {}", src.ip(), src.port());

                                if let Err(e) = addr.add_to_as(&mut response, ATTR_XORMAPPED_ADDRESS) {
                                    error!("Failed to add XOR-MAPPED-ADDRESS: {}", e);
                                    continue;
                                }
                                trace!("Added XOR-MAPPED-ADDRESS attribute to response");

                                match response.marshal_binary() {
                                    Ok(bytes) => {
                                        debug!("Sending STUN binding response ({} bytes) to {}", bytes.len(), src);
                                        trace!("Response data: {:?}", bytes);
                                        if let Err(e) = socket.send_to(&bytes, src).await {
                                            error!("Failed to send response: {}", e);
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to marshal STUN response: {}", e);
                                    }
                                }
                            } else {
                                debug!("Received non-binding request: {:?}", msg.typ);
                            }
                        }
                        Err(e) => {
                            debug!("Failed to decode STUN message: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Socket receive error: {}", e);
                    return Err(Error::IO(e.to_string()));
                }
            }
        }
    }
}