use anyhow::{Result, anyhow};
use catremote_env::Capabilities;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

#[cfg(feature = "opus")]
use opus;

#[cfg(target_os = "linux")]
mod libei_stub {
    pub struct Context;
    pub struct Seat;
    pub struct Device;
    pub enum DeviceType { Pointer, Keyboard }
    pub enum ButtonState { Pressed, Released }
    pub enum KeyState { Pressed, Released }

    impl Context {
        pub fn new(_name: &str) -> anyhow::Result<Self> { Ok(Self) }
        pub fn connect(&self) -> anyhow::Result<()> { Ok(()) }
        pub fn get_seat(&self, _name: &str) -> anyhow::Result<Seat> { Ok(Seat) }
    }
    impl Seat {
        pub fn create_device(&self, _name: &str, _types: DeviceType) -> anyhow::Result<Device> { Ok(Device) }
    }
    impl Device {
        pub fn pointer_move(&mut self, _x: f64, _y: f64) -> anyhow::Result<()> { Ok(()) }
        pub fn pointer_button(&mut self, _button: u32, _state: ButtonState) -> anyhow::Result<()> { Ok(()) }
        pub fn pointer_scroll(&mut self, _dx: f64, _dy: f64) -> anyhow::Result<()> { Ok(()) }
        pub fn keyboard_key(&mut self, _key: u32, _state: KeyState) -> anyhow::Result<()> { Ok(()) }
        pub fn keyboard_modifiers(&mut self, _mods: u32) -> anyhow::Result<()> { Ok(()) }
        pub fn touch_down(&mut self, _slot: i32, _x: f64, _y: f64) -> anyhow::Result<()> { Ok(()) }
        pub fn touch_move(&mut self, _slot: i32, _x: f64, _y: f64) -> anyhow::Result<()> { Ok(()) }
        pub fn touch_up(&mut self, _slot: i32) -> anyhow::Result<()> { Ok(()) }
        pub fn touch_frame(&mut self) -> anyhow::Result<()> { Ok(()) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: u32,
    pub frame_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    pub device_path: Option<String>,
    pub deadzone: f32,
    pub sensitivity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    pub data: Vec<u8>,
    pub pts: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputEvent {
    pub timestamp: u64,
    pub event_type: InputEventType,
    pub device_id: String,
    pub data: InputEventData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputEventType {
    PointerMove { x: f64, y: f64 },
    PointerButton { button: u32, pressed: bool },
    PointerScroll { dx: f64, dy: f64 },
    KeyboardKey { key: u32, pressed: bool },
    KeyboardModifiers { mods: u32 },
    TouchDown { slot: i32, x: f64, y: f64 },
    TouchMove { slot: i32, x: f64, y: f64 },
    TouchUp { slot: i32 },
    TouchFrame,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputEventData {
    None,
    Raw(Vec<u8>),
}

#[derive(Debug, thiserror::Error)]
pub enum AudioInputError {
    #[error("Audio capture not available: {0}")]
    AudioUnavailable(String),
    #[error("Input injection not available: {0}")]
    InputUnavailable(String),
    #[error("PipeWire error: {0}")]
    PipeWire(String),
    #[error("libei error: {0}")]
    Libei(String),
    #[error("Opus encoding error: {0}")]
    Opus(String),
    #[error("Controller mapping error: {0}")]
    Controller(String),
}

pub struct AudioEngine {
    capture: Option<Box<dyn AudioCapture>>,
    encoder: Option<OpusEncoder>,
    frame_tx: mpsc::Sender<AudioFrame>,
    frame_rx: Arc<Mutex<mpsc::Receiver<AudioFrame>>>,
    running: Arc<Mutex<bool>>,
    stats: Arc<Mutex<AudioStats>>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AudioStats {
    pub frames_captured: u64,
    pub frames_encoded: u64,
    pub frames_dropped: u64,
    pub avg_encode_latency_ms: f64,
    pub current_bitrate: u32,
    pub buffer_level: f32,
}

impl AudioEngine {
    pub async fn new(config: AudioConfig, _capabilities: &Capabilities) -> Result<Self> {
        let capture = select_audio_capture(&config).await?;
        let encoder = OpusEncoder::new(&config)?;

        let (frame_tx, frame_rx) = mpsc::channel(64);

        Ok(Self {
            capture: Some(capture),
            encoder: Some(encoder),
            frame_tx,
            frame_rx: Arc::new(Mutex::new(frame_rx)),
            running: Arc::new(Mutex::new(false)),
            stats: Arc::new(Mutex::new(AudioStats::default())),
        })
    }

    pub async fn start(&mut self) -> Result<()> {
        *self.running.lock().await = true;

        let capture = self.capture.take().unwrap();
        let mut encoder = self.encoder.take().unwrap();
        let frame_tx = self.frame_tx.clone();
        let running = self.running.clone();
        let stats = self.stats.clone();
        let frame_size = encoder.config.frame_size;

        tokio::spawn(async move {
            let mut capture = capture;
            capture.start().await.ok();
            let mut buffer = vec![0f32; frame_size * encoder.config.channels as usize];

            while *running.lock().await {
                match capture.read_frames(&mut buffer).await {
                    Ok(frames_read) if frames_read > 0 => {
                        let encode_start = Instant::now();
                        let pcm_data: Vec<i16> = buffer[..frames_read * encoder.config.channels as usize]
                            .iter()
                            .map(|&f| (f.clamp(-1.0, 1.0) * 32767.0) as i16)
                            .collect();

                        if let Ok(encoded) = encoder.encode(&pcm_data).await {
                            let encode_latency = encode_start.elapsed().as_secs_f64() * 1000.0;

                            let mut s = stats.lock().await;
                            s.frames_encoded += 1;
                            s.avg_encode_latency_ms =
                                (s.avg_encode_latency_ms * (s.frames_encoded - 1) as f64 + encode_latency)
                                    / s.frames_encoded as f64;

                            let audio_frame = AudioFrame {
                                data: encoded,
                                pts: s.frames_encoded,
                                sample_rate: encoder.config.sample_rate,
                                channels: encoder.config.channels,
                                frames: frames_read,
                            };

                            if frame_tx.send(audio_frame).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Audio capture error: {}", e);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }

            let _ = capture.stop().await;
        });

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;
        Ok(())
    }

    pub async fn next_frame(&mut self) -> Option<AudioFrame> {
        self.frame_rx.lock().await.recv().await
    }

    pub async fn stats(&self) -> AudioStats {
        self.stats.lock().await.clone()
    }
}

#[async_trait::async_trait]
trait AudioCapture: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn read_frames(&mut self, buffer: &mut [f32]) -> Result<usize>;
}

struct OpusEncoder {
    config: AudioConfig,
    #[cfg(feature = "opus")]
    encoder: opus::Encoder,
    frame_count: u64,
}

impl OpusEncoder {
    fn new(config: &AudioConfig) -> Result<Self> {
        #[cfg(feature = "opus")]
        {
            let channels = if config.channels == 1 {
                opus::Channels::Mono
            } else {
                opus::Channels::Stereo
            };
            let encoder = opus::Encoder::new(config.sample_rate, channels, opus::Application::Audio)?;
            encoder.set_bitrate(opus::Bitrate::Bits(config.bitrate))?;
            encoder.set_frame_size(config.frame_size)?;

            Ok(Self {
                config: config.clone(),
                encoder,
                frame_count: 0,
            })
        }
        #[cfg(not(feature = "opus"))]
        {
            Ok(Self {
                config: config.clone(),
                frame_count: 0,
            })
        }
    }

    async fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>> {
        self.frame_count += 1;
        #[cfg(feature = "opus")]
        {
            let mut output = vec![0u8; 4096];
            let len = self.encoder.encode(pcm, &mut output)?;
            output.truncate(len);
            Ok(output)
        }
        #[cfg(not(feature = "opus"))]
        {
            let mut output = Vec::with_capacity(pcm.len() * 2);
            for sample in pcm {
                output.extend_from_slice(&sample.to_le_bytes());
            }
            Ok(output)
        }
    }
}

async fn select_audio_capture(config: &AudioConfig) -> Result<Box<dyn AudioCapture>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(PipeWireAudioCapture::new(config).await?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(Box::new(CpalAudioCapture::new(config).await?))
    }
}

struct PipeWireAudioCapture {
    #[cfg(target_os = "linux")]
    stream: Option<pipewire::Stream>,
    #[cfg(target_os = "linux")]
    buffer: Option<pipewire::Buffer>,
    config: AudioConfig,
}

impl PipeWireAudioCapture {
    async fn new(config: &AudioConfig) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let main_loop = pipewire::MainLoop::new()?;
            let context = pipewire::Context::new(&main_loop)?;

            let mut props = pipewire::Properties::new();
            props.set(pipewire::keys::MEDIA_TYPE, "Audio");
            props.set(pipewire::keys::MEDIA_CATEGORY, "Capture");
            props.set(pipewire::keys::MEDIA_ROLE, "Music");
            props.set(pipewire::keys::TARGET_OBJECT, "0");

            let stream = pipewire::Stream::new(&context, "catremote-audio-capture", &props)?;

            Ok(Self {
                stream: Some(stream),
                buffer: None,
                config: config.clone(),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(anyhow!("PipeWire only available on Linux"))
        }
    }
}

#[async_trait::async_trait]
impl AudioCapture for PipeWireAudioCapture {
    async fn start(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let Some(stream) = &mut self.stream {
                stream.connect(
                    pipewire::stream::Direction::Input,
                    None,
                    pipewire::stream::Flags::AUTOCONNECT | pipewire::stream::Flags::MAP_BUFFERS,
                    &[],
                )?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow!("Not available"))
    }

    async fn stop(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let Some(stream) = &mut self.stream {
                stream.disconnect()?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow!("Not available"))
    }

    async fn read_frames(&mut self, buffer: &mut [f32]) -> Result<usize> {
        #[cfg(target_os = "linux")]
        {
            if let Some(stream) = &mut self.stream {
                if let Some(buffer_pw) = stream.dequeue_buffer()? {
                    let datas = buffer_pw.datas();
                    if let Some(data) = datas.first() {
                        let samples = unsafe {
                            std::slice::from_raw_parts(
                                data.data() as *const f32,
                                data.chunk().size() / 4,
                            )
                        };
                        let len = samples.len().min(buffer.len());
                        buffer[..len].copy_from_slice(&samples[..len]);
                        return Ok(len / self.config.channels as usize);
                    }
                }
            }
            Ok(0)
        }
        #[cfg(not(target_os = "linux"))]
        Ok(0)
    }
}

struct CpalAudioCapture {
    config: AudioConfig,
}

impl CpalAudioCapture {
    async fn new(config: &AudioConfig) -> Result<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            // Stub implementation for non-Linux platforms
            Ok(Self {
                config: config.clone(),
            })
        }
        #[cfg(target_os = "linux")]
        {
            Err(anyhow!("CPAL fallback not used on Linux"))
        }
    }
}

#[async_trait::async_trait]
impl AudioCapture for CpalAudioCapture {
    async fn start(&mut self) -> Result<()> {
        #[cfg(not(target_os = "linux"))]
        {
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Err(anyhow!("Not available"))
    }

    async fn stop(&mut self) -> Result<()> {
        #[cfg(not(target_os = "linux"))]
        {
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Err(anyhow!("Not available"))
    }

    async fn read_frames(&mut self, _buffer: &mut [f32]) -> Result<usize> {
        Ok(0)
    }
}

pub struct InputEngine {
    libei_context: Option<Box<dyn LibeiContext>>,
    controller_mapper: ControllerMapper,
    event_tx: mpsc::Sender<InputEvent>,
    event_rx: Arc<Mutex<mpsc::Receiver<InputEvent>>>,
    running: Arc<Mutex<bool>>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct InputStats {
    pub events_injected: u64,
    pub events_dropped: u64,
    pub avg_latency_ms: f64,
}

impl InputEngine {
    pub async fn new(config: InputConfig, _capabilities: &Capabilities) -> Result<Self> {
        let libei_context = select_libei_context().await?;
        let controller_mapper = ControllerMapper::new(config.deadzone, config.sensitivity);

        let (event_tx, event_rx) = mpsc::channel(128);

        Ok(Self {
            libei_context: Some(libei_context),
            controller_mapper,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            running: Arc::new(Mutex::new(false)),
        })
    }

    pub async fn start(&mut self) -> Result<()> {
        *self.running.lock().await = true;

        if let Some(context) = &mut self.libei_context {
            context.connect().await?;
        }

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;

        if let Some(context) = &mut self.libei_context {
            context.disconnect().await?;
        }

        Ok(())
    }

    pub async fn inject_event(&mut self, event: InputEvent) -> Result<()> {
        if let Some(context) = &mut self.libei_context {
            context.inject_event(&event).await?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub async fn inject_controller_event(&mut self, event: evdev::Event) -> Result<()> {
        let input_events = self.controller_mapper.map_event(event);
        for event in input_events {
            self.inject_event(event).await?;
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn inject_controller_event(&mut self, _event: ()) -> Result<()> {
        Err(anyhow!("Controller mapping only available on Linux"))
    }

    pub async fn next_event(&mut self) -> Option<InputEvent> {
        self.event_rx.lock().await.recv().await
    }
}

#[async_trait::async_trait]
trait LibeiContext: Send + Sync {
    async fn connect(&mut self) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;
    async fn inject_event(&mut self, event: &InputEvent) -> Result<()>;
}

struct LibeiContextImpl {
    #[cfg(target_os = "linux")]
    context: Option<libei_stub::Context>,
    #[cfg(target_os = "linux")]
    seat: Option<libei_stub::Seat>,
    #[cfg(target_os = "linux")]
    device: Option<libei_stub::Device>,
}

impl LibeiContextImpl {
    async fn new() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                context: None,
                seat: None,
                device: None,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(anyhow!("libei only available on Linux"))
        }
    }
}

#[async_trait::async_trait]
impl LibeiContext for LibeiContextImpl {
    async fn connect(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let context = libei_stub::Context::new("catremote")?;
            context.connect()?;

            let seat = context.get_seat("default")?;
            let device = seat.create_device("catremote-virtual", libei_stub::DeviceType::Pointer)?;

            self.context = Some(context);
            self.seat = Some(seat);
            self.device = Some(device);

            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow!("Not available"))
    }

    async fn disconnect(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.device = None;
            self.seat = None;
            self.context = None;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow!("Not available"))
    }

    async fn inject_event(&mut self, event: &InputEvent) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let Some(device) = &mut self.device {
                match &event.event_type {
                    InputEventType::PointerMove { x, y } => {
                        device.pointer_move(*x, *y)?;
                    }
                    InputEventType::PointerButton { button, pressed } => {
                        let state = if *pressed { libei_stub::ButtonState::Pressed } else { libei_stub::ButtonState::Released };
                        device.pointer_button(*button, state)?;
                    }
                    InputEventType::PointerScroll { dx, dy } => {
                        device.pointer_scroll(*dx, *dy)?;
                    }
                    InputEventType::KeyboardKey { key, pressed } => {
                        let state = if *pressed { libei_stub::KeyState::Pressed } else { libei_stub::KeyState::Released };
                        device.keyboard_key(*key, state)?;
                    }
                    InputEventType::KeyboardModifiers { mods } => {
                        device.keyboard_modifiers(*mods)?;
                    }
                    InputEventType::TouchDown { slot, x, y } => {
                        device.touch_down(*slot, *x, *y)?;
                    }
                    InputEventType::TouchMove { slot, x, y } => {
                        device.touch_move(*slot, *x, *y)?;
                    }
                    InputEventType::TouchUp { slot } => {
                        device.touch_up(*slot)?;
                    }
                    InputEventType::TouchFrame => {
                        device.touch_frame()?;
                    }
                }
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(anyhow!("Not available"))
    }
}

async fn select_libei_context() -> Result<Box<dyn LibeiContext>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(LibeiContextImpl::new().await?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(anyhow!("libei only available on Linux"))
    }
}

pub struct ControllerMapper {
    deadzone: f32,
    sensitivity: f32,
    axis_state: std::collections::HashMap<(u16, u16), i32>,
}

impl ControllerMapper {
    pub fn new(deadzone: f32, sensitivity: f32) -> Self {
        Self {
            deadzone,
            sensitivity,
            axis_state: std::collections::HashMap::new(),
        }
    }

    #[cfg(target_os = "linux")]
    pub fn map_event(&mut self, event: evdev::Event) -> Vec<InputEvent> {
        let mut events = Vec::new();

        match event.event_type() {
            evdev::EventType::ABSOLUTE => {
                let code = event.code();
                let value = event.value();

                match code {
                    0x00 => { // ABS_X - Left stick X
                        let normalized = self.apply_deadzone(value, 32767);
                        let x = normalized * self.sensitivity as f64;
                        events.push(InputEvent {
                            timestamp: current_timestamp(),
                            event_type: InputEventType::PointerMove { x, y: 0.0 },
                            device_id: "controller".to_string(),
                            data: InputEventData::None,
                        });
                    }
                    0x01 => { // ABS_Y - Left stick Y
                        let normalized = self.apply_deadzone(value, 32767);
                        let y = normalized * self.sensitivity as f64;
                        events.push(InputEvent {
                            timestamp: current_timestamp(),
                            event_type: InputEventType::PointerMove { x: 0.0, y },
                            device_id: "controller".to_string(),
                            data: InputEventData::None,
                        });
                    }
                    0x03 => { // ABS_RX - Right stick X
                        let normalized = self.apply_deadzone(value, 32767);
                        let dx = normalized * self.sensitivity as f64 * 0.5;
                        events.push(InputEvent {
                            timestamp: current_timestamp(),
                            event_type: InputEventType::PointerScroll { dx, dy: 0.0 },
                            device_id: "controller".to_string(),
                            data: InputEventData::None,
                        });
                    }
                    0x04 => { // ABS_RY - Right stick Y
                        let normalized = self.apply_deadzone(value, 32767);
                        let dy = normalized * self.sensitivity as f64 * 0.5;
                        events.push(InputEvent {
                            timestamp: current_timestamp(),
                            event_type: InputEventType::PointerScroll { dx: 0.0, dy },
                            device_id: "controller".to_string(),
                            data: InputEventData::None,
                        });
                    }
                    _ => {}
                }
            }
            evdev::EventType::KEY => {
                let code = event.code();
                let value = event.value();

                let pressed = value == 1;

                let button = match code {
                    0x130 => 1,  // BTN_A -> Left click
                    0x131 => 3,  // BTN_B -> Right click
                    0x133 => 2,  // BTN_X -> Middle click
                    0x134 => 4,  // BTN_Y -> Back button
                    0x136 => 5,  // BTN_TL -> Forward
                    0x137 => 6,  // BTN_TR -> Back
                    0x13a => 7,  // BTN_THUMBL -> Thumb left
                    0x13b => 8,  // BTN_THUMBR -> Thumb right
                    _ => 0,
                };

                if button != 0 {
                    events.push(InputEvent {
                        timestamp: current_timestamp(),
                        event_type: InputEventType::PointerButton {
                            button,
                            pressed,
                        },
                        device_id: "controller".to_string(),
                        data: InputEventData::None,
                    });
                }

                let key = match code {
                    0x13c => 0x3b, // BTN_SELECT -> Escape
                    0x13d => 0x3c, // BTN_START -> Enter
                    0x13e => 0x3d, // BTN_MODE -> Space
                    _ => 0,
                };

                if key != 0 {
                    events.push(InputEvent {
                        timestamp: current_timestamp(),
                        event_type: InputEventType::KeyboardKey { key, pressed },
                        device_id: "controller".to_string(),
                        data: InputEventData::None,
                    });
                }
            }
            _ => {}
        }

        events
    }

    #[cfg(not(target_os = "linux"))]
    pub fn map_event(&mut self, _event: ()) -> Vec<InputEvent> {
        Vec::new()
    }

    fn apply_deadzone(&self, value: i32, max: i32) -> f64 {
        let normalized = value as f64 / max as f64;
        if normalized.abs() < self.deadzone as f64 {
            0.0
        } else {
            let sign = normalized.signum();
            let adjusted = (normalized.abs() - self.deadzone as f64) / (1.0 - self.deadzone as f64);
            sign * adjusted
        }
    }
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}