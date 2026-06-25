//! awdl-rs — Rust port of owl (Apple Wireless Direct Link daemon).
//!
//! See PORT.md in this crate for the full implementation spec and the
//! hard-won lessons (hardware-TSF sync, monitor mode, channel 44, etc.).
//!
//! Reference: ../../owl/  (C source; note our hardware-TSF patch in owl/src/rx.c)

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "awdl_rs=debug,info".into()),
        )
        .init();

    tracing::info!("awdl-rs starting (skeleton — see PORT.md)");
    // TODO(gpt-5.5): implement per PORT.md
    Ok(())
}
