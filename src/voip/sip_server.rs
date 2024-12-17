use anyhow::Result;
use rsip::{
    SipMessage, 
    Request,
    Response,
    Headers,
    typed::{Via, From, To, Contact, CSeq},
    StatusCode,
    Method,
    Transport,
    Version,
    Uri,
    Domain,
    Host,
    Scheme,
    message::HeadersExt,
    headers::{ToTypedHeader, UntypedHeader},
};
use std::sync::Arc;
use tokio::net::UdpSocket;
use std::net::SocketAddr;
use log::{info, debug, error};
use tokio::sync::RwLock;
use std::collections::HashMap;

pub struct SipServer {
    socket: Arc<UdpSocket>,
    domain: String,
    registrar: Arc<RwLock<HashMap<String, RegistrationInfo>>>,
}

#[derive(Clone)]
struct RegistrationInfo {
    contact: String,
    expires: u32,
    address: SocketAddr,
}

impl SipServer {
    pub async fn new(bind_addr: &str, domain: &str) -> Result<Self> {
        let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
        info!("SIP server listening on {}", bind_addr);
        
        Ok(Self {
            socket,
            domain: domain.to_string(),
            registrar: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn run(&self) -> Result<()> {
        let mut buf = vec![0u8; 65535];
        
        loop {
            let (size, addr) = self.socket.recv_from(&mut buf).await?;
            let data = &buf[..size];
            
            if let Ok(message) = std::str::from_utf8(data) {
                if let Ok(sip_message) = SipMessage::try_from(message) {
                    self.handle_message(sip_message, addr).await?;
                }
            }
        }
    }

    async fn handle_message(&self, message: SipMessage, addr: SocketAddr) -> Result<()> {
        match message {
            SipMessage::Request(request) => {
                match request.method() {
                    Method::Register => self.handle_register(request, addr).await?,
                    Method::Invite => self.handle_invite(request, addr).await?,
                    Method::Bye => self.handle_bye(request, addr).await?,
                    _ => debug!("Unhandled SIP request: {}", request.method()),
                }
            },
            SipMessage::Response(response) => {
                debug!("Received SIP response: {}", response.status_code());
            }
        }
        Ok(())
    }

    async fn handle_register(&self, request: Request, addr: SocketAddr) -> Result<()> {
        // Extract username from request
        let from_header = request.from_header()?.typed()?;
        let username = from_header.uri.host().to_string();

        // Update registration
        let contact_header = request.contact_header()?.typed()?;
        let contact = contact_header.uri.to_string();
        let expires = request.expires_header()
            .map(|h| h.value().parse::<u32>().unwrap_or(3600))
            .unwrap_or(3600);

        self.registrar.write().await.insert(username.clone(), RegistrationInfo {
            contact,
            expires,
            address: addr,
        });

        // Create 200 OK response
        let mut response = Response {
            status_code: StatusCode::OK,
            headers: Headers::default(),
            version: Version::V2,
            body: Default::default(),
        };

        // Add required headers
        response.headers.push(Via {
            version: Version::V2,
            transport: Transport::Udp,
            uri: Uri::from((
                Scheme::Sip,
                Host::Domain(Domain::from(self.domain.clone()))
            )),
            params: Default::default(),
        }.into());

        response.headers.push(From {
            display_name: None,
            uri: Uri::from((
                Scheme::Sip,
                Host::Domain(Domain::from(username.clone()))
            )),
            params: Default::default(),
        }.into());

        response.headers.push(To {
            display_name: None,
            uri: Uri::from((
                Scheme::Sip,
                Host::Domain(Domain::from(self.domain.clone()))
            )),
            params: Default::default(),
        }.into());

        response.headers.push(Contact {
            display_name: None,
            uri: Uri::from((
                Scheme::Sip,
                Host::Domain(Domain::from(format!("{}@{}", username, self.domain)))
            )),
            params: Default::default(),
        }.into());

        self.socket.send_to(
            response.to_string().as_bytes(),
            addr
        ).await?;

        info!("Registered user {} at {}", username, addr);
        Ok(())
    }

    async fn handle_invite(&self, request: Request, addr: SocketAddr) -> Result<()> {
        // TODO: Implement INVITE handling
        debug!("Received INVITE request from {}", addr);
        Ok(())
    }

    async fn handle_bye(&self, request: Request, addr: SocketAddr) -> Result<()> {
        // TODO: Implement BYE handling
        debug!("Received BYE request from {}", addr);
        Ok(())
    }
}
