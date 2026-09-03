use anyhow::{Result, anyhow};
use bytes::{BufMut, BytesMut};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, warn, error};

pub const MTU_SIZE: usize = 1300;
pub const MAX_DATAGRAM_SIZE: usize = 1400;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StreamType {
    Video = 0,
    Audio = 1,
    Control = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    pub bind_addr: SocketAddr,
    pub server_name: String,
    pub max_idle_timeout_ms: u64,
    pub keep_alive_interval_ms: u64,
    pub initial_mtu: usize,
    pub congestion_controller: CongestionController,
    pub enable_0rtt: bool,
    pub kem_algorithm: KemAlgorithm,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CongestionController {
    Cubic,
    Bbr,
    NewReno,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum KemAlgorithm {
    MlKem768,
    Hybrid,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            server_name: "catremote".to_string(),
            max_idle_timeout_ms: 5000,
            keep_alive_interval_ms: 1000,
            initial_mtu: MTU_SIZE,
            congestion_controller: CongestionController::Cubic,
            enable_0rtt: true,
            kem_algorithm: KemAlgorithm::Hybrid,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtpHeader {
    pub version: u8,
    pub padding: bool,
    pub extension: bool,
    pub csrc_count: u8,
    pub marker: bool,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

impl RtpHeader {
    pub fn new(payload_type: u8, sequence_number: u16, timestamp: u32, ssrc: u32, marker: bool) -> Self {
        Self {
            version: 2,
            padding: false,
            extension: false,
            csrc_count: 0,
            marker,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
        }
    }

    pub fn encode(&self) -> [u8; 12] {
        let mut buf = [0u8; 12];
        buf[0] = (self.version << 6) | (if self.padding { 1 << 5 } else { 0 }) | (if self.extension { 1 << 4 } else { 0 }) | self.csrc_count;
        buf[1] = (if self.marker { 1 << 7 } else { 0 }) | self.payload_type;
        buf[2..4].copy_from_slice(&self.sequence_number.to_be_bytes());
        buf[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        buf[8..12].copy_from_slice(&self.ssrc.to_be_bytes());
        buf
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 12 { return None; }
        Some(Self {
            version: data[0] >> 6,
            padding: (data[0] & 0x20) != 0,
            extension: (data[0] & 0x10) != 0,
            csrc_count: data[0] & 0x0F,
            marker: (data[1] & 0x80) != 0,
            payload_type: data[1] & 0x7F,
            sequence_number: u16::from_be_bytes([data[2], data[3]]),
            timestamp: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            ssrc: u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NaluHeader {
    pub forbidden_zero_bit: bool,
    pub nal_ref_idc: u8,
    pub nal_unit_type: u8,
}

impl NaluHeader {
    pub fn from_byte(byte: u8) -> Self {
        Self {
            forbidden_zero_bit: (byte & 0x80) != 0,
            nal_ref_idc: (byte & 0x60) >> 5,
            nal_unit_type: byte & 0x1F,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TransportStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_lost: u64,
    pub rtt_ms: f64,
    pub congestion_window: u64,
    pub active_connections: usize,
    pub video_frames_sent: u64,
    pub audio_frames_sent: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlMessage {
    Pli,
    Nack { sequence_numbers: Vec<u16> },
    BitrateRequest { bitrate: u32 },
    KeyframeRequest,
    StatsRequest,
    StatsResponse(TransportStats),
}

fn find_next_nalu(data: &[u8], start: usize) -> Option<usize> {
    for i in start..data.len().saturating_sub(3) {
        if data[i] == 0 && data[i+1] == 0 && data[i+2] == 1 {
            return Some(i);
        }
        if i + 3 < data.len() && data[i] == 0 && data[i+1] == 0 && data[i+2] == 0 && data[i+3] == 1 {
            return Some(i);
        }
    }
    None
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use super::*;
    use quinn::{Endpoint, ServerConfig, ClientConfig, Connection, SendStream, RecvStream};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::fs;

    pub struct TransportEngine {
        config: TransportConfig,
        endpoint: Option<Endpoint>,
        connections: Arc<Mutex<HashMap<SocketAddr, ConnectionHandle>>>,
        stats: Arc<Mutex<TransportStats>>,
        running: Arc<Mutex<bool>>,
    }

    struct ConnectionHandle {
        connection: Connection,
        video_stream: Option<SendStream>,
        audio_stream: Option<SendStream>,
        control_stream: Option<SendStream>,
        video_recv: Option<RecvStream>,
        audio_recv: Option<RecvStream>,
        control_recv: Option<RecvStream>,
        remote_addr: SocketAddr,
        video_seq: u16,
        audio_seq: u16,
        video_timestamp: u32,
        audio_timestamp: u32,
        video_ssrc: u32,
        audio_ssrc: u32,
        last_pli: Instant,
        pending_nacks: Vec<u16>,
    }

    impl TransportEngine {
        pub fn new(config: TransportConfig) -> Self {
            Self {
                config,
                endpoint: None,
                connections: Arc::new(Mutex::new(HashMap::new())),
                stats: Arc::new(Mutex::new(TransportStats::default())),
                running: Arc::new(Mutex::new(false)),
            }
        }

        pub async fn start_server(&mut self) -> Result<SocketAddr> {
            let server_config = self.make_server_config()?;
            let endpoint = Endpoint::server(server_config, self.config.bind_addr)?;
            let local_addr = endpoint.local_addr()?;
            info!("Transport server listening on {}", local_addr);
            self.endpoint = Some(endpoint);
            *self.running.lock() = true;
            self.accept_loop().await;
            Ok(local_addr)
        }

        pub async fn connect(&mut self, addr: SocketAddr) -> Result<Connection> {
            let client_config = self.make_client_config()?;
            let endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
            endpoint.set_default_client_config(client_config);
            
            let connection = endpoint.connect(addr, &self.config.server_name)?.await?;
            info!("Connected to {}", addr);
            
            self.setup_connection_streams(&connection).await?;
            
            if self.endpoint.is_none() {
                self.endpoint = Some(endpoint);
            }
            *self.running.lock() = true;
            
            Ok(connection)
        }

        async fn accept_loop(&self) {
            let endpoint = self.endpoint.as_ref().unwrap().clone();
            let connections = self.connections.clone();
            let stats = self.stats.clone();
            let running = self.running.clone();
            let config = self.config.clone();

            tokio::spawn(async move {
                while *running.lock() {
                    if let Some(incoming) = endpoint.accept().await {
                        let conn = match incoming.await {
                            Ok(c) => c,
                            Err(e) => {
                                debug!("Connection failed: {}", e);
                                continue;
                            }
                        };
                        
                        info!("Accepted connection from {}", conn.remote_address());
                        
                        if let Err(e) = Self::setup_connection_streams(&conn).await {
                            error!("Failed to setup streams: {}", e);
                            continue;
                        }
                        
                        connections.lock().insert(conn.remote_address(), ConnectionHandle::new(conn));
                        stats.lock().active_connections += 1;
                    }
                }
            });
        }

        async fn setup_connection_streams(conn: &Connection) -> Result<()> {
            let (video_send, video_recv) = conn.open_bi().await?;
            let (audio_send, audio_recv) = conn.open_bi().await?;
            let (control_send, control_recv) = conn.open_bi().await?;
            
            tokio::spawn(Self::handle_control_stream(control_send, control_recv));
            
            Ok(())
        }

        async fn handle_control_stream(mut send: SendStream, mut recv: RecvStream) -> Result<()> {
            let mut buf = vec![0u8; 4096];
            loop {
                match recv.read_to_end(4096).await {
                    Ok(data) if !data.is_empty() => {
                        if let Ok(msg) = serde_json::from_slice::<ControlMessage>(&data) {
                            match msg {
                                ControlMessage::Pli => {
                                    debug!("Received PLI, requesting keyframe");
                                }
                                ControlMessage::Nack { sequence_numbers } => {
                                    debug!("Received NACK for sequences: {:?}", sequence_numbers);
                                }
                                ControlMessage::BitrateRequest { bitrate } => {
                                    debug!("Received bitrate request: {}", bitrate);
                                }
                                ControlMessage::KeyframeRequest => {
                                    debug!("Received keyframe request");
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(_) => break,
                    Err(e) => {
                        debug!("Control stream error: {}", e);
                        break;
                    }
                }
            }
            Ok(())
        }

        pub async fn send_video_frame(&self, data: &[u8], timestamp: u64, is_keyframe: bool) -> Result<()> {
            let connections = self.connections.lock();
            for handle in connections.values() {
                self.send_video_frame_to_handle(handle, data, timestamp, is_keyframe).await?;
            }
            Ok(())
        }

        async fn send_video_frame_to_handle(&self, handle: &ConnectionHandle, data: &[u8], timestamp: u64, is_keyframe: bool) -> Result<()> {
            if handle.video_stream.is_none() { return Ok(()); }
            
            let mut nalu_start = 0;
            let mut seq = handle.video_seq;
            let ts = (timestamp / 1000) as u32;
            
            while nalu_start < data.len() {
                let nalu_end = find_next_nalu(data, nalu_start + 1).unwrap_or(data.len());
                let nalu = &data[nalu_start..nalu_end];
                
                if nalu.len() + 12 <= MTU_SIZE {
                    let header = RtpHeader::new(96, seq, ts, handle.video_ssrc, nalu_end == data.len());
                    let mut packet = BytesMut::with_capacity(nalu.len() + 12);
                    packet.extend_from_slice(&header.encode());
                    packet.extend_from_slice(nalu);
                    
                    if let Some(stream) = &handle.video_stream {
                        stream.write_all(&packet).await?;
                    }
                } else {
                    self.send_fua(handle, nalu, seq, ts, is_keyframe).await?;
                }
                
                seq = seq.wrapping_add(1);
                nalu_start = nalu_end;
            }
            
            let mut stats = self.stats.lock();
            stats.bytes_sent += data.len() as u64;
            stats.packets_sent += 1;
            stats.video_frames_sent += 1;
            
            Ok(())
        }

        async fn send_fua(&self, handle: &ConnectionHandle, nalu: &[u8], seq: u16, ts: u32, is_keyframe: bool) -> Result<()> {
            let header_byte = nalu[0];
            let nal_header = NaluHeader::from_byte(header_byte);
            
            let mut offset = 1;
            let mut remaining = nalu.len() - 1;
            let mut first = true;
            
            while remaining > 0 {
                let payload_size = std::cmp::min(remaining, MTU_SIZE - 14);
                let mut packet = BytesMut::with_capacity(payload_size + 14);
                
                let fu_indicator = (header_byte & 0xE0) | 28;
                let fu_header = if first {
                    0x80 | nal_header.nal_unit_type
                } else if remaining <= payload_size {
                    0x40 | nal_header.nal_unit_type
                } else {
                    0x00 | nal_header.nal_unit_type
                };
                
                let rtp_header = RtpHeader::new(96, seq, ts, handle.video_ssrc, remaining <= payload_size);
                packet.extend_from_slice(&rtp_header.encode());
                packet.put_u8(fu_indicator);
                packet.put_u8(fu_header);
                packet.extend_from_slice(&nalu[offset..offset + payload_size]);
                
                if let Some(stream) = &handle.video_stream {
                    stream.write_all(&packet).await?;
                }
                
                first = false;
                offset += payload_size;
                remaining -= payload_size;
                seq = seq.wrapping_add(1);
            }
            
            Ok(())
        }

        pub async fn send_audio_frame(&self, data: &[u8], timestamp: u64) -> Result<()> {
            let connections = self.connections.lock();
            for handle in connections.values() {
                if handle.audio_stream.is_none() { continue; }
                
                let header = RtpHeader::new(97, handle.audio_seq, (timestamp / 1000) as u32, handle.audio_ssrc, true);
                let mut packet = BytesMut::with_capacity(data.len() + 12);
                packet.extend_from_slice(&header.encode());
                packet.extend_from_slice(data);
                
                if let Some(stream) = &handle.audio_stream {
                    stream.write_all(&packet).await?;
                }
                
                let mut stats = self.stats.lock();
                stats.bytes_sent += data.len() as u64;
                stats.packets_sent += 1;
                stats.audio_frames_sent += 1;
            }
            Ok(())
        }

        pub async fn send_control(&self, msg: ControlMessage) -> Result<()> {
            let data = serde_json::to_vec(&msg)?;
            let connections = self.connections.lock();
            for handle in connections.values() {
                if let Some(stream) = &handle.control_stream {
                    stream.write_all(&data).await?;
                }
            }
            Ok(())
        }

        pub async fn request_keyframe(&self) -> Result<()> {
            self.send_control(ControlMessage::KeyframeRequest).await
        }

        pub async fn send_pli(&self) -> Result<()> {
            self.send_control(ControlMessage::Pli).await
        }

        pub async fn send_nack(&self, sequences: Vec<u16>) -> Result<()> {
            self.send_control(ControlMessage::Nack { sequence_numbers: sequences }).await
        }

        pub async fn request_bitrate(&self, bitrate: u32) -> Result<()> {
            self.send_control(ControlMessage::BitrateRequest { bitrate }).await
        }

        pub async fn stats(&self) -> TransportStats {
            self.stats.lock().clone()
        }

        pub async fn stop(&mut self) -> Result<()> {
            *self.running.lock() = false;
            if let Some(endpoint) = self.endpoint.take() {
                endpoint.close(0u32.into(), b"shutdown");
            }
            self.connections.lock().clear();
            Ok(())
        }

        fn make_server_config(&self) -> Result<ServerConfig> {
            let (cert_der, key_der) = load_or_generate_cert(&self.config.server_name)?;
            
            let mut cfg = ServerConfig::with_single_cert(vec![cert_der], key_der)?;
            cfg.max_idle_timeout(Some(self.max_idle_timeout()));
            cfg.transport_config(Arc::new(self.make_transport_config()));
            Ok(cfg)
        }

        fn make_client_config(&self) -> Result<ClientConfig> {
            let mut roots = rustls::RootCertStore::empty();
            let (cert_der, _) = load_or_generate_cert("catremote")?;
            roots.add(cert_der)?;
            
            let mut cfg = ClientConfig::with_root_certificates(roots)?;
            cfg.max_idle_timeout(Some(self.max_idle_timeout()));
            cfg.transport_config(Arc::new(self.make_transport_config()));
            Ok(cfg)
        }

        fn make_transport_config(&self) -> quinn::TransportConfig {
            let mut cfg = quinn::TransportConfig::default();
            cfg.max_idle_timeout(Some(self.max_idle_timeout()));
            cfg.keep_alive_interval(Some(Duration::from_millis(self.config.keep_alive_interval_ms)));
            cfg.initial_mtu(self.config.initial_mtu);
            cfg.max_datagram_frame_size(MAX_DATAGRAM_SIZE);
            cfg
        }

        fn max_idle_timeout(&self) -> quinn::IdleTimeout {
            quinn::IdleTimeout::try_from(Duration::from_millis(self.config.max_idle_timeout_ms)).unwrap()
        }
    }

    impl ConnectionHandle {
        fn new(conn: Connection) -> Self {
            Self {
                connection: conn,
                video_stream: None,
                audio_stream: None,
                control_stream: None,
                video_recv: None,
                audio_recv: None,
                control_recv: None,
                remote_addr: conn.remote_address(),
                video_seq: 0,
                audio_seq: 0,
                video_timestamp: 0,
                audio_timestamp: 0,
                video_ssrc: rand::random(),
                audio_ssrc: rand::random(),
                last_pli: Instant::now(),
                pending_nacks: Vec::new(),
            }
        }
    }

    fn load_or_generate_cert(name: &str) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
        let cert_path = format!("/tmp/catremote_{}.cert", name);
        let key_path = format!("/tmp/catremote_{}.key", name);
        
        if fs::metadata(&cert_path).is_ok() && fs::metadata(&key_path).is_ok() {
            let cert_pem = fs::read(&cert_path)?;
            let key_pem = fs::read(&key_path)?;
            let cert_der = CertificateDer::from(pem::parse(&cert_pem)?.contents);
            let key_der = PrivateKeyDer::from(pem::parse(&key_pem)?.contents);
            return Ok((cert_der, key_der));
        }
        
        let cert = rcgen::generate_simple_self_signed(vec![name.to_string()])?;
        let cert_der = CertificateDer::from(cert.cert);
        let key_der = PrivateKeyDer::from(cert.key_pair.serialize_der());
        
        fs::write(&cert_path, pem::encode(&pem::Pem::new("CERTIFICATE", cert_der.as_ref())))?;
        fs::write(&key_path, pem::encode(&pem::Pem::new("PRIVATE KEY", key_der.as_ref())))?;
        
        Ok((cert_der, key_der))
    }

    pub async fn run_transport_server(config: TransportConfig) -> Result<()> {
        let mut engine = TransportEngine::new(config);
        let addr = engine.start_server().await?;
        info!("Server running on {}", addr);
        
        tokio::signal::ctrl_c().await?;
        engine.stop().await?;
        Ok(())
    }

    pub async fn run_transport_client(config: TransportConfig, server_addr: SocketAddr) -> Result<()> {
        let mut engine = TransportEngine::new(config);
        let _conn = engine.connect(server_addr).await?;
        info!("Connected to server");
        
        tokio::signal::ctrl_c().await?;
        engine.stop().await?;
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
mod stub_impl {
    use super::*;
    use std::net::SocketAddr;

    pub struct TransportEngine {
        config: TransportConfig,
        running: Arc<Mutex<bool>>,
        stats: Arc<Mutex<TransportStats>>,
    }

    impl TransportEngine {
        pub fn new(config: TransportConfig) -> Self {
            Self {
                config,
                running: Arc::new(Mutex::new(false)),
                stats: Arc::new(Mutex::new(TransportStats::default())),
            }
        }

        pub async fn start_server(&mut self) -> Result<SocketAddr> {
            *self.running.lock() = true;
            Ok(self.config.bind_addr)
        }

        pub async fn connect(&mut self, _addr: SocketAddr) -> Result<()> {
            *self.running.lock() = true;
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn send_video_frame(&self, _data: &[u8], _timestamp: u64, _is_keyframe: bool) -> Result<()> {
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn send_audio_frame(&self, _data: &[u8], _timestamp: u64) -> Result<()> {
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn send_control(&self, _msg: ControlMessage) -> Result<()> {
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn request_keyframe(&self) -> Result<()> {
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn send_pli(&self) -> Result<()> {
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn send_nack(&self, _sequences: Vec<u16>) -> Result<()> {
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn request_bitrate(&self, _bitrate: u32) -> Result<()> {
            Err(anyhow!("QUIC transport not supported on this platform (stub)"))
        }

        pub async fn stats(&self) -> TransportStats {
            self.stats.lock().clone()
        }

        pub async fn stop(&mut self) -> Result<()> {
            *self.running.lock() = false;
            Ok(())
        }
    }

    pub async fn run_transport_server(_config: TransportConfig) -> Result<()> {
        Err(anyhow!("QUIC transport not supported on this platform (stub)"))
    }

    pub async fn run_transport_client(_config: TransportConfig, _server_addr: SocketAddr) -> Result<()> {
        Err(anyhow!("QUIC transport not supported on this platform (stub)"))
    }
}

#[cfg(target_os = "linux")]
pub use linux_impl::{TransportEngine, run_transport_server, run_transport_client};

#[cfg(not(target_os = "linux"))]
pub use stub_impl::{TransportEngine, run_transport_server, run_transport_client};