#[cfg(target_os = "linux")]
mod cli {
    use catremote_transport::{run_transport_server, run_transport_client, TransportConfig, KemAlgorithm};
    use clap::{Parser, Subcommand, ValueEnum};
    use std::net::SocketAddr;
    use std::str::FromStr;

    #[derive(Parser)]
    #[command(name = "catremote-transport", version, about = "QUIC transport engine")]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[derive(Subcommand)]
    enum Commands {
        /// Run as server
        Server {
            /// Bind address
            #[arg(short, long, default_value = "0.0.0.0:8080")]
            bind: String,
            
            /// Server name for TLS
            #[arg(long, default_value = "catremote")]
            server_name: String,
            
            /// KEM algorithm
            #[arg(long, value_enum, default_value = "hybrid")]
            kem: CliKemAlgorithm,
        },
        
        /// Run as client
        Client {
            /// Server address
            #[arg(short, long)]
            server: String,
            
            /// Server name for TLS
            #[arg(long, default_value = "catremote")]
            server_name: String,
            
            /// KEM algorithm
            #[arg(long, value_enum, default_value = "hybrid")]
            kem: CliKemAlgorithm,
        },
    }

    #[derive(Clone, ValueEnum, Debug)]
    enum CliKemAlgorithm {
        MlKem768,
        Hybrid,
    }

    impl From<CliKemAlgorithm> for KemAlgorithm {
        fn from(k: CliKemAlgorithm) -> Self {
            match k {
                CliKemAlgorithm::MlKem768 => KemAlgorithm::MlKem768,
                CliKemAlgorithm::Hybrid => KemAlgorithm::Hybrid,
            }
        }
    }

    #[tokio::main]
    async fn main() -> anyhow::Result<()> {
        tracing_subscriber::fmt::init();
        
        let cli = Cli::parse();
        
        match cli.command {
            Commands::Server { bind, server_name, kem } => {
                let config = TransportConfig {
                    bind_addr: SocketAddr::from_str(&bind)?,
                    server_name,
                    kem_algorithm: kem.into(),
                    ..Default::default()
                };
                run_transport_server(config).await?;
            }
            Commands::Client { server, server_name, kem } => {
                let config = TransportConfig {
                    server_name,
                    kem_algorithm: kem.into(),
                    ..Default::default()
                };
                let addr = SocketAddr::from_str(&server)?;
                run_transport_client(config, addr).await?;
            }
        }
        
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("catremote-transport CLI is only available on Linux");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    cli::main()
}