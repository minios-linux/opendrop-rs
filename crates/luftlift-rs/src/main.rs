//! luftlift — Rust port of opendrop (the AirDrop application layer).
//!
//! Runs over the `awdl0` interface that `filin-rs` brings up. See PORT.md for
//! the full spec and the reverse-engineered iOS-18 protocol details (dvzip,
//! chunked Ask/Discover, etc.).
//!
//! Reference: ../../opendrop-patch/ (patched Python opendrop + dvzip.py decoder).

use anyhow::Result;
use clap::{Parser, Subcommand};
use luftlift_rs::{
    mdns, netutil, server, tls,
    plist_impl::ReceiverConfig,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "luftlift", about = "AirDrop receiver (port of opendrop)")]
struct Cli {
    /// Network interface to bind (e.g. awdl0).
    #[arg(short, long, default_value = "awdl0")]
    interface: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Receive files via AirDrop (announce + HTTPS server).
    Receive {
        /// Directory to save received files (default: current dir).
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },
    /// Find AirDrop senders on the local network.
    Find {
        /// Browse duration in seconds.
        #[arg(short, long, default_value_t = 5)]
        duration: u64,
    },
    /// Send a file to a specific AirDrop receiver.
    Send {
        /// Recipient identifier (mDNS instance name).
        recipient: String,
        /// File to send.
        file: PathBuf,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "luftlift_rs=debug,info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Receive { output } => receive(&cli.interface, output),
        Command::Find { duration } => find(duration),
        Command::Send { recipient, file } => {
            tracing::error!("send not yet implemented (recipient={recipient}, file={file:?})");
            anyhow::bail!("send not yet implemented")
        }
    }
}

fn receive(interface: &str, output: PathBuf) -> Result<()> {
    let addr = netutil::get_ipv6_for_interface(interface)
        .ok_or_else(|| anyhow::anyhow!("interface {interface} has no IPv6 address — is filin-rs/awdl0 running?"))?;
    tracing::info!(interface, %addr, "starting AirDrop receiver");

    let cert = tls::generate_self_signed_cert()?;
    let tls_config = tls::build_server_config(&cert)?;
    tracing::info!("TLS cert generated");

    let cfg = ReceiverConfig {
        computer_name: hostname(),
        computer_model: "MacBookPro".into(),
        record_data: None,
    };
    let mdns_cfg = mdns::MdnsConfig {
        computer_name: cfg.computer_name.clone(),
        port: 8771,
        flags: 1,
    };
    let (_mdns_daemon, _info) = mdns::announce(&mdns_cfg, addr.into())?;
    tracing::info!("mDNS announced — receiver visible in AirDrop picker");

    let listener = server::bind_listener(8771)?;
    tracing::info!(output = %output.display(), "saving received files to output dir");

    server::serve_forever(listener, Arc::new(tls_config), cfg, Some(output));
    Ok(())
}

fn find(duration: u64) -> Result<()> {
    tracing::info!(duration, "browsing for _airdrop._tcp");
    let peers = mdns::browse(Duration::from_secs(duration))?;
    if peers.is_empty() {
        println!("No AirDrop senders found.");
        println!("Tip: Apple devices announce only while their sharing pane is open.");
    } else {
        println!("AirDrop senders found:");
        for peer in &peers {
            let addrs = peer
                .addresses
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!("  {} ({}:{})", peer.name, addrs, peer.port);
        }
    }
    Ok(())
}

/// Best-effort hostname for the receiver identity.
fn hostname() -> String {
    std::env::var("LUFTLIFT_NAME")
        .unwrap_or_else(|_| "luftlift".to_string())
}
