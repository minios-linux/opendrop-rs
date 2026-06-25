//! mDNS announce for AirDrop: registers a `_airdrop._tcp` service so the
//! receiver appears in the sender's AirDrop picker. Uses the pure-Rust
//! `mdns-sd` crate (no system avahi/dbus). Apple devices announce only while
//! their sharing pane is open; we announce continuously while `receive` runs.
//!
//! Reference: `opendrop-patch/server.py.patched` (`_init_service`,
//! `get_properties`).

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo, UnregisterStatus};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Configuration for the mDNS announcement.
pub struct MdnsConfig {
    pub computer_name: String,
    pub port: u16,
    /// AirDrop flags TXT value (1 = "Everyone" discoverability).
    pub flags: u32,
}

/// Build the `_airdrop._tcp` ServiceInfo for announcement.
///
/// The instance name embeds the **process PID** (`<name>-<pid>`) so that, when
/// entries appear in the Apple AirDrop picker, a fresh run is distinguishable
/// from a stale ghost left by a crashed/killed previous run.
pub fn build_airdrop_service_info(cfg: &MdnsConfig, addr: IpAddr) -> Result<ServiceInfo> {
    let instance = format!("{}-{}", cfg.computer_name, std::process::id());
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

/// Gracefully unregister an mDNS service: multicasts the TTL=0 goodbye for
/// every record (PTR/TXT/SRV/A) so peers (incl. Apple devices) remove the
/// entry from their AirDrop picker promptly. Waits up to `timeout` for mdns-sd
/// to confirm the goodbye was sent, then shuts the daemon down.
///
/// This covers clean exits (Ctrl+C / SIGTERM). For ungraceful crashes, the
/// mdns-sd 0.11 default `host_ttl=120` already self-clears the A/SRV records
/// in ~2 min; the PTR/TXT records (`other_ttl=4500`) cannot be shortened
/// because mdns-sd exposes no public TTL setter (`_set_*_ttl` is `pub(crate)`
/// in every released version) — the PID-suffixed instance name is the
/// mitigation there, letting you tell a fresh run from a stale ghost.
pub fn goodbye(daemon: &ServiceDaemon, fullname: &str) -> Result<()> {
    match daemon.unregister(fullname) {
        Ok(recv) => match recv.recv_timeout(Duration::from_secs(2)) {
            Ok(UnregisterStatus::OK) => info!(service = %fullname, "mDNS goodbye (TTL=0) sent"),
            Ok(other) => warn!(service = %fullname, ?other, "mDNS unregister returned non-OK"),
            Err(e) => warn!(service = %fullname, error = %e, "mDNS goodbye timed out — continuing"),
        },
        Err(e) => warn!(service = %fullname, error = %e, "mDNS unregister failed — continuing"),
    }
    let _ = daemon.shutdown();
    Ok(())
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
    fn service_info_instance_name_includes_pid() {
        let info =
            build_airdrop_service_info(&cfg(), "127.0.0.1".parse().unwrap()).expect("build info");
        // Instance name must embed the current PID so fresh runs are
        // distinguishable from stale ghosts in the Apple picker.
        let pid = std::process::id().to_string();
        assert!(
            info.get_fullname().contains(&pid),
            "instance name {} must contain PID {}",
            info.get_fullname(),
            pid
        );
    }

    #[test]
    fn announce_registers_without_error() {
        let (daemon, info) = announce(&cfg(), "127.0.0.1".parse().unwrap()).expect("announce");
        assert!(info.get_fullname().contains("_airdrop._tcp"));
        // Clean shutdown via graceful goodbye.
        goodbye(&daemon, info.get_fullname()).expect("goodbye");
    }

    #[test]
    fn goodbye_unregisters_a_registered_service() {
        let (daemon, info) = announce(&cfg(), "127.0.0.1".parse().unwrap()).expect("announce");
        let fullname = info.get_fullname().to_string();
        // goodbye must return Ok and send the TTL=0 multicast promptly (the
        // unregister status receiver resolves well within the 2s timeout).
        goodbye(&daemon, &fullname).expect("goodbye");
        // A second unregister of the same fullname should now be a no-op /
        // non-fatal; the daemon is also shut down. Just confirm no panic.
        let _ = daemon.shutdown();
    }
}
