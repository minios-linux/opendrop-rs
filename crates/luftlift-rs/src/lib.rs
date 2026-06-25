//! luftlift-rs — Rust port of opendrop (the AirDrop application layer).
//!
//! Runs over the `awdl0` interface that `filin-rs` brings up. See PORT.md for the
//! full spec and the reverse-engineered iOS-18 protocol details (dvzip, chunked
//! Ask/Discover, etc.).
//!
//! Reference: ../../opendrop-patch/ (patched Python opendrop + dvzip.py decoder).

pub mod cpio;
pub mod dvzip;
