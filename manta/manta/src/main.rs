use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;
use walrus_core::metadata::BlobMetadataApi;
use walrus_core::BlobId;

use manta_core::cache::eviction::EvictionManager;
use manta_core::cache::pinning::PinningManager;
use manta_core::cache::store::BlobStore;
use manta_core::client::ZingClient;
use manta_core::mesh::reputation::PeerReputationTable;
use manta_core::mesh::resolver::Resolver;
use manta_core::walrus::verify::BlobVerifier;

const CACHE_BUDGET_BYTES: u64 = 500 * 1024 * 1024; // 500 MB
const DEFAULT_CACHE_DIR: &str = "~/.manta/cache";

#[derive(Parser)]
#[command(name = "manta", about = "Walrus-native P2P content distribution mesh")]
struct Cli {
    #[arg(long, default_value = DEFAULT_CACHE_DIR)]
    cache_dir: String,

    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch blob from Walrus and cache locally
    Get { blob_id: String },
    /// Fetch blob and write raw bytes to stdout
    Cat { blob_id: String },
    /// Fetch and display blob metadata
    Metadata { blob_id: String },
    /// Check blob status on Walrus
    Status { blob_id: String },
    /// Read blob and verify Blake2b-256 blob ID
    Verify { blob_id: String },
    /// List all cached blobs
    List,
    /// Pin a cached blob
    Pin { blob_id: String },
    /// Unpin a cached blob
    Unpin { blob_id: String },
    /// Show cached blob info
    Info { blob_id: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::from_default_env()
            .add_directive("manta=debug".parse().unwrap())
    } else {
        EnvFilter::from_default_env()
            .add_directive("manta=info".parse().unwrap())
            .add_directive("walrus=warn".parse().unwrap())
            .add_directive("sui=warn".parse().unwrap())
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Command::Get { ref blob_id } => cmd_get(&cli, blob_id).await,
        Command::Cat { ref blob_id } => cmd_cat(&cli, blob_id).await,
        Command::Metadata { ref blob_id } => cmd_metadata(&cli, blob_id).await,
        Command::Status { ref blob_id } => cmd_status(&cli, blob_id).await,
        Command::Verify { ref blob_id } => cmd_verify(&cli, blob_id).await,
        Command::List => cmd_list(&cli).await,
        Command::Pin { ref blob_id } => cmd_pin(&cli, blob_id).await,
        Command::Unpin { ref blob_id } => cmd_unpin(&cli, blob_id).await,
        Command::Info { ref blob_id } => cmd_info(&cli, blob_id).await,
    }
}

fn resolve_cache_dir(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    }
}

fn open_cache(cache_dir: &PathBuf) -> (BlobStore, PinningManager, EvictionManager) {
    std::fs::create_dir_all(cache_dir).expect("create cache directory");
    let store = BlobStore::open(cache_dir).expect("open blob store");
    let pinning = PinningManager::new(store.clone());
    let eviction = EvictionManager::new(store.clone(), CACHE_BUDGET_BYTES);
    (store, pinning, eviction)
}

async fn connect_mainnet() -> anyhow::Result<ZingClient> {
    tracing::info!("connecting to Walrus mainnet");
    let client = ZingClient::from_mainnet().await?;
    tracing::info!("connected to Walrus mainnet");
    Ok(client)
}

fn parse_blob_id(s: &str) -> anyhow::Result<BlobId> {
    s.parse::<BlobId>()
        .map_err(|_| anyhow::anyhow!("invalid blob ID: {s}"))
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{size} {}", UNITS[unit_idx])
    } else {
        format!("{size:.2} {}", UNITS[unit_idx])
    }
}

// ── Commands ──────────────────────────────────────────────────────

async fn cmd_get(cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let blob_id = parse_blob_id(blob_id_str)?;
    let client = connect_mainnet().await?;
    let cache_dir = resolve_cache_dir(&cli.cache_dir);

    let resolver = {
        let (store, pinning, eviction) = open_cache(&cache_dir);
        let walrus_client = client.walrus_client_arc();
        let verifier = Arc::new(BlobVerifier::new(client.encoding_config_arc()));
        Resolver::new(
            Arc::new(RwLock::new(store)),
            Arc::new(RwLock::new(pinning)),
            Arc::new(RwLock::new(eviction)),
            walrus_client,
            verifier,
            Arc::new(RwLock::new(PeerReputationTable::new())),
        )
    };

    let result = resolver.resolve(&blob_id).await?;

    let source_str = match result.resolution {
        manta_core::types::BlobResolution::LocalCache => "L0 local cache",
        manta_core::types::BlobResolution::L1Peer => "L1 peer",
        manta_core::types::BlobResolution::L3Walrus => "L3 Walrus",
    };

    let cache_str = if result.cached {
        format!("yes (at {})", cache_dir.display())
    } else {
        "now cached".to_string()
    };

    println!("Blob:    {blob_id_str}");
    println!("Size:    {} ({} bytes)", format_size(result.data.len() as u64), result.data.len());
    println!("Source:  {source_str}");
    println!("Cached:  {cache_str}");

    Ok(())
}

async fn cmd_cat(cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let blob_id = parse_blob_id(blob_id_str)?;
    let client = connect_mainnet().await?;
    let cache_dir = resolve_cache_dir(&cli.cache_dir);

    let resolver = {
        let (store, pinning, eviction) = open_cache(&cache_dir);
        let walrus_client = client.walrus_client_arc();
        let verifier = Arc::new(BlobVerifier::new(client.encoding_config_arc()));
        Resolver::new(
            Arc::new(RwLock::new(store)),
            Arc::new(RwLock::new(pinning)),
            Arc::new(RwLock::new(eviction)),
            walrus_client,
            verifier,
            Arc::new(RwLock::new(PeerReputationTable::new())),
        )
    };

    let result = resolver.resolve(&blob_id).await?;
    // Write raw bytes to stdout (stderr is used for tracing)
    use std::io::Write;
    std::io::stdout().write_all(&result.data)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn cmd_metadata(_cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let blob_id = parse_blob_id(blob_id_str)?;
    let client = connect_mainnet().await?;

    let metadata = client.fetch_metadata(&blob_id).await?;
    let m = metadata.metadata();

    println!("Blob ID:         {blob_id_str}");
    println!("Unencoded:        {} bytes", m.unencoded_length());
    println!("Encoding Type:    {:?}", m.encoding_type());

    Ok(())
}

async fn cmd_status(_cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let blob_id = parse_blob_id(blob_id_str)?;
    let client = connect_mainnet().await?;

    let status = client.check_blob_status(&blob_id).await?;

    println!("Blob ID:    {blob_id_str}");
    match status {
        walrus_storage_node_client::api::BlobStatus::Permanent {
            initial_certified_epoch,
            end_epoch,
            ..
        } => {
            println!("Status:     permanent");
            println!("Certified:  epoch {}", initial_certified_epoch.map(|e| e.to_string()).unwrap_or_else(|| "none".into()));
            println!("End Epoch:  {end_epoch}");
        }
        walrus_storage_node_client::api::BlobStatus::Deletable {
            initial_certified_epoch,
            ..
        } => {
            println!("Status:     deletable");
            println!("Certified:  epoch {}", initial_certified_epoch.map(|e| e.to_string()).unwrap_or_else(|| "none".into()));
        }
        walrus_storage_node_client::api::BlobStatus::Invalid { .. } => {
            println!("Status:     invalid");
        }
        walrus_storage_node_client::api::BlobStatus::Nonexistent => {
            println!("Status:     not found");
        }
    }

    Ok(())
}

async fn cmd_verify(_cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let blob_id = parse_blob_id(blob_id_str)?;
    let client = connect_mainnet().await?;

    tracing::info!("fetching blob data");
    let data = client.read_blob(&blob_id).await?;

    tracing::info!("fetching blob metadata");
    let metadata = client.fetch_metadata(&blob_id).await?;

    let verifier = BlobVerifier::new(client.encoding_config_arc());
    match verifier.verify_blob_against_metadata(&metadata, &data) {
        Ok(()) => {
            println!("Verification: ✅ PASSED");
            println!("  Computed: {blob_id_str}");
            println!("  Expected: {blob_id_str}");
        }
        Err(e) => {
            println!("Verification: ❌ FAILED");
            println!("  Error: {e}");
        }
    }

    Ok(())
}

async fn cmd_list(cli: &Cli) -> anyhow::Result<()> {
    let cache_dir = resolve_cache_dir(&cli.cache_dir);
    let (store, pinning, _eviction) = open_cache(&cache_dir);

    let ids = store.list_blob_ids()?;
    if ids.is_empty() {
        println!("No blobs cached (cache dir: {})", cache_dir.display());
        return Ok(());
    }

    println!("Cached Blobs:");
    for id in &ids {
        let size = store.blob_size(id)?.unwrap_or(0);
        let pinned = pinning.is_pinned(id)?;
        println!(
            "  {id}  {}  pinned: {}",
            format_size(size),
            if pinned { "yes" } else { "no " }
        );
    }

    Ok(())
}

async fn cmd_pin(cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let cache_dir = resolve_cache_dir(&cli.cache_dir);
    let (store, pinning, _eviction) = open_cache(&cache_dir);

    let exists = store.get(blob_id_str)?.is_some();
    if !exists {
        anyhow::bail!("blob {blob_id_str} not found in cache. fetch it first with `manta get {blob_id_str}`");
    }

    pinning.pin(blob_id_str)?;
    println!("Blob {blob_id_str} pinned");
    Ok(())
}

async fn cmd_unpin(cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let cache_dir = resolve_cache_dir(&cli.cache_dir);
    let (_store, pinning, _eviction) = open_cache(&cache_dir);

    pinning.unpin(blob_id_str)?;
    println!("Blob {blob_id_str} unpinned");
    Ok(())
}

async fn cmd_info(cli: &Cli, blob_id_str: &str) -> anyhow::Result<()> {
    let cache_dir = resolve_cache_dir(&cli.cache_dir);
    let (store, pinning, _eviction) = open_cache(&cache_dir);

    let data = match store.get(blob_id_str)? {
        Some(d) => d,
        None => anyhow::bail!("blob {blob_id_str} not found in cache"),
    };

    let pinned = pinning.is_pinned(blob_id_str)?;
    let size = data.len();

    println!("Blob ID:  {blob_id_str}");
    println!("State:    {}", if pinned { "Pinned" } else { "Cached" });
    println!("Pinned:   {}", if pinned { "yes" } else { "no" });
    println!("Size:     {} ({} bytes)", format_size(size as u64), size);

    Ok(())
}
