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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Default AirDrop `flags` TXT value: matches opendrop's verified default
/// (`--flags 0x06`), proven to make the receiver appear in iOS 18's AirDrop
/// picker. `0x06 = 0b110` sets the receiver-capability + discoverability bits.
pub const DEFAULT_FLAGS: u32 = 0x06;

/// Interval between proactive mDNS re-announcements while `receive` runs.
///
/// mdns-sd 0.11's record TTLs are: A/SRV = **120s**, PTR/TXT = 4500s (the
/// setters are `pub(crate)`, so we cannot raise them). mdns-sd answers
/// incoming queries, but over a quiet/flaky AWDL link the Apple device's
/// refresh queries (or our responses) don't always get through; once the
/// 120s A/SRV TTL lapses with no refresh the device can no longer resolve
/// the receiver and it drops from the AirDrop picker. Re-announcing every
/// 60s (half the A/SRV TTL) keeps the records fresh on peers independent of
/// query traffic, matching the symptom observed in live testing (receiver
/// disappeared from the Mac's picker ~2 min after starting `receive`).
pub const REANNOUNCE_SECS: u64 = 60;

/// Tick rate of the re-announce loop. Short enough that shutdown (via the
/// stop flag) is responsive, long enough that the loop is effectively idle
/// between announcements.
const REANNOUNCE_TICK: Duration = Duration::from_secs(1);

/// Configuration for the mDNS announcement.
pub struct MdnsConfig {
    pub computer_name: String,
    pub port: u16,
    /// AirDrop `flags` TXT record value (decimal integer advertised as a
    /// string). This advertises the receiver's discoverability/capability
    /// bits to the sender; if the wrong bits are set the sender never
    /// surfaces us in its AirDrop picker.
    ///
    /// The opendrop default (verified against iOS 18.6.2) is **`0x06`** =
    /// `0b110` — sets the "supports receiving" and discoverability bits
    /// Apple's sender checks. `0x01` (a single bit) is NOT enough for
    /// current iOS to surface us. Override via `--flags` for experimentation.
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

/// Re-announce an already-registered service: calling `register` again on the
/// same `ServiceDaemon` pushes out fresh A/SRV/PTR/TXT records immediately and
/// schedules mdns-sd's RFC 6762 §8.3 1s retransmit, without a TTL=0 goodbye
/// (no picker flicker). This is the canonical mdns-sd 0.11 refresh mechanism
/// — `register_service` always emits an unsolicited response, then replaces
/// the existing entry in `my_services`. Used to keep the receiver visible in
/// the AirDrop picker past the 120s A/SRV TTL.
pub fn reannounce(daemon: &ServiceDaemon, info: &ServiceInfo) -> Result<()> {
    daemon.register(info.clone())?;
    Ok(())
}

/// Pure decision helper: has `REANNOUNCE_SECS` elapsed since `last`?
/// Split out so the interval logic is unit-testable without sleeping for a
/// full minute. Returns `true` when `now - last >= REANNOUNCE_SECS`.
pub fn should_reannounce(last: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last) >= Duration::from_secs(REANNOUNCE_SECS)
}

/// Spawn a background thread that proactively re-announces the service every
/// `REANNOUNCE_SECS` until the returned stop flag is set. Keeps the A/SRV
/// records (120s TTL) fresh on peers so the receiver stays in the Apple
/// AirDrop picker indefinitely. The `ServiceDaemon` is shared (`Arc`) with
/// the main thread's goodbye handler; `ServiceInfo` is `Clone`.
///
/// Returns the stop flag (set `true` to ask the thread to exit on its next
/// tick, ≤ `REANNOUNCE_TICK` later) plus the thread handle. The thread is
/// also killed cleanly by `std::process::exit` in the SIGINT handler, so
/// setting the flag is best-effort hygiene rather than a hard requirement.
pub fn spawn_reannouncer(
    daemon: Arc<ServiceDaemon>,
    info: ServiceInfo,
) -> (Arc<AtomicBool>, JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let handle = thread::spawn(move || {
        let mut last = Instant::now();
        debug!(interval_secs = REANNOUNCE_SECS, "mDNS re-announcer started");
        while !stop.load(Ordering::Acquire) {
            thread::sleep(REANNOUNCE_TICK);
            if stop.load(Ordering::Acquire) {
                break;
            }
            let now = Instant::now();
            if should_reannounce(last, now) {
                let fullname = info.get_fullname().to_string();
                match reannounce(&daemon, &info) {
                    Ok(_) => {
                        debug!(service = %fullname, "mDNS re-announced (refresh A/SRV/PTR/TXT)");
                        last = now;
                    }
                    Err(e) => warn!(service = %fullname, error = %e, "mDNS re-announce failed"),
                }
            }
        }
        debug!("mDNS re-announcer stopped");
    });
    (stop_clone, handle)
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
        MdnsConfig { computer_name: "luftlift-pc".into(), port: 8771, flags: DEFAULT_FLAGS }
    }

    #[test]
    fn service_info_has_airdrop_type_and_flags() {
        let info =
            build_airdrop_service_info(&cfg(), "127.0.0.1".parse().unwrap()).expect("build info");
        assert_eq!(info.get_port(), 8771);
        assert!(info.get_fullname().contains("_airdrop._tcp"));
        // flags TXT property must be present and carry the opendrop default 0x06.
        let props = info.get_properties();
        assert_eq!(props.get("flags").map(|v| v.val_str()), Some("6"));
    }

    #[test]
    fn default_flags_is_opendrop_value_0x06() {
        // Guards against regressing back to 1 (which didn't surface us on iOS 18).
        assert_eq!(DEFAULT_FLAGS, 0x06);
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

    // ---- re-announce (the 120s TTL refresh fix) ----

    #[test]
    #[allow(clippy::assertions_on_constants)] // intentional invariant guard on a const
    fn reannounce_interval_is_under_asrv_ttl() {
        // The A/SRV record TTL is 120s. REANNOUNCE_SECS must be comfortably
        // under it so a refresh goes out before the records expire on a peer
        // (a ~2x safety margin, matching common mDNS practice).
        assert!(REANNOUNCE_SECS < 120, "re-announce interval must be < 120s A/SRV TTL");
        assert_eq!(REANNOUNCE_SECS, 60, "default is half the TTL (RFC-flavoured)");
    }

    #[test]
    fn should_reannounce_returns_false_before_interval_and_true_at_or_after() {
        // Pure decision-logic test (no real sleeping). Guards the interval
        // boundary the live receiver depends on.
        let last = Instant::now();
        // 1s before the threshold: not yet.
        let before =
            last + Duration::from_secs(REANNOUNCE_SECS) - Duration::from_secs(1);
        assert!(!should_reannounce(last, before), "1s before interval -> no re-announce");
        // Exactly at the threshold: yes.
        let at = last + Duration::from_secs(REANNOUNCE_SECS);
        assert!(should_reannounce(last, at), "at interval -> re-announce");
        // 30s past the threshold: still yes.
        let past = last + Duration::from_secs(REANNOUNCE_SECS + 30);
        assert!(should_reannounce(last, past), "past interval -> re-announce");
    }

    #[test]
    fn reannounce_calls_register_again_on_same_daemon_without_error() {
        // Re-registering an already-announced service on the same daemon is
        // the canonical mdns-sd refresh (emits fresh A/SRV/PTR/TXT + the
        // RFC 6762 §8.3 1s retransmit, no TTL=0 goodbye). Verify it accepts
        // the second register without error.
        let (daemon, info) = announce(&cfg(), "127.0.0.1".parse().unwrap()).expect("announce");
        reannounce(&daemon, &info).expect("re-announce must succeed on same daemon");
        // A second re-announce should also be fine (idempotent refresh).
        reannounce(&daemon, &info).expect("second re-announce must succeed");
        goodbye(&daemon, info.get_fullname()).expect("goodbye");
    }

    #[test]
    fn spawn_reannouncer_stops_promptly_when_flag_is_set() {
        // The re-announcer thread must observe the stop flag within ~1 tick
        // (REANNOUNCE_TICK = 1s), so the SIGINT handler can wind it down
        // cleanly. Verifies the join handle resolves promptly after stop.
        let (daemon, info) = announce(&cfg(), "127.0.0.1".parse().unwrap()).expect("announce");
        let fullname = info.get_fullname().to_string();
        let daemon = Arc::new(daemon);
        let (stop, handle) = spawn_reannouncer(daemon.clone(), info);
        // Let it tick once, then stop it.
        std::thread::sleep(Duration::from_millis(1100));
        stop.store(true, std::sync::atomic::Ordering::Release);
        handle.join().expect("re-announcer thread must exit cleanly after stop");
        goodbye(&daemon, &fullname).ok();
    }
}
