//! airdrop-rs — Rust port of opendrop (the AirDrop application layer).
//!
//! Runs over the `awdl0` interface that `awdl-rs` brings up. See PORT.md for the
//! full spec and the reverse-engineered iOS-18 protocol details (dvzip, chunked
//! Ask/Discover, etc.).
//!
//! Reference: ../../opendrop-patch/ (patched Python opendrop + dvzip.py decoder).

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "airdrop_rs=debug,info".into()),
        )
        .init();

    tracing::info!("airdrop-rs starting (skeleton — see PORT.md)");
    // TODO(glm-5.2): implement per PORT.md
    Ok(())
}
