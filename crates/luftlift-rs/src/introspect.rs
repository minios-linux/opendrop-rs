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

/// Live, lock-free-ish receiver state for `/status`. Counters are atomic so
/// the AirDrop HTTPS handler thread can bump them without contending with the
/// introspection HTTP thread's snapshot read.
#[derive(Default)]
pub struct IntrospectState {
    pub receiver_name: Mutex<String>,
    pub mdns_announced: AtomicBool,
    pub https_port: AtomicU16,
    pub last_announce_unix_ms: AtomicU64,
    pub mdns_query_rx: AtomicU64,
    pub discover_count: AtomicU64,
    pub ask_count: AtomicU64,
    pub upload_count: AtomicU64,
    pub last_peer: Mutex<String>,
    pub trace_enabled: AtomicU8,
}

impl IntrospectState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_announce(&self, _receiver_name: String, _https_port: u16) {
        todo!()
    }

    pub fn bump_mdns_query_rx(&self) {
        todo!()
    }

    pub fn bump_discover(&self) {
        todo!()
    }

    pub fn bump_ask(&self, _sender: Option<&str>) {
        todo!()
    }

    pub fn bump_upload(&self) {
        todo!()
    }

    pub fn snapshot(&self) -> StatusSnapshot {
        todo!()
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
        let json = serde_json::to_value(&st.snapshot()).unwrap();
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
