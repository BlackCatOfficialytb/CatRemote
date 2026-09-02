use anyhow::Result;
use catremote_env::Capabilities;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderConfig {
    pub codec: Codec,
    pub bitrate: u32,
    pub fps: u32,
    pub keyframe_interval: u32,
    pub width: u32,
    pub height: u32,
    pub profile: Option<String>,
    pub preset: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Codec {
    H264,
    HEVC,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub pts: u64,
    pub dts: u64,
    pub is_keyframe: bool,
    pub frame_type: FrameType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FrameType {
    IDR,
    I,
    P,
    B,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("No suitable capture backend found")]
    NoCaptureBackend,
    #[error("No suitable encoder found: {0}")]
    NoEncoder(String),
    #[error("DMA-BUF import failed: {0}")]
    DmaBufImport(String),
    #[error("Encoder error: {0}")]
    Encoder(String),
    #[error("Capture error: {0}")]
    Capture(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

pub struct CaptureEngine {
    backend: Option<Box<dyn CaptureBackend>>,
    encoder: Option<Box<dyn Encoder>>,
    frame_tx: mpsc::Sender<EncodedFrame>,
    frame_rx: Arc<Mutex<mpsc::Receiver<EncodedFrame>>>,
    running: Arc<Mutex<bool>>,
    stats: Arc<Mutex<EngineStats>>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EngineStats {
    pub frames_captured: u64,
    pub frames_encoded: u64,
    pub frames_dropped: u64,
    pub avg_encode_latency_ms: f64,
    pub current_bitrate: u32,
    pub current_fps: f64,
}

impl CaptureEngine {
    pub async fn new(config: EncoderConfig, capabilities: &Capabilities) -> Result<Self> {
        let backend = select_capture_backend(capabilities).await?;
        let encoder = select_encoder(&config, capabilities).await?;

        let (frame_tx, frame_rx) = mpsc::channel(32);

        Ok(Self {
            backend: Some(backend),
            encoder: Some(encoder),
            frame_tx,
            frame_rx: Arc::new(Mutex::new(frame_rx)),
            running: Arc::new(Mutex::new(false)),
            stats: Arc::new(Mutex::new(EngineStats::default())),
        })
    }

    pub async fn start(&mut self) -> Result<()> {
        *self.running.lock().await = true;

        let mut backend = self.backend.take().unwrap();
        let mut encoder = self.encoder.take().unwrap();

        backend.start().await?;
        encoder.start().await?;

        let frame_tx = self.frame_tx.clone();
        let running = self.running.clone();
        let stats = self.stats.clone();

        tokio::spawn(async move {
            let frame_interval = Duration::from_millis(1000 / 60);
            let mut last_frame = Instant::now();

            while *running.lock().await {
                let now = Instant::now();
                if now.duration_since(last_frame) >= frame_interval {
                    if let Ok(frame) = backend.capture_frame().await {
                        let encode_start = Instant::now();
                        if let Ok(encoded) = encoder.encode(frame).await {
                            let encode_latency = encode_start.elapsed().as_secs_f64() * 1000.0;

                            let mut s = stats.lock().await;
                            s.frames_encoded += 1;
                            s.avg_encode_latency_ms = (s.avg_encode_latency_ms * (s.frames_encoded - 1) as f64 + encode_latency) / s.frames_encoded as f64;

                            if frame_tx.send(encoded).await.is_err() {
                                break;
                            }
                        }
                    }
                    last_frame = now;
                }

                tokio::time::sleep(Duration::from_millis(1)).await;
            }

            let _ = backend.stop().await;
            let _ = encoder.stop().await;
        });

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;
        Ok(())
    }

    pub async fn next_frame(&mut self) -> Option<EncodedFrame> {
        self.frame_rx.lock().await.recv().await
    }

    pub async fn stats(&self) -> EngineStats {
        self.stats.lock().await.clone()
    }

    pub async fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if let Some(encoder) = &mut self.encoder {
            encoder.set_bitrate(bitrate).await?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
trait CaptureBackend: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn capture_frame(&mut self) -> Result<CapturedFrame>;
}

#[async_trait::async_trait]
trait Encoder: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn encode(&mut self, frame: CapturedFrame) -> Result<EncodedFrame>;
    async fn set_bitrate(&mut self, bitrate: u32) -> Result<()>;
}

struct CapturedFrame {
    dma_buf_fd: i32,
    width: u32,
    height: u32,
    format: u32,
    modifier: u64,
    timestamp: u64,
}

async fn select_capture_backend(capabilities: &Capabilities) -> Result<Box<dyn CaptureBackend>> {
    if capabilities.compositor.ext_image_capture_source_v1 {
        return Ok(Box::new(ExtImageCaptureSourceBackend::new().await?));
    }
    if capabilities.compositor.wlr_screencopy_v1 {
        return Ok(Box::new(WlrScreencopyBackend::new().await?));
    }
    Err(CoreError::NoCaptureBackend.into())
}

async fn select_encoder(config: &EncoderConfig, capabilities: &Capabilities) -> Result<Box<dyn Encoder>> {
    if capabilities.gpu_encoders.vaapi.available {
        return Ok(Box::new(VaapiEncoder::new(config).await?));
    }
    if capabilities.gpu_encoders.nvenc.available {
        return Ok(Box::new(NvencEncoder::new(config).await?));
    }
    Err(CoreError::NoEncoder("No VA-API or NVENC available".to_string()).into())
}

struct ExtImageCaptureSourceBackend {
    #[cfg(target_os = "linux")]
    manager: Option<wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_manager_v1::ExtImageCaptureSourceManagerV1>,
    #[cfg(target_os = "linux")]
    source: Option<wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1>,
    #[cfg(target_os = "linux")]
    queue: Option<wayland_client::EventQueue<()>>,
}

impl ExtImageCaptureSourceBackend {
    async fn new() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let conn = wayland_client::Connection::connect_to_env()?;
            let mut event_queue = conn.new_event_queue();
            let qh = event_queue.handle();

            let display = conn.display();
            let registry = display.get_registry(&qh, ());
            event_queue.roundtrip(&mut ())?;

            let globals = registry.contents::<wayland_client::protocol::wl_registry::WlRegistry>(&qh);
            let mut manager = None;

            if let Some(reg) = globals {
                for (name, interface, version) in reg.globals() {
                    if interface == "ext_image_capture_source_manager_v1" {
                        manager = Some(registry.bind(&qh, name, version.min(1), ())?);
                        break;
                    }
                }
            }

            Ok(Self {
                manager,
                source: None,
                queue: Some(event_queue),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(anyhow::anyhow!("ext-image-capture-source only available on Linux"))
        }
    }
}

#[async_trait::async_trait]
impl CaptureBackend for ExtImageCaptureSourceBackend {
    async fn start(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let (Some(manager), Some(queue)) = (&self.manager, &self.queue) {
                let qh = queue.handle();
                let source = manager.create_source(&qh, ())?;
                self.source = Some(source);
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }

    async fn stop(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.source = None;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }

    async fn capture_frame(&mut self) -> Result<CapturedFrame> {
        #[cfg(target_os = "linux")]
        {
            if let Some(queue) = &mut self.queue {
                queue.roundtrip(&mut ())?;
            }
            Err(anyhow::anyhow!("Not fully implemented"))
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }
}

struct WlrScreencopyBackend {
    #[cfg(target_os = "linux")]
    manager: Option<wayland_protocols::wlr::unstable::screencopy::v1::client::wlr_screencopy_manager_v1::WlrScreencopyManagerV1>,
    #[cfg(target_os = "linux")]
    frame: Option<wayland_protocols::wlr::unstable::screencopy::v1::client::wlr_screencopy_frame_v1::WlrScreencopyFrameV1>,
    #[cfg(target_os = "linux")]
    queue: Option<wayland_client::EventQueue<()>>,
}

impl WlrScreencopyBackend {
    async fn new() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let conn = wayland_client::Connection::connect_to_env()?;
            let mut event_queue = conn.new_event_queue();
            let qh = event_queue.handle();

            let display = conn.display();
            let registry = display.get_registry(&qh, ());
            event_queue.roundtrip(&mut ())?;

            let globals = registry.contents::<wayland_client::protocol::wl_registry::WlRegistry>(&qh);
            let mut manager = None;

            if let Some(reg) = globals {
                for (name, interface, version) in reg.globals() {
                    if interface == "wlr_screencopy_manager_v1" {
                        manager = Some(registry.bind(&qh, name, version.min(1), ())?);
                        break;
                    }
                }
            }

            Ok(Self {
                manager,
                frame: None,
                queue: Some(event_queue),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(anyhow::anyhow!("wlr-screencopy only available on Linux"))
        }
    }
}

#[async_trait::async_trait]
impl CaptureBackend for WlrScreencopyBackend {
    async fn start(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let (Some(manager), Some(queue)) = (&self.manager, &self.queue) {
                let qh = queue.handle();
                let output = manager.get_output(&qh, ())?;
                let frame = output.capture(&qh, ())?;
                self.frame = Some(frame);
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }

    async fn stop(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.frame = None;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }

    async fn capture_frame(&mut self) -> Result<CapturedFrame> {
        #[cfg(target_os = "linux")]
        {
            if let Some(queue) = &mut self.queue {
                queue.roundtrip(&mut ())?;
            }
            Err(anyhow::anyhow!("Not fully implemented"))
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }
}

struct VaapiEncoder {
    #[cfg(target_os = "linux")]
    display: Option<libva::Display>,
    #[cfg(target_os = "linux")]
    config_id: Option<i32>,
    #[cfg(target_os = "linux")]
    context_id: Option<i32>,
    config: EncoderConfig,
    frame_count: u64,
}

impl VaapiEncoder {
    async fn new(_config: &EncoderConfig) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let display = libva::Display::open()?;
            let mut major = 0;
            let mut minor = 0;
            unsafe { libva::vaInitialize(display.as_raw(), &mut major, &mut minor) };

            Ok(Self {
                display: Some(display),
                config_id: None,
                context_id: None,
                config: config.clone(),
                frame_count: 0,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(anyhow::anyhow!("VA-API only available on Linux"))
        }
    }
}

#[async_trait::async_trait]
impl Encoder for VaapiEncoder {
    async fn start(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let display = self.display.as_ref().unwrap();

            let profile = match self.config.codec {
                Codec::H264 => libva::VAProfile::VAProfileH264Main,
                Codec::HEVC => libva::VAProfile::VAProfileHEVCMain,
            };

            let entrypoint = libva::VAEntrypoint::VAEntrypointEncSlice;
            let config_id = display.create_config(profile, entrypoint, &[])?;
            self.config_id = Some(config_id);

            let context_id = display.create_context(config_id, self.config.width as i32, self.config.height as i32, 0, &[])?;
            self.context_id = Some(context_id);

            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }

    async fn stop(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let (Some(display), Some(context_id)) = (&self.display, self.context_id) {
                display.destroy_context(context_id)?;
            }
            if let (Some(display), Some(config_id)) = (&self.display, self.config_id) {
                display.destroy_config(config_id)?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }

    async fn encode(&mut self, _frame: CapturedFrame) -> Result<EncodedFrame> {
        #[cfg(target_os = "linux")]
        {
            self.frame_count += 1;
            let pts = self.frame_count;

            let display = self.display.as_ref().unwrap();
            let context_id = self.context_id.unwrap();

            let mut surface = display.create_surface(self.config.width as i32, self.config.height as i32, libva::VA_RT_FORMAT_YUV420, 0, &[])?;

            let dma_buf = unsafe { nix::sys::mman::mmap(
                std::ptr::null_mut(),
                4096,
                nix::sys::mman::ProtFlags::PROT_READ | nix::sys::mman::ProtFlags::PROT_WRITE,
                nix::sys::mman::MapFlags::MAP_SHARED,
                frame.dma_buf_fd,
                0,
            )? };

            let _ = display.put_surface(&mut surface, &[])?;

            let mut coded_buffer = display.create_buffer(1024 * 1024, &[])?;
            let mut params = vec![
                libva::VAEncSliceParameterBuffer {
                    slice_data_size: 0,
                    slice_data_offset: 0,
                    slice_data_flag: 0,
                    slice_data: std::ptr::null_mut(),
                }.into(),
            ];

            let mut codedbuf = libva::VACodedBufferSegment {
                buf: coded_buffer.as_raw(),
                offset: 0,
                size: 0,
                status: 0,
                reserved: [0; 4],
            };

            let status = unsafe {
                libva::vaEncodePicture(
                    display.as_raw(),
                    context_id,
                    surface.as_raw(),
                    params.as_mut_ptr() as *mut _,
                    params.len() as i32,
                    &mut codedbuf as *mut _,
                )
            };

            if status != libva::VA_STATUS_SUCCESS {
                return Err(anyhow::anyhow!("VA encode failed: {}", status));
            }

            let data = vec![0u8; codedbuf.size as usize];
            Ok(EncodedFrame {
                data,
                pts,
                dts: pts,
                is_keyframe: self.frame_count % self.config.keyframe_interval as u64 == 1,
                frame_type: if self.frame_count % self.config.keyframe_interval as u64 == 1 { FrameType::IDR } else { FrameType::P },
            })
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow::anyhow!("Not available"))
    }

    async fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        self.config.bitrate = bitrate;
        #[cfg(target_os = "linux")]
        {
            if let (Some(display), Some(config_id)) = (&self.display, self.config_id) {
                let mut attr = libva::VAConfigAttrib {
                    type_: libva::VAConfigAttribType::VAConfigAttribRateControl,
                    value: bitrate as i32,
                };
                display.set_config_attributes(config_id, &mut [attr])?;
            }
        }
        Ok(())
    }
}

struct NvencEncoder {
    config: EncoderConfig,
    frame_count: u64,
}

impl NvencEncoder {
    async fn new(config: &EncoderConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            frame_count: 0,
        })
    }
}

#[async_trait::async_trait]
impl Encoder for NvencEncoder {
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    async fn encode(&mut self, _frame: CapturedFrame) -> Result<EncodedFrame> {
        self.frame_count += 1;
        let pts = self.frame_count;
        Ok(EncodedFrame {
            data: vec![],
            pts,
            dts: pts,
            is_keyframe: self.frame_count % self.config.keyframe_interval as u64 == 1,
            frame_type: if self.frame_count % self.config.keyframe_interval as u64 == 1 { FrameType::IDR } else { FrameType::P },
        })
    }

    async fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        self.config.bitrate = bitrate;
        Ok(())
    }
}

pub async fn run_cli_record(output: &str, config: EncoderConfig) -> Result<()> {
    let capabilities = catremote_env::detect_capabilities().await?;
    let mut engine = CaptureEngine::new(config, &capabilities).await?;
    engine.start().await?;

    let mut file = tokio::fs::File::create(output).await?;
    let mut frame_count = 0;

    while let Some(frame) = engine.next_frame().await {
        tokio::io::AsyncWriteExt::write_all(&mut file, &frame.data).await?;
        frame_count += 1;
        if frame_count % 60 == 0 {
            println!("Recorded {} frames", frame_count);
        }
    }

    engine.stop().await?;
    println!("Recording complete: {} frames", frame_count);
    Ok(())
}