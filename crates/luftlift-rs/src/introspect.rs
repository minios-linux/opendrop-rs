//! Loopback HTTP/JSON introspection API for the AirDrop receiver.
//!
//! Exposes live receiver state (`/status`), a runtime-toggleable structured
//! trace ring buffer (`GET/POST /trace`), bound to **127.0.0.1 only** so it
//! never reaches the AWDL link. This is SEPARATE from the AirDrop HTTPS
//! listener on 8771 — the two do not share a port or a server.
//!
//! See LUFTLIFT_HTTP_INTROSPECT.md for the spec.

use std::collections::VecDeque;
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
    /// Trace ring buffer; oldest events dropped past `TRACE_RING_CAP`.
    pub trace_ring: Mutex<VecDeque<TraceEvent>>,
    /// Whether the trace ring is accepting events (off by default).
    /// 0 = off, 1 = on.
    pub trace_enabled: AtomicU8,
    /// Current trace level setting (`"off"` / `"debug"` / `"trace"`), off by
    /// default. Stored for `/status` transparency and for re-applying on
    /// POST /trace. The atomic flag above is the hot-path gate.
    pub trace_level: Mutex<String>,
}

impl Default for IntrospectState {
    fn default() -> Self {
        Self {
            receiver_name: Mutex::new(String::new()),
            mdns_announced: AtomicBool::new(false),
            https_port: AtomicU16::new(0),
            last_announce_unix_ms: AtomicU64::new(0),
            mdns_query_rx: AtomicU64::new(0),
            discover_count: AtomicU64::new(0),
            ask_count: AtomicU64::new(0),
            upload_count: AtomicU64::new(0),
            last_peer: Mutex::new(String::new()),
            trace_ring: Mutex::new(VecDeque::new()),
            trace_enabled: AtomicU8::new(0),
            trace_level: Mutex::new("off".into()),
        }
    }
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

    /// Push a trace event into the ring. **No-op when `trace_enabled == 0`**
    /// (the default) — the hot path pays only a single atomic load. When on,
    /// appends to the ring and drops the oldest entry past `TRACE_RING_CAP`.
    pub fn push_trace(&self, event: TraceEvent) {
        if self.trace_enabled.load(Ordering::Relaxed) == 0 {
            return;
        }
        if let Ok(mut ring) = self.trace_ring.lock() {
            while ring.len() >= TRACE_RING_CAP {
                ring.pop_front();
            }
            ring.push_back(event);
        }
    }

    /// Return the last `n` trace events (newest last). `n == 0` returns all.
    pub fn trace_last_n(&self, n: usize) -> Vec<TraceEvent> {
        if let Ok(ring) = self.trace_ring.lock() {
            if n == 0 {
                ring.iter().cloned().collect()
            } else {
                let start = ring.len().saturating_sub(n);
                ring.iter().skip(start).cloned().collect()
            }
        } else {
            Vec::new()
        }
    }

    /// Set the trace level (`"off"` / `"debug"` / `"trace"`). Anything other
    /// than `"off"` enables the ring; `"off"` disables it but does NOT clear
    /// the buffer — re-enabling surfaces previously captured events.
    pub fn set_trace_level(&self, level: &str) {
        let normalized = normalize_trace_level(level);
        let enabled = if normalized == "off" { 0 } else { 1 };
        self.trace_enabled.store(enabled, Ordering::Release);
        if let Ok(mut cell) = self.trace_level.lock() {
            *cell = normalized;
        }
    }

    /// Current trace level (`"off"` / `"debug"` / `"trace"`).
    pub fn trace_level(&self) -> String {
        self.trace_level
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "off".into())
    }

    /// Whether the trace ring is currently accepting events.
    pub fn trace_enabled(&self) -> bool {
        self.trace_enabled.load(Ordering::Acquire) != 0
    }
}

/// Normalise a POSTed trace level to one of `off|debug|trace`. Unknown
/// values default to `"off"` (fail-safe: never silently enable tracing).
pub fn normalize_trace_level(level: &str) -> String {
    let l = level.trim().to_ascii_lowercase();
    match l.as_str() {
        "off" | "debug" | "trace" => l,
        _ => "off".into(),
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

    // ---- trace ring + off-by-default gate ----

    fn ev(msg: &str) -> TraceEvent {
        TraceEvent {
            ts: 0,
            level: "INFO".into(),
            target: "test".into(),
            msg: msg.into(),
            fields: serde_json::Value::Null,
        }
    }

    #[test]
    fn trace_off_by_default_push_is_noop() {
        let st = IntrospectState::new();
        assert!(!st.trace_enabled(), "trace must be off by default");
        assert_eq!(st.trace_level(), "off");
        st.push_trace(ev("a"));
        assert!(st.trace_last_n(0).is_empty(), "push must be a no-op when off");
    }

    #[test]
    fn trace_enabled_push_appends_and_returns_in_order() {
        let st = IntrospectState::new();
        st.set_trace_level("debug");
        assert!(st.trace_enabled());
        assert_eq!(st.trace_level(), "debug");
        st.push_trace(ev("a"));
        st.push_trace(ev("b"));
        st.push_trace(ev("c"));
        let events = st.trace_last_n(0);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].msg, "a");
        assert_eq!(events[1].msg, "b");
        assert_eq!(events[2].msg, "c");
    }

    #[test]
    fn trace_last_n_returns_tail_respecting_n() {
        let st = IntrospectState::new();
        st.set_trace_level("trace");
        for i in 0..5 {
            st.push_trace(ev(&format!("e{i}")));
        }
        assert_eq!(st.trace_last_n(0).len(), 5, "n == 0 returns all");
        let last3 = st.trace_last_n(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0].msg, "e2");
        assert_eq!(last3[2].msg, "e4");
        // n larger than the ring returns everything.
        assert_eq!(st.trace_last_n(100).len(), 5);
    }

    #[test]
    fn trace_ring_drops_oldest_past_cap() {
        // Sanity: cap is the documented ~2000.
        assert_eq!(TRACE_RING_CAP, 2000);
        let st = IntrospectState::new();
        st.set_trace_level("debug");
        for i in 0..(TRACE_RING_CAP + 50) {
            st.push_trace(ev(&format!("e{i}")));
        }
        let all = st.trace_last_n(0);
        assert_eq!(all.len(), TRACE_RING_CAP, "ring must be capped");
        // Oldest entries (e0..e49) were dropped; newest first kept is e50.
        assert_eq!(all.first().unwrap().msg, "e50", "oldest in ring is e50");
        assert_eq!(all.last().unwrap().msg, format!("e{}", TRACE_RING_CAP + 49));
    }

    #[test]
    fn trace_set_level_off_disables_but_does_not_clear_buffer() {
        let st = IntrospectState::new();
        st.set_trace_level("debug");
        st.push_trace(ev("captured"));
        assert_eq!(st.trace_last_n(0).len(), 1);
        st.set_trace_level("off");
        assert!(!st.trace_enabled());
        // Push is a no-op again.
        st.push_trace(ev("ignored"));
        // But previously captured events are still retrievable.
        assert_eq!(st.trace_last_n(0).len(), 1);
        assert_eq!(st.trace_last_n(0)[0].msg, "captured");
    }

    #[test]
    fn trace_normalize_level_accepts_off_debug_trace_rejects_unknown() {
        assert_eq!(normalize_trace_level("off"), "off");
        assert_eq!(normalize_trace_level("DEBUG"), "debug");
        assert_eq!(normalize_trace_level("trace"), "trace");
        assert_eq!(normalize_trace_level("  debug  "), "debug");
        // Unknown values fail-safe to "off" — never silently enable tracing.
        assert_eq!(normalize_trace_level("verbose"), "off");
        assert_eq!(normalize_trace_level(""), "off");
    }

    #[test]
    fn trace_set_level_unknown_fails_safe_to_off() {
        let st = IntrospectState::new();
        st.set_trace_level("debug");
        st.set_trace_level("nonsense");
        assert!(!st.trace_enabled(), "unknown level must disable tracing");
        st.push_trace(ev("x"));
        assert!(st.trace_last_n(0).is_empty());
    }

    #[test]
    fn trace_event_serialises_to_json_with_expected_shape() {
        let e = TraceEvent {
            ts: 1_700_000_000_000,
            level: "INFO".into(),
            target: "luftlift_rs::server".into(),
            msg: "Discover request".into(),
            fields: serde_json::json!({"peer": "fe80::1"}),
        };
        let v = serde_json::to_value(&e).unwrap();
        let obj = v.as_object().unwrap();
        for key in ["ts", "level", "target", "msg", "fields"] {
            assert!(obj.contains_key(key), "TraceEvent JSON must include {key}");
        }
        assert_eq!(obj["fields"]["peer"], serde_json::json!("fe80::1"));
    }
}
