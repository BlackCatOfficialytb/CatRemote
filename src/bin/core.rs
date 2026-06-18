use clap::Parser;
use catremote::portal;
use catremote::capture::CaptureStream;
use catremote::encoder::{Encoder, HardwareEncoder};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "catremote-core", version = "0.1.0", about = "CatRemote Capture and Encode Core CLI")]
struct Args {
    /// Record output directly to a file
    #[arg(short, long)]
    record: Option<PathBuf>,

    /// Video codec to use (h264 or hevc)
    #[arg(short, long, default_value = "h264")]
    codec: String,

    /// Target frame rate
    #[arg(short, long, default_value_t = 60)]
    fps: u32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    println!("Starting CatRemote Core Daemon...");

    // 1. Initialize DBus ScreenCast Session (Linux/Wayland only, stubs elsewhere)
    let node_id = portal::init_screencast_session().await?;
    println!("Screencast portal node ID retrieved: {}", node_id);

    // 2. Set up Encoder
    let mut encoder = HardwareEncoder::new(&args.codec, args.record.as_deref())?;
    encoder.initialize(1920, 1080, args.fps)?;

    // 3. Set up PipeWire Capture (Linux/Wayland only, stubs elsewhere)
    let stream = CaptureStream::new(node_id)?;
    stream.start()?;

    // 4. Capture & Encode Loop
    println!("Core pipeline active. Press Ctrl+C to terminate.");
    
    // Simulate framing loop when in stub/mock mode or let stream callbacks run
    let mut frame_count = 0;
    loop {
        tokio::time::sleep(Duration::from_millis((1000 / args.fps) as u64)).await;
        
        // Mock capture buffer retrieval (e.g., in Windows testing env)
        let dummy_frame = vec![0u8; 1920 * 1080 * 4]; // Dummy 1080p frame
        let packets = encoder.encode_frame(&dummy_frame)?;
        
        frame_count += 1;
        if frame_count % args.fps == 0 {
            println!("Encoded {} frames (last packet size: {} bytes)", frame_count, packets.len());
        }
    }
}
