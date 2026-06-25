//! mDNS announce for AirDrop: registers a `_airdrop._tcp` service so the
//! receiver appears in the sender's AirDrop picker. Uses the pure-Rust
//! `mdns-sd` crate (no system avahi/dbus). Apple devices announce only while
//! their sharing pane is open; we announce continuously while `receive` runs.
//!
//! Reference: `opendrop-patch/server.py.patched` (`_init_service`,
//! `get_properties`).

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use tracing::info;

/// Configuration for the mDNS announcement.
pub struct MdnsConfig {
    pub computer_name: String,
    pub port: u16,
    /// AirDrop flags TXT value (1 = "Everyone" discoverability).
    pub flags: u32,
}

/// Build the `_airdrop._tcp` ServiceInfo for announcement.
pub fn build_airdrop_service_info(cfg: &MdnsConfig, addr: IpAddr) -> Result<ServiceInfo> {
    // The instance name doubles as the AirDrop receiver identity hash in
    // opendrop; we derive a simple deterministic prefix from the computer name.
    let instance = format!("{}-{}", cfg.computer_name, cfg.flags);
    let hostname = format!("{}.local.", cfg.computer_name);
    let mut props = HashMap::new();
    props.insert("flags".to_string(), cfg.flags.to_string());

    let info = ServiceInfo::new(
        "_airdrop._tcp.local.",
        &instance,
        &hostname,
        addr,
        cfg.port,
        Some(props),
    )?;
    Ok(info)
}

/// Register the AirDrop service on the local network. Returns the daemon
/// (keep alive to maintain the announcement) and the registered info.
pub fn announce(cfg: &MdnsConfig, addr: IpAddr) -> Result<(ServiceDaemon, ServiceInfo)> {
    let daemon = ServiceDaemon::new()?;
    let info = build_airdrop_service_info(cfg, addr)?;
    daemon.register(info.clone())?;
    info!(name = %cfg.computer_name, addr = %addr, port = cfg.port, "mDNS _airdrop._tcp announced");
    Ok((daemon, info))
}

/// A discovered AirDrop peer (from mDNS browse).
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub name: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
}

/// Browse for `_airdrop._tcp` services for `duration`. Returns discovered peers.
/// Apple devices appear only while their sharing pane is open.
pub fn browse(duration: Duration) -> Result<Vec<DiscoveredPeer>> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse("_airdrop._tcp.local.")?;
    let deadline = Instant::now() + duration;
    let mut peers = Vec::new();

    while let Ok(timeout) = deadline.checked_duration_since(Instant::now()).ok_or(()) {
        match receiver.recv_timeout(timeout) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                info!(name = %info.get_fullname(), addresses = ?info.get_addresses(), "discovered AirDrop peer");
                peers.push(DiscoveredPeer {
                    name: info.get_fullname().to_string(),
                    addresses: info.get_addresses().iter().copied().collect(),
                    port: info.get_port(),
                });
            }
            Ok(_) => {}
            Err(_) => break, // timeout
        }
    }
    let _ = daemon.shutdown();
    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MdnsConfig {
        MdnsConfig { computer_name: "luftlift-pc".into(), port: 8771, flags: 1 }
    }

    #[test]
    fn service_info_has_airdrop_type_and_flags() {
        let info =
            build_airdrop_service_info(&cfg(), "127.0.0.1".parse().unwrap()).expect("build info");
        assert_eq!(info.get_port(), 8771);
        assert!(info.get_fullname().contains("_airdrop._tcp"));
        // flags TXT property must be present.
        let props = info.get_properties();
        assert_eq!(props.get("flags").map(|v| v.val_str()), Some("1"));
    }

    #[test]
    fn announce_registers_without_error() {
        let (daemon, info) = announce(&cfg(), "127.0.0.1".parse().unwrap()).expect("announce");
        assert!(info.get_fullname().contains("_airdrop._tcp"));
        // Clean shutdown.
        let _ = daemon.shutdown();
    }
}
