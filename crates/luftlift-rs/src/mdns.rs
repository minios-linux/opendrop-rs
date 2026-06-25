//! mDNS announce for AirDrop: registers a `_airdrop._tcp` service so the
//! receiver appears in the sender's AirDrop picker. Uses the pure-Rust
//! `mdns-sd` crate (no system avahi/dbus). Apple devices announce only while
//! their sharing pane is open; we announce continuously while `receive` runs.
//!
//! Reference: `opendrop-patch/server.py.patched` (`_init_service`,
//! `get_properties`).

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::IpAddr;

/// Configuration for the mDNS announcement.
pub struct MdnsConfig {
    pub computer_name: String,
    pub port: u16,
    /// AirDrop flags TXT value (1 = "Everyone" discoverability).
    pub flags: u32,
}

/// Build the `_airdrop._tcp` ServiceInfo for announcement.
pub fn build_airdrop_service_info(_cfg: &MdnsConfig, _addr: IpAddr) -> Result<ServiceInfo> {
    anyhow::bail!("unimplemented")
}

/// Register the AirDrop service on the local network. Returns the daemon
/// (keep alive to maintain the announcement) and the registered info.
pub fn announce(_cfg: &MdnsConfig, _addr: IpAddr) -> Result<(ServiceDaemon, ServiceInfo)> {
    anyhow::bail!("unimplemented")
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
