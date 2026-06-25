//! filin-rs — Rust port of owl (Apple Wireless Direct Link daemon).
//!
//! See PORT.md in this crate for the full implementation spec and the
//! hard-won lessons (hardware-TSF sync, monitor mode, channel 44, etc.).
//!
//! Reference: ../../owl/  (C source; note our hardware-TSF patch in owl/src/rx.c)

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "filin_rs=debug,info".into()),
        )
        .init();

    tracing::info!("filin starting (skeleton — see PORT.md)");
    let config = filin_rs::cli::parse_config_from(std::env::args_os())
        .map_err(|err| anyhow::anyhow!("invalid CLI arguments: {err:?}"))?;
    tracing::info!(
        monitor_iface = %config.monitor_iface,
        host_iface = %config.host_iface,
        anchor_channel = config.anchor_channel,
        assume_monitor = config.assume_monitor,
        "validated filin configuration"
    );
    let links = filin_rs::runtime::open_links(&config)
        .map_err(|err| anyhow::anyhow!("failed to open filin links: {err:?}"))?;
    tracing::info!("filin links are up");
    filin_rs::runtime::run(links).map_err(|err| anyhow::anyhow!("filin runtime stopped: {err:?}"))
}
