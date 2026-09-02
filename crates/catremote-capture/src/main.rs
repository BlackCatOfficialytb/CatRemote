use catremote_capture::{run_cli_record, EncoderConfig, Codec};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "catremote-core", version, about = "Core capture and encode engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Record screen to file
    Record {
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,

        /// Codec to use
        #[arg(short, long, value_enum, default_value = "hevc")]
        codec: CliCodec,

        /// Bitrate in kbps
        #[arg(short, long, default_value = "20000")]
        bitrate: u32,

        /// Frame rate
        #[arg(short, long, default_value = "60")]
        fps: u32,

        /// Keyframe interval (seconds)
        #[arg(long, default_value = "2")]
        keyframe_interval: u32,

        /// Width
        #[arg(long, default_value = "1920")]
        width: u32,

        /// Height
        #[arg(long, default_value = "1080")]
        height: u32,
    },
}

#[derive(Clone, ValueEnum, Debug)]
enum CliCodec {
    H264,
    Hevc,
}

impl From<CliCodec> for Codec {
    fn from(c: CliCodec) -> Self {
        match c {
            CliCodec::H264 => Codec::H264,
            CliCodec::Hevc => Codec::HEVC,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Record { output, codec, bitrate, fps, keyframe_interval, width, height } => {
            let config = EncoderConfig {
                codec: codec.into(),
                bitrate: bitrate * 1000,
                fps,
                keyframe_interval: keyframe_interval * fps,
                width,
                height,
                profile: None,
                preset: None,
            };
            run_cli_record(output.to_str().unwrap(), config).await?;
        }
    }

    Ok(())
}