use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("zing=info".parse().unwrap()))
        .init();

    tracing::info!("Zing — Walrus-native P2P content distribution mesh");
    tracing::info!("Desktop UI not yet implemented. Running in headless mode.");

    // TODO: Initialize zing-core components and run the event loop.
    // This will be wired up during the Tauri + Dioxus integration phase.
    // For now, the app starts and exits immediately.

    Ok(())
}