//!
//! S7 PLC Simulator - Main Entry Point
//! 
//! Starts both S7 protocol server and Web Admin API

use clap::Parser;
use tracing_subscriber::prelude::*;
use s7_plc_simulator::{create_shared_memory, create_shared_memory_from_config, create_connection_list, PlcSimulator, api};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "s7-plc-simulator")]
#[command(about = "S7 PLC Simulator with Web Admin", long_about = None)]
struct Cli {
    /// S7 port (default: 102)
    #[arg(short, long, default_value_t = 102)]
    s7_port: u16,
    
    /// Web API port (default: 8080)
    #[arg(short, long, default_value_t = 8080)]
    web_port: u16,
    
    /// PLC type
    #[arg(short, long, default_value = "S7-300")]
    plc_type: String,
    
    /// Rack number
    #[arg(short = 'R', long, default_value_t = 0)]
    rack: u8,
    
    /// Slot number
    #[arg(short = 'S', long, default_value_t = 2)]
    slot: u8,
    
    /// Verbose logging
    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    verbose: bool,
    
    /// Configuration file path (JSON)
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    
    // Initialize logging
    let filter = if cli.verbose {
        tracing_subscriber::EnvFilter::new("debug")
    } else {
        tracing_subscriber::EnvFilter::new("info")
    };
    
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();
    
    // Print banner
    println!();
    println!("╔══════════════════════════════════════════════════╗");
    println!("║           S7 PLC Simulator v0.2                 ║");
    println!("╠══════════════════════════════════════════════════╣");
    println!("║  ⚙️  S7 Protocol Port:  {}                      ║", format!("{:>5}", cli.s7_port));
    println!("║  🌐 Web Admin Port:     {}                      ║", format!("{:>5}", cli.web_port));
    println!("║  🖥️  PLC Type:          {:<24}║", cli.plc_type);
    println!("║  📍 Rack/Slot:          {}/{}                        ║", cli.rack, cli.slot);
    println!("╚══════════════════════════════════════════════════╝");
    println!();
    
    // Create shared memory from config or default
    let memory = if let Some(config_path) = &cli.config {
        match create_shared_memory_from_config(config_path) {
            Ok(m) => {
                println!("📄 Loaded configuration from: {}", config_path.display());
                m
            }
            Err(e) => {
                eprintln!("❌ Failed to load config: {}", e);
                eprintln!("   Using default memory configuration...");
                create_shared_memory()
            }
        }
    } else {
        create_shared_memory()
    };
    
    // Create shared connection list
    let connections = create_connection_list();
    
    // Print memory info
    {
        let mem = memory.read().unwrap();
        let dbs = mem.list_dbs();
        println!("📦 Pre-loaded Data Blocks:");
        for db in &dbs {
            println!("   DB{} - {} bytes", db.number, db.size);
        }
        println!();
    }
    
    println!("🌐 Web Admin: http://localhost:{}/", cli.web_port);
    println!("⚙️  S7 Server:  0.0.0.0:{}", cli.s7_port);
    println!();
    println!("Press Ctrl+C to stop");
    println!();
    
    // Spawn S7 server
    let memory_clone = memory.clone();
    let connections_clone = connections.clone();
    let plc_type = cli.plc_type.clone();
    let s7_port = cli.s7_port;
    let rack = cli.rack;
    let slot = cli.slot;
    
    let s7_handle = tokio::spawn(async move {
        PlcSimulator::start_s7_server(s7_port, memory_clone, &plc_type, rack, slot, connections_clone).await
    });
    
    // Spawn Web API server
    let web_handle = tokio::spawn(async move {
        api::start_server(cli.web_port, memory, cli.s7_port, connections).await
    });
    
    // Wait for Ctrl+C, then shutdown gracefully
    tokio::signal::ctrl_c().await?;
    eprintln!("\nShutting down...");
    
    // Abort both server tasks (they're infinite loops, must force-kill)
    s7_handle.abort();
    web_handle.abort();
    
    // Give them a moment to clean up
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        let _ = s7_handle.await;
        let _ = web_handle.await;
    }).await;
    
    eprintln!("S7 PLC Simulator stopped.");
    Ok(())
}
