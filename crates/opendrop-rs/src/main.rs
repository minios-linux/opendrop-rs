//! opendrop-rs — top-level CLI orchestrating filin-rs + luftlift-rs.
//!
//! Brings up AWDL (filin-rs) and runs AirDrop (luftlift-rs) over it, with a unified
//! status/debug view (peers, master, sync%, discovered services, transfers).

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tracing::info!("opendrop-rs (orchestrator skeleton)");
    Ok(())
}
