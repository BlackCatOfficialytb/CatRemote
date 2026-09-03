use catremote_env::{detect_capabilities, Capabilities};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::process;

#[derive(Parser)]
#[command(name = "catremote-env", version, about = "Environment validation and capability detection")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check and print capabilities as JSON
    Check {
        /// Output format (json, pretty)
        #[arg(short, long, default_value = "pretty")]
        format: OutputFormat,
        /// Only output specific capability section
        #[arg(short, long)]
        section: Option<String>,
    },
    /// Validate required capabilities for capture
    Validate {
        /// Exit with non-zero code if validation fails
        #[arg(short, long)]
        strict: bool,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum OutputFormat {
    Json,
    Pretty,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check { format, section } => {
            match run_check(format, section).await {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            }
        }
        Commands::Validate { strict } => {
            match run_validate(strict).await {
                Ok(valid) => {
                    if !valid && strict {
                        process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            }
        }
    }
}

async fn run_check(format: OutputFormat, section: Option<String>) -> anyhow::Result<()> {
    let caps = detect_capabilities().await?;

    let output = if let Some(section) = section {
        match section.as_str() {
            "compositor" => json!(caps.compositor),
            "portal" => json!(caps.portal),
            "gpu" | "gpu-encoders" => json!(caps.gpu_encoders),
            "pipewire" => json!(caps.pipewire),
            "kernel" => json!(caps.kernel),
            _ => {
                anyhow::bail!("Unknown section: {}", section);
            }
        }
    } else {
        json!(caps)
    };

    let output_str = match format {
        OutputFormat::Json => serde_json::to_string(&output)?,
        OutputFormat::Pretty => serde_json::to_string_pretty(&output)?,
    };

    println!("{}", output_str);
    Ok(())
}

async fn run_validate(strict: bool) -> anyhow::Result<bool> {
    let caps = detect_capabilities().await?;

    let mut issues = Vec::<String>::new();

    if !caps.compositor.wlr_screencopy_v1 && !caps.compositor.ext_image_capture_source_v1 {
        issues.push("No supported screen capture protocol (wlr-screencopy-v1 or ext-image-capture-source-v1)".to_string());
    }

    if caps.portal.screencast_permission != catremote_env::PortalPermissionState::Granted {
        issues.push("ScreenCast portal permission not granted".to_string());
    }

    if !caps.gpu_encoders.vaapi.available && !caps.gpu_encoders.nvenc.available {
        issues.push("No GPU encoder available (VA-API or NVENC)".to_string());
    }

    if !caps.pipewire.available {
        issues.push("PipeWire not available".to_string());
    } else if let Some(version) = &caps.pipewire.version {
        if let Some(ver) = parse_version(version) {
            if ver < (0, 3, 70) {
                issues.push("PipeWire version < 0.3.70".to_string());
            }
        }
    }

    if !caps.pipewire.missing_modules.is_empty() {
        issues.push(format!("Missing PipeWire modules: {}", caps.pipewire.missing_modules.join(", ")));
    }

    if !caps.kernel.dma_buf.available {
        issues.push("DMA-BUF heaps not available".to_string());
    }

    if !caps.kernel.libei.available {
        issues.push("libei not available".to_string());
    }

    if !caps.kernel.evdev.available {
        issues.push("evdev not available".to_string());
    }

    if issues.is_empty() {
        println!("✓ All required capabilities available");
        Ok(true)
    } else {
        println!("✗ Validation failed:");
        for issue in &issues {
            println!("  - {}", issue);
        }
        Ok(false)
    }
}

fn parse_version(version_str: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = version_str.split('.').collect();
    if parts.len() >= 3 {
        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts[2].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
        Some((major, minor, patch))
    } else {
        None
    }
}