//! filin-rs — Rust port of owl (Apple Wireless Direct Link daemon).
//!
//! See PORT.md in this crate for the full implementation spec and the
//! hard-won lessons (hardware-TSF sync, monitor mode, channel 44, etc.).
//!
//! Reference: ../../owl/  (C source; note our hardware-TSF patch in owl/src/rx.c)

fn main() -> anyhow::Result<()> {
    use tracing_subscriber::prelude::*;

    // Build the introspection handle first so both the fmt layer and the
    // ring-buffer TraceLayer can be installed before any tracing! call. The
    // fmt layer keeps its own EnvFilter; the TraceLayer is unfiltered (sees
    // every event) and gates capture internally via its AtomicU8 — so the
    // `/trace` endpoint can surface debug/trace events even when RUST_LOG
    // is set to info+ on the console.
    let introspect = filin_rs::introspect::Introspection::new();
    let fmt_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "filin_rs=debug,info".into());
    let registry = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(fmt_filter))
        .with(introspect.trace_layer());
    tracing::subscriber::set_global_default(registry)
        .map_err(|err| anyhow::anyhow!("failed to install tracing subscriber: {err:?}"))?;

    tracing::info!("filin starting");
    let config = filin_rs::cli::parse_config_from_env()
        .map_err(|err| anyhow::anyhow!("invalid CLI arguments: {err:?}"))?;
    tracing::info!(
        monitor_iface = %config.monitor_iface,
        host_iface = %config.host_iface,
        anchor_channel = config.anchor_channel,
        assume_monitor = config.assume_monitor,
        pcap_path = ?config.pcap_path,
        http_addr = ?config.http_addr,
        "validated filin configuration"
    );
    let links = filin_rs::runtime::open_links(&config)
        .map_err(|err| anyhow::anyhow!("failed to open filin links: {err:?}"))?;
    tracing::info!("filin links are up");
    filin_rs::runtime::run(links, introspect, config.http_addr)
        .map_err(|err| anyhow::anyhow!("filin runtime stopped: {err:?}"))
}
