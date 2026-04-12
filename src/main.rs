//!
//! S7 PLC Simulator CLI
//! 
//! A simulated S7 PLC for testing S7 Connector

use clap::Parser;
use tracing_subscriber::prelude::*;
use s7_plc_simulator::PlcSimulator;

#[derive(Parser)]
#[command(name = "s7-plc-simulator")]
#[command(about = "S7 PLC Simulator for testing", long_about = None)]
struct Cli {
    /// Port to listen on
    #[arg(short, long, default_value_t = 102)]
    port: u16,
    
    /// PLC type (S7-300, S7-1500, etc.)
    #[arg(short, long, default_value = "S7-300")]
    plc_type: String,
    
    /// Rack number
    #[arg(short, long, default_value_t = 0)]
    rack: u8,
    
    /// Slot number
    #[arg(short, long, default_value_t = 2)]
    slot: u8,
    
    /// Verbose output
    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    
    // Initialize logging
    let filter = if cli.verbose {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();
    
    println!("╔════════════════════════════════════════════╗");
    println!("║        S7 PLC Simulator                    ║");
    println!("╠════════════════════════════════════════════╣");
    println!("║  Type:  {}                                ║", cli.plc_type);
    println!("║  Rack:  {}                                ║", cli.rack);
    println!("║  Slot:  {}                                ║", cli.slot);
    println!("║  Port:  {}                                ║", cli.port);
    println!("╚════════════════════════════════════════════╝");
    println!();
    
    // Print pre-loaded data blocks
    println!("Pre-loaded Data Blocks:");
    println!("  DB1  - General data (256 bytes)");
    println!("  DB2  - Counter values (128 bytes)");
    println!("  DB3  - Timer values (128 bytes)");
    println!("  DB10 - Real values (64 bytes, test floats)");
    println!("  DB11 - Integer values (32 bytes, test ints)");
    println!("  DB20 - String test (128 bytes, 'Hello World!')");
    println!();
    
    println!("Waiting for S7 connections on 0.0.0.0:{}...", cli.port);
    println!("Press Ctrl+C to stop");
    println!();
    
    // Start server
    PlcSimulator::start_server(cli.port, &cli.plc_type, cli.rack, cli.slot).await
}
