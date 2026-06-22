use async_trait::async_trait;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    UDP,
    TCP,
    WebRTC,
    WebSocket,
    SPICE,
    RDP,
    VNC,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::UDP => write!(f, "UDP"),
            Protocol::TCP => write!(f, "TCP"),
            Protocol::WebRTC => write!(f, "WebRTC"),
            Protocol::WebSocket => write!(f, "WebSocket"),
            Protocol::SPICE => write!(f, "SPICE"),
            Protocol::RDP => write!(f, "RDP"),
            Protocol::VNC => write!(f, "VNC"),
        }
    }
}

#[async_trait]
pub trait ProtocolHandler: Send + Sync {
    fn name(&self) -> Protocol;
    
    /// Tests the latency of the protocol in milliseconds.
    /// Returns None if the connection fails or is completely unavailable.
    async fn test_latency(&self) -> Option<u64>;
}

// Mock implementations for testing
pub struct MockProtocol {
    pub protocol: Protocol,
    pub mocked_latency: Option<u64>,
}

impl MockProtocol {
    pub fn new(protocol: Protocol, mocked_latency: Option<u64>) -> Self {
        Self { protocol, mocked_latency }
    }
}

#[async_trait]
impl ProtocolHandler for MockProtocol {
    fn name(&self) -> Protocol {
        self.protocol
    }

    async fn test_latency(&self) -> Option<u64> {
        // Simulate network delay
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        self.mocked_latency
    }
}
