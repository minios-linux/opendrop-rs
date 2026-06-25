//! Loopback HTTP/JSON introspection API for the AirDrop receiver.
//!
//! Exposes live receiver state (`/status`), a runtime-toggleable structured
//! trace ring buffer (`GET/POST /trace`), bound to **127.0.0.1 only** so it
//! never reaches the AWDL link. This is SEPARATE from the AirDrop HTTPS
//! listener on 8771 — the two do not share a port or a server.
//!
//! See LUFTLIFT_HTTP_INTROSPECT.md for the spec.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

/// Maximum number of trace events kept in the ring buffer. Older events are
/// dropped on push. The spec calls for ~2000.
pub const TRACE_RING_CAP: usize = 2000;

/// UNIX epoch milliseconds, used as a coarse timestamp origin for
/// `last_announce_ms_ago`. Stored once to avoid re-reading the clock on hot
/// paths.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Live, lock-free-ish receiver state for `/status`. Counters are atomic so
/// the AirDrop HTTPS handler thread can bump them without contending with the
/// introspection HTTP thread's snapshot read. The receiver name and last
/// peer are updated rarely, so a `Mutex<String>` is plenty.
#[derive(Default)]
pub struct IntrospectState {
    /// `ReceiverComputerName` shown in the Apple AirDrop picker.
    pub receiver_name: Mutex<String>,
    /// True once mDNS announce has succeeded.
    pub mdns_announced: AtomicBool,
    /// Port the AirDrop HTTPS listener is bound to (8771).
    pub https_port: AtomicU16,
    /// ms-since-epoch of the most recent successful mDNS (re-)announce.
    pub last_announce_unix_ms: AtomicU64,
    /// Inbound mDNS queries observed. Best-effort — mdns-sd 0.11 does not
    /// expose incoming queries on a registered service directly, so this
    /// is incremented opportunistically and may stay 0 in practice.
    pub mdns_query_rx: AtomicU64,
    /// Count of `/Discover` POSTs received from Apple senders.
    pub discover_count: AtomicU64,
    /// Count of `/Ask` POSTs received.
    pub ask_count: AtomicU64,
    /// Count of `/Upload` POSTs received.
    pub upload_count: AtomicU64,
    /// Most recent peer's sender name (from `/Ask`), for "who tried us last".
    pub last_peer: Mutex<String>,
    /// Whether the trace ring is accepting events (off by default).
    /// 0 = off, 1 = on.
    pub trace_enabled: AtomicU8,
}

impl IntrospectState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful mDNS announce: sets the flag and the timestamp.
    pub fn record_announce(&self, receiver_name: String, https_port: u16) {
        if let Ok(mut n) = self.receiver_name.lock() {
            *n = receiver_name;
        }
        self.https_port.store(https_port, Ordering::Release);
        self.last_announce_unix_ms.store(now_unix_ms(), Ordering::Release);
        self.mdns_announced.store(true, Ordering::Release);
    }

    /// Bump the inbound mDNS-query counter by 1.
    pub fn bump_mdns_query_rx(&self) {
        self.mdns_query_rx.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a `/Discover` POST: bump the counter.
    pub fn bump_discover(&self) {
        self.discover_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an `/Ask` POST: bump the counter and stash the sender name.
    pub fn bump_ask(&self, sender: Option<&str>) {
        self.ask_count.fetch_add(1, Ordering::Relaxed);
        if let Some(s) = sender {
            if let Ok(mut cell) = self.last_peer.lock() {
                *cell = s.to_string();
            }
        }
    }

    /// Record an `/Upload` POST: bump the counter.
    pub fn bump_upload(&self) {
        self.upload_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot the status for `/status`. The ms-ago field is computed at
    /// snapshot time so callers don't have to.
    pub fn snapshot(&self) -> StatusSnapshot {
        let last_announce_ms = self.last_announce_unix_ms.load(Ordering::Acquire);
        let now = now_unix_ms();
        let last_announce_ms_ago = now.saturating_sub(last_announce_ms);
        let receiver_name = self
            .receiver_name
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let last_peer = self.last_peer.lock().map(|s| s.clone()).unwrap_or_default();
        StatusSnapshot {
            receiver_name,
            mdns_announced: self.mdns_announced.load(Ordering::Acquire),
            https_port: self.https_port.load(Ordering::Acquire),
            last_announce_ms_ago,
            mdns_query_rx: self.mdns_query_rx.load(Ordering::Acquire),
            discover_count: self.discover_count.load(Ordering::Acquire),
            ask_count: self.ask_count.load(Ordering::Acquire),
            upload_count: self.upload_count.load(Ordering::Acquire),
            last_peer,
        }
    }
}

/// A captured structured trace event in the ring buffer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TraceEvent {
    pub ts: u64,
    pub level: String,
    pub target: String,
    pub msg: String,
    pub fields: serde_json::Value,
}

/// The JSON-serialisable `/status` body.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusSnapshot {
    pub receiver_name: String,
    pub mdns_announced: bool,
    pub https_port: u16,
    pub last_announce_ms_ago: u64,
    pub mdns_query_rx: u64,
    pub discover_count: u64,
    pub ask_count: u64,
    pub upload_count: u64,
    pub last_peer: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_defaults_are_zero_and_unannounced() {
        let st = IntrospectState::new();
        let s = st.snapshot();
        assert!(!s.mdns_announced, "default state is not announced");
        assert_eq!(s.https_port, 0);
        assert_eq!(s.mdns_query_rx, 0);
        assert_eq!(s.discover_count, 0);
        assert_eq!(s.ask_count, 0);
        assert_eq!(s.upload_count, 0);
        assert_eq!(s.receiver_name, "");
        assert_eq!(s.last_peer, "");
    }

    #[test]
    fn record_announce_sets_name_port_flag_and_recent_timestamp() {
        let st = IntrospectState::new();
        st.record_announce("luftlift-42".into(), 8771);
        let s = st.snapshot();
        assert!(s.mdns_announced);
        assert_eq!(s.receiver_name, "luftlift-42");
        assert_eq!(s.https_port, 8771);
        assert!(s.last_announce_ms_ago < 5_000, "ms_ago must be tiny after announce");
    }

    #[test]
    fn counters_bump_independently() {
        let st = IntrospectState::new();
        st.bump_discover();
        st.bump_discover();
        st.bump_discover();
        st.bump_ask(Some("iPhone"));
        st.bump_upload();
        st.bump_upload();
        st.bump_mdns_query_rx();
        st.bump_mdns_query_rx();
        let s = st.snapshot();
        assert_eq!(s.discover_count, 3);
        assert_eq!(s.ask_count, 1);
        assert_eq!(s.upload_count, 2);
        assert_eq!(s.mdns_query_rx, 2);
    }

    #[test]
    fn bump_ask_updates_last_peer() {
        let st = IntrospectState::new();
        st.bump_ask(Some("Andrew's iPhone"));
        assert_eq!(st.snapshot().last_peer, "Andrew's iPhone");
        st.bump_ask(None);
        assert_eq!(st.snapshot().last_peer, "Andrew's iPhone");
        st.bump_ask(Some("MacBook"));
        assert_eq!(st.snapshot().last_peer, "MacBook");
    }

    #[test]
    fn snapshot_is_a_point_in_time_copy() {
        let st = IntrospectState::new();
        st.bump_discover();
        let s = st.snapshot();
        st.bump_discover();
        st.bump_discover();
        assert_eq!(s.discover_count, 1, "snapshot is a copy");
        assert_eq!(st.snapshot().discover_count, 3);
    }

    #[test]
    fn status_snapshot_serialises_to_json_with_expected_fields() {
        let st = IntrospectState::new();
        st.record_announce("luftlift".into(), 8771);
        st.bump_discover();
        let json = serde_json::to_value(st.snapshot()).unwrap();
        let obj = json.as_object().expect("status is a JSON object");
        for key in [
            "receiver_name",
            "mdns_announced",
            "https_port",
            "last_announce_ms_ago",
            "mdns_query_rx",
            "discover_count",
            "ask_count",
            "upload_count",
            "last_peer",
        ] {
            assert!(obj.contains_key(key), "/status JSON must include field {key}");
        }
        assert_eq!(obj["discover_count"], serde_json::json!(1));
        assert_eq!(obj["mdns_announced"], serde_json::json!(true));
    }
}
