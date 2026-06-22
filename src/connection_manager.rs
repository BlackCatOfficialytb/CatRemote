use crate::protocol::{Protocol, ProtocolHandler, MockProtocol};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ConnectionManager {
    handlers: Vec<Arc<dyn ProtocolHandler>>,
    pub active_protocol: Arc<RwLock<Option<Protocol>>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        // Initialize with mocked handlers for now
        let handlers: Vec<Arc<dyn ProtocolHandler>> = vec![
            Arc::new(MockProtocol::new(Protocol::UDP, Some(35))),
            Arc::new(MockProtocol::new(Protocol::TCP, Some(45))),
            Arc::new(MockProtocol::new(Protocol::WebRTC, Some(60))),
            Arc::new(MockProtocol::new(Protocol::WebSocket, Some(50))),
            Arc::new(MockProtocol::new(Protocol::SPICE, None)), // unavailable
            Arc::new(MockProtocol::new(Protocol::RDP, None)),
            Arc::new(MockProtocol::new(Protocol::VNC, Some(100))),
        ];

        Self {
            handlers,
            active_protocol: Arc::new(RwLock::new(None)),
        }
    }

    pub fn start_background_testing(&self) {
        let handlers = self.handlers.clone();
        let active_protocol = self.active_protocol.clone();

        tokio::spawn(async move {
            loop {
                println!("--- Starting Protocol Latency Tests ---");
                
                let mut best_protocol = Protocol::WebRTC;
                let mut lowest_latency = u64::MAX;

                // Run tests concurrently
                let mut handles = Vec::new();
                for handler in &handlers {
                    let h = handler.clone();
                    handles.push(tokio::spawn(async move {
                        let latency = h.test_latency().await;
                        (h.name(), latency)
                    }));
                }

                for handle in handles {
                    if let Ok((proto, latency)) = handle.await {
                        if let Some(lat) = latency {
                            println!("Protocol {} tested at {}ms", proto, lat);
                            if lat < lowest_latency {
                                lowest_latency = lat;
                                best_protocol = proto;
                            }
                        } else {
                            println!("Protocol {} is unavailable", proto);
                        }
                    }
                }

                // Decision logic: if the best is > 30ms, fallback to WebRTC
                let selected = if lowest_latency > 30 {
                    println!("Lowest latency is {}ms (>30ms). Defaulting to WebRTC.", lowest_latency);
                    Protocol::WebRTC
                } else {
                    println!("Lowest latency is {}ms (<=30ms). Selecting {}.", lowest_latency, best_protocol);
                    best_protocol
                };

                let mut current = active_protocol.write().await;
                *current = Some(selected);

                println!("--- Next test in 30 seconds ---");
                // Wait 30 seconds before re-testing
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    }
}
