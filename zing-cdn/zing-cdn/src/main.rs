use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use libp2p::{Multiaddr, identity, PeerId};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;
use walrus_core::metadata::BlobMetadataApi;
use walrus_core::BlobId;

use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::client::ZingClient;
use zing_cdn_core::mesh::reputation::PeerReputationTable;
use zing_cdn_core::mesh::resolver::Resolver;
use zing_cdn_core::p2p::node::{P2pCommand, ZingP2pNode};
use zing_cdn_core::sui::wallet::ZingWallet;
use zing_cdn_core::sui::settlement::SettlementConfig;
use zing_cdn_core::walrus::verify::BlobVerifier;

const CACHE_BUDGET_BYTES: u64 = 500 * 1024 * 1024; // 500 MB
const DEFAULT_CACHE_DIR: &str = "~/.zing-cdn/cache";

const DEFAULT_BOOTSTRAP: &[&str] = &[];

#[derive(Parser)]
#[command(name = "zing-cdn", about = "Walrus-native P2P content distribution mesh")]
struct Cli {
    #[arg(long, default_value = DEFAULT_CACHE_DIR)]
    cache_dir: String,

    #[arg(short, long)]
    verbose: bool,

    /// P2P listen address
    #[arg(long, default_value = "/ip4/0.0.0.0/udp/34291/quic-v1")]
    listen: Multiaddr,

    /// External/advertised addresses for this node (so peers can dial us back).
    /// Used to populate Kad provider records. May be specified multiple times.
    /// Example: --external-addr /ip4/203.0.113.5/udp/34291/quic-v1
    #[arg(long)]
    external_addr: Vec<Multiaddr>,

    /// Bootstrap peers (format: /ip4/.../udp/.../quic-v1/p2p/<peer_id>)
    #[arg(long, short = 'b')]
    bootstrap: Vec<String>,

    /// Keep P2P node alive after command completes
    #[arg(long)]
    serve: bool,

    /// Path to Sui CLI keystore config file for WAL token payments
    /// (default: auto-discovers ~/.sui/sui_config/client.yaml)
    #[arg(long)]
    sui_keystore: Option<String>,

    /// Sui package ID of the deployed zing_cdn settlement contract
    #[arg(long)]
    settlement_package: Option<String>,

    /// Object ID of the shared Settlement object
    #[arg(long)]
    settlement_object: Option<String>,

    /// Object ID of the peer's PeerVault (for routing payments)
    #[arg(long)]
    vault_object: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
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
            .add_directive("zing_cdn=debug".parse().unwrap())
    } else {
        EnvFilter::from_default_env()
            .add_directive("zing_cdn=info".parse().unwrap())
            .add_directive("walrus=warn".parse().unwrap())
            .add_directive("sui=warn".parse().unwrap())
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let cache_dir = resolve_cache_dir(&cli.cache_dir);
    std::fs::create_dir_all(&cache_dir).expect("create cache directory");
    let store = BlobStore::open(&cache_dir).expect("open blob store");
    let pinning = PinningManager::new(store.clone());
    let eviction = EvictionManager::new(store.clone(), CACHE_BUDGET_BYTES);
    let cache_store = Arc::new(RwLock::new(store));
    let cache_pinning = Arc::new(RwLock::new(pinning));
    let cache_eviction = Arc::new(RwLock::new(eviction));

    let store_handle = cache_store.clone();
    let keypair = load_or_generate_keypair(&cache_dir);
    tracing::info!(peer_id = %keypair.public().to_peer_id(), "P2P keypair loaded");

    let keystore_path = cli.sui_keystore.as_ref().map(|s| resolve_cache_dir(s));
    let wallet: Option<Arc<ZingWallet>> = match ZingWallet::from_keystore(keystore_path.as_deref(), None).await {
        Ok(w) => {
            tracing::info!(address = %w.address(), "Sui wallet loaded for WAL payments");
            Some(Arc::new(w))
        }
        Err(e) => {
            tracing::warn!(%e, "Sui wallet not available — running without WAL payments");
            None
        }
    };
    let sui_address_bytes = wallet.as_ref().map(|w| w.address().to_inner());

    let settlement_config: Option<SettlementConfig> = match (
        &cli.settlement_package,
        &cli.settlement_object,
    ) {
        (Some(pkg), Some(obj)) => {
            let package_id = pkg.parse()
                .map_err(|e| tracing::warn!(%e, "invalid settlement-package")).ok();
            let settlement_object_id = obj.parse()
                .map_err(|e| tracing::warn!(%e, "invalid settlement-object")).ok();
            let vault_object_id = cli.vault_object.as_ref()
                .and_then(|v| v.parse().ok());
            let wal_package_id = "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59"
                .parse()
                .ok();
            match (package_id, settlement_object_id, wal_package_id) {
                (Some(package_id), Some(settlement_object_id), Some(wal_package_id)) => Some(
                    SettlementConfig {
                        package_id,
                        settlement_object_id,
                        registry_object_id: settlement_object_id, // TODO: add --registry-object flag
                        vault_object_id,
                        wal_coin_type: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL".into(),
                        wal_package_id,
                    }
                ),
                _ => None,
            }
        }
        _ => None,
    };

    let (p2p_node, command_rx) = ZingP2pNode::new(store_handle, keypair);
    let p2p_peer_id = p2p_node.local_peer_id();
    let p2p_command_tx = p2p_node.command_tx().clone();
    let p2p_key = p2p_node.key().clone();
    let p2p_listen = cli.listen.clone();
    let mut bootstrap_peers_cli: Vec<String> = cli.bootstrap.clone();
    for bp in DEFAULT_BOOTSTRAP {
        let bp_str = bp.to_string();
        if !bootstrap_peers_cli.contains(&bp_str) {
            bootstrap_peers_cli.push(bp_str);
        }
    }
    let bootstrap_peers = parse_bootstrap_peers(&bootstrap_peers_cli);

    tracing::info!(peer_id = %p2p_peer_id, listen = %cli.listen, "starting P2P swarm");
    let p2p_store = cache_store.clone();
    let p2p_external_addrs = cli.external_addr.clone();
    tokio::spawn(async move {
        if let Err(e) = ZingP2pNode::run(
            p2p_key,
            command_rx,
            p2p_store,
            p2p_listen,
            bootstrap_peers,
            p2p_external_addrs,
            sui_address_bytes,
            None,
        )
        .await
        {
            tracing::error!(error = %e, "P2P swarm exited");
        }
    });

    let p2p_tx = if cli.bootstrap.is_empty() {
        None
    } else {
        Some(p2p_command_tx.clone())
    };

    if let Some(ref cmd) = cli.command {
        match cmd {
            Command::Get { ref blob_id } => {
                cmd_get(&cli, blob_id, &p2p_tx, &Some(p2p_peer_id), &cache_store, &cache_pinning, &cache_eviction, wallet.clone(), settlement_config.clone()).await
            }
            Command::Cat { ref blob_id } => {
                cmd_cat(&cli, blob_id, &p2p_tx, &Some(p2p_peer_id), &cache_store, &cache_pinning, &cache_eviction, wallet.clone(), settlement_config.clone()).await
            }
            Command::Metadata { ref blob_id } => cmd_metadata(&cli, blob_id).await,
            Command::Status { ref blob_id } => cmd_status(&cli, blob_id).await,
            Command::Verify { ref blob_id } => cmd_verify(&cli, blob_id).await,
            Command::List => cmd_list(&cache_store, &cache_pinning).await,
            Command::Pin { ref blob_id } => cmd_pin(blob_id, &cache_store, &cache_pinning).await,
            Command::Unpin { ref blob_id } => cmd_unpin(blob_id, &cache_pinning).await,
            Command::Info { ref blob_id } => cmd_info(blob_id, &cache_store, &cache_pinning).await,
        }?;
    } else if !cli.serve {
        Cli::parse_from(["--help"]);
        std::process::exit(1);
    }

    if cli.serve {
        tracing::info!("P2P node running. Press Ctrl+C to stop.");
        tokio::signal::ctrl_c().await?;
        tracing::info!("Shutting down...");
    }

    Ok(())
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


fn parse_bootstrap_peers(inputs: &[String]) -> Vec<(libp2p::PeerId, Multiaddr)> {
    inputs
        .iter()
        .filter_map(|s| {
            let mut addr = Multiaddr::from_str(s).ok()?;
            let mut peer_id = None;
            for protocol in addr.iter() {
                if let libp2p::multiaddr::Protocol::P2p(peer) = protocol {
                    peer_id = Some(peer);
                    break;
                }
            }
            let peer_id = peer_id?;
            addr.pop();
            Some((peer_id, addr))
        })
        .collect()
}

fn load_or_generate_keypair(cache_dir: &PathBuf) -> identity::Keypair {
    let path = cache_dir.join("keypair");
    if let Ok(data) = std::fs::read(&path) {
        if let Ok(kp) = identity::Keypair::from_protobuf_encoding(&data) {
            return kp;
        }
    }
    let kp = identity::Keypair::generate_ed25519();
    let data = kp.to_protobuf_encoding().expect("serialize keypair");
    if std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, &data))
        .is_err()
    {
        if let Ok(data) = std::fs::read(&path) {
            if let Ok(kp) = identity::Keypair::from_protobuf_encoding(&data) {
                return kp;
            }
        }
    }
    kp
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

async fn cmd_get(
    cli: &Cli,
    blob_id_str: &str,
    p2p_tx: &Option<mpsc::Sender<P2pCommand>>,
    p2p_peer_id: &Option<PeerId>,
    cache_store: &Arc<RwLock<BlobStore>>,
    cache_pinning: &Arc<RwLock<PinningManager>>,
    cache_eviction: &Arc<RwLock<EvictionManager>>,
    wallet: Option<Arc<ZingWallet>>,
    _settlement_config: Option<SettlementConfig>,
) -> anyhow::Result<()> {
    let blob_id = parse_blob_id(blob_id_str)?;
    let client = connect_mainnet().await?;
    let cache_dir = resolve_cache_dir(&cli.cache_dir);

    let mut resolver = Resolver::new(
        cache_store.clone(),
        cache_pinning.clone(),
        cache_eviction.clone(),
        client.walrus_client_arc(),
        Arc::new(BlobVerifier::new(client.encoding_config_arc())),
        Arc::new(RwLock::new(PeerReputationTable::new())),
        *p2p_peer_id,
    );
    if let Some(tx) = p2p_tx {
        resolver.set_p2p_channel(tx.clone());
    }
    if let Some(wallet) = wallet {
        resolver.set_wallet(wallet);
    }

    let result = resolver.resolve(&blob_id).await?;

    // Announce blob via P2P so other peers can discover it
    if let Some(tx) = p2p_tx {
        let blob_id_bytes = blob_id.0;
        let _ = tx
            .send(P2pCommand::AnnounceBlob {
                blob_id: blob_id_bytes,
            })
            .await;
    }

    let source_str = match result.resolution {
        zing_cdn_core::types::BlobResolution::LocalCache => "L0 local cache",
        zing_cdn_core::types::BlobResolution::L1Peer => "L1 peer",
        zing_cdn_core::types::BlobResolution::L3Walrus => "L3 Walrus",
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

async fn cmd_cat(
    _cli: &Cli,
    blob_id_str: &str,
    p2p_tx: &Option<mpsc::Sender<P2pCommand>>,
    p2p_peer_id: &Option<PeerId>,
    cache_store: &Arc<RwLock<BlobStore>>,
    cache_pinning: &Arc<RwLock<PinningManager>>,
    cache_eviction: &Arc<RwLock<EvictionManager>>,
    wallet: Option<Arc<ZingWallet>>,
    _settlement_config: Option<SettlementConfig>,
) -> anyhow::Result<()> {
    let blob_id = parse_blob_id(blob_id_str)?;
    let client = connect_mainnet().await?;

    let mut resolver = Resolver::new(
        cache_store.clone(),
        cache_pinning.clone(),
        cache_eviction.clone(),
        client.walrus_client_arc(),
        Arc::new(BlobVerifier::new(client.encoding_config_arc())),
        Arc::new(RwLock::new(PeerReputationTable::new())),
        *p2p_peer_id,
    );
    if let Some(tx) = p2p_tx {
        resolver.set_p2p_channel(tx.clone());
    }
    if let Some(wallet) = wallet {
        resolver.set_wallet(wallet);
    }

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

async fn cmd_list(
    cache_store: &Arc<RwLock<BlobStore>>,
    cache_pinning: &Arc<RwLock<PinningManager>>,
) -> anyhow::Result<()> {
    let store = cache_store.read().await;
    let pinning = cache_pinning.read().await;

    let ids = store.list_blob_ids()?;
    if ids.is_empty() {
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

async fn cmd_pin(
    blob_id_str: &str,
    cache_store: &Arc<RwLock<BlobStore>>,
    cache_pinning: &Arc<RwLock<PinningManager>>,
) -> anyhow::Result<()> {
    let store = cache_store.read().await;
    let exists = store.get(blob_id_str)?.is_some();
    if !exists {
        anyhow::bail!("blob {blob_id_str} not found in cache. fetch it first with `zing-cdn get {blob_id_str}`");
    }
    drop(store);

    let pinning = cache_pinning.read().await;
    pinning.pin(blob_id_str)?;
    println!("Blob {blob_id_str} pinned");
    Ok(())
}

async fn cmd_unpin(
    blob_id_str: &str,
    cache_pinning: &Arc<RwLock<PinningManager>>,
) -> anyhow::Result<()> {
    let pinning = cache_pinning.read().await;
    pinning.unpin(blob_id_str)?;
    println!("Blob {blob_id_str} unpinned");
    Ok(())
}

async fn cmd_info(
    blob_id_str: &str,
    cache_store: &Arc<RwLock<BlobStore>>,
    cache_pinning: &Arc<RwLock<PinningManager>>,
) -> anyhow::Result<()> {
    let store = cache_store.read().await;
    let data = match store.get(blob_id_str)? {
        Some(d) => d,
        None => anyhow::bail!("blob {blob_id_str} not found in cache"),
    };
    drop(store);

    let pinning = cache_pinning.read().await;
    let pinned = pinning.is_pinned(blob_id_str)?;
    let size = data.len();

    println!("Blob ID:  {blob_id_str}");
    println!("State:    {}", if pinned { "Pinned" } else { "Cached" });
    println!("Pinned:   {}", if pinned { "yes" } else { "no" });
    println!("Size:     {} ({} bytes)", format_size(size as u64), size);

    Ok(())
}
