use clap::Parser;
use catremote_core::portal;
use catremote_core::capture::CaptureStream;
use catremote_core::encoder::{Encoder, HardwareEncoder};
use catremote_core::audio::AudioCaptureStream;
use catremote_core::input::InputInjector;
use catremote_core::controller::{ControllerMapper, GamepadState};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "catremote-core", version = "0.1.0", about = "CatRemote Capture and Encode Core CLI")]
struct Args {
    #[arg(short, long)]
    record: Option<PathBuf>,

    #[arg(short, long, default_value = "h264")]
    codec: String,

    #[arg(short, long, default_value_t = 60)]
    fps: u32,

    #[arg(long, default_value_t = false)]
    enable_audio: bool,

    #[arg(long, default_value_t = false)]
    inject_inputs: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    println!("Starting CatRemote Core Daemon...");

    let node_id = portal::init_screencast_session().await?;
    println!("Screencast portal node ID retrieved: {}", node_id);

    let mut encoder = HardwareEncoder::new(&args.codec, args.record.as_deref())?;
    encoder.initialize(1920, 1080, args.fps)?;

    let stream = CaptureStream::new(node_id)?;
    stream.start()?;

    let audio_stream = if args.enable_audio {
        let stream = AudioCaptureStream::new()?;
        stream.start()?;
        Some(stream)
    } else {
        None
    };

    let input_system = if args.inject_inputs {
        let injector = InputInjector::new()?;
        let mapper = ControllerMapper::new();
        Some((injector, mapper))
    } else {
        None
    };

    println!("Core pipeline active. Press Ctrl+C to terminate.");
    
    let mut frame_count = 0;
    loop {
        tokio::time::sleep(Duration::from_millis((1000 / args.fps) as u64)).await;
        
        let dummy_frame = vec![0u8; 1920 * 1080 * 4];
        let packets = encoder.encode_frame(&dummy_frame)?;
        
        frame_count += 1;
        if frame_count % args.fps == 0 {
            println!("Encoded {} frames (last packet size: {} bytes)", frame_count, packets.len());
        }

        if let Some(ref audio) = audio_stream {
            let audio_frame = audio.capture_frame()?;
            if frame_count % args.fps == 0 {
                println!("Processed audio loopback frame (size: {} bytes)", audio_frame.len());
            }
        }

        if let Some((ref injector, ref mapper)) = input_system {
            // Periodically simulate gamepad input events
            if frame_count % (args.fps * 3) == 0 {
                println!("--- Simulating Gamepad Input Injection Event (Frame {}) ---", frame_count);
                let mock_gamepad = GamepadState {
                    left_stick_x: 0.75,
                    left_stick_y: -0.4,
                    button_a: true,
                    button_b: frame_count % (args.fps * 6) == 0,
                    ..Default::default()
                };
                mapper.map_gamepad_to_input(&mock_gamepad, injector)?;
            }
        }
    }
}

