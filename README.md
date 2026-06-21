# zing-cdn: The Decentralized Edge CDN for Walrus

**zing-cdn** is a core infrastructure component of the broader **Zing** ecosystem. Zing functions as a decentralized knowledge layer — a sovereign platform for storing, organizing, and retrieving information. zing-cdn provides the high-performance P2P content delivery network that powers it, replacing traditional CDNs like Cloudflare and AWS CloudFront with a cryptoeconomically incentivized peer-to-peer mesh. Together, they form a complete decentralization stack: sovereign knowledge hosted on Walrus, delivered at wire speed via zing-cdn's edge network.

zing-cdn is a high-performance, decentralized Content Delivery Network (dCDN) built natively on top of the Walrus storage protocol. While Walrus provides permanent, highly resilient data storage using two-dimensional erasure coding (Red Stuff), retrieving that data directly from core storage nodes introduces computational overhead and multi-node network latency. zing-cdn solves this by introducing a lightning-fast, peer-to-peer edge caching layer.

Built in Rust, zing-cdn allows node operators to aggregate, cache, and redistribute fully reconstructed Walrus blobs directly to end users over high-speed libp2p QUIC connections. By decoupling data availability (handled by Walrus) from data delivery (handled by zing-cdn), developers get the decentralized permanence of blockchain storage with wire-speed delivery — no more reliance on Cloudflare, AWS CloudFront, or any centralized CDN.

## The Problem: The "Tragedy of the Commons"

Walrus is a highly resilient storage network, but its current architecture faces three critical bottlenecks regarding data retrieval (reads):

- **The Public Good Dilemma:** In the core Walrus protocol, storage nodes are compensated via staking to store data, but serving reads is treated as a best-effort, free public good. Rational node operators are incentivized to hoard their egress bandwidth rather than serve free data, risking a "tragedy of the commons" where data is safely stored but practically inaccessible.

- **Aggregator Burnout:** Standard Walrus Aggregators operate permissionlessly, meaning operators pay for server compute and egress bandwidth entirely out of pocket with zero monetization. Running a high-traffic gateway for free is not a sustainable business model.

- **The Compute Bottleneck:** Every time a standard aggregator fetches an uncached file, it must download slivers from a quorum of global storage nodes and execute heavy finite-field linear algebra (Red Stuff decoding) on the fly. Traffic spikes instantly bottleneck the aggregator's CPU.

## The Solution: A Decentralized CDN with On-Chain Staking

zing-cdn decouples data availability (handled by core Walrus L3 nodes) from data delivery (handled by the zing-cdn L1 edge mesh). By introducing a native bandwidth market powered by a staking and delegation mechanism, zing-cdn turns read-serving from a sunk cost into a profitable enterprise.

### How the Economic Engine Works

**Incentivized Edge Caching:** zing-cdn edge nodes aggregate and cache fully reconstructed Walrus blobs. When end-users request a file, they pay a micro-fee in WAL to stream the pre-decoded raw binary directly from a nearby peer, bypassing the heavy CPU decode bottleneck and dropping latency to near-zero.

**Proof of Stake for Registration:** Node operators must stake WAL to register as a peer on-chain. This stake acts as a security bond — it's reclaimable but slashable — ensuring only committed operators participate in the network.

**Per-Peer Delegation Vaults (LST-like Mechanism):** Each registered peer can create a `PeerVault` to accept delegated WAL from the community. Delegators deposit WAL into a specific peer's vault and receive a `ShareCertificate` — an on-chain receipt representing their proportional ownership of the vault's reserves. As the peer serves more data and earns WAL read fees, the vault's reserves grow, increasing the value of each share. Delegators can burn their `ShareCertificate` at any time to withdraw their original deposit plus accrued yield. This implements a similar economic flywheel to a Liquid Staking Token (LST) without requiring a fungible Coin in the MVP — the `ShareCertificate` is an NFT-like receipt with a planned upgrade to per-peer liquid `Coin<PeerLST>`.

**Commission + Yield Split:** When a peer serves data and receives a WAL payment, the smart contract splits the reward: a configurable commission (default 10%) goes to the peer's accumulated earnings, while the remainder flows directly into the vault's reserves. This means delegators earn yield passively simply by backing a high-performing edge node.

**Claim & Undelegate:** Peers can claim their accumulated commission earnings at any time. Delegators can undelegate by burning their `ShareCertificate`, receiving WAL back at the current exchange rate (original deposit + accrued yield).

### How This Supercharges the Walrus Ecosystem

- **Protects the Core Network:** By absorbing viral read traffic at the edge and serving it peer-to-peer, zing-cdn protects the primary Walrus L3 storage committee from bandwidth exhaustion.
- **Guarantees Sustainability:** It replaces the fragile reliance on altruistic aggregators with a robust, crypto-economically secured market. Node operators are incentivized to provide stellar uptime and high speeds to attract delegated stake.
- **Unlocks Web2-Level Performance:** By utilizing Kademlia DHT for localized peer discovery and streaming fully reconstructed binaries, zing-cdn delivers Walrus-hosted content at wire speeds, making decentralized storage viable for high-demand consumer dApps, video streaming, and rapid frontend delivery.

## The Zing Ecosystem

zing-cdn is one component of the Zing platform:

- **Zing** — A decentralized knowledge layer for storing, organizing, and retrieving information with sovereign control. A sovereign alternative to centralized platforms like Google Drive, Notion, and GitHub.
- **zing-cdn** — The P2P edge CDN that delivers Zing's (and any Walrus-hosted) content at wire speeds. Replaces Cloudflare, AWS CloudFront, and other centralized CDNs with a decentralized, cryptoeconomically incentivized peer-to-peer mesh.

Together, they complete the vision of end-to-end decentralization: data permanence via Walrus, peer-to-peer delivery via zing-cdn, and a sovereign knowledge interface via Zing.

This repository contains **zing-cdn only**. For the Zing knowledge layer, see the [Zing](https://github.com/ZingHall) project.

## Core Architecture

zing-cdn operates on a seamless multi-tiered caching architecture:

### The Hot Path (L1 Edge Cache)

When a user requests a file, zing-cdn queries a gasless Kademlia Distributed Hash Table (DHT) to instantly discover nearby edge nodes holding the fully reconstructed file. The file is then streamed 1:1 via a direct QUIC connection, completely bypassing the computational cost of erasure decoding.

### The Cold Path (L3 Walrus Fallback)

If an asset is entirely missing from the edge network, zing-cdn seamlessly falls back to acting as a standard Walrus client. It queries the Sui blockchain for the active storage committee, pulls the required cryptographic slivers, reconstructs the blob, and promotes it to the local L1 cache to become a new seeder for the network.

### Trustless Edge Verification

To ensure malicious edge nodes cannot serve tampered data, zing-cdn utilizes a "Metadata Pre-Fetch" mechanism. Before streaming a cached file from an unknown peer, the client fetches the tiny, authenticated metadata payload directly from the Walrus committee. The incoming edge stream is then cryptographically verified against this trusted baseline, ensuring complete data integrity without sacrificing speed.

## CLI Usage

### Install & Build

```bash
cd zing-cdn && cargo build --release
```

### Fetch a Blob

```bash
./target/release/zing-cdn get <blob_id>
./target/release/zing-cdn cat <blob_id>
./target/release/zing-cdn metadata <blob_id>
./target/release/zing-cdn status <blob_id>
./target/release/zing-cdn verify <blob_id>
```

### Run a Node

```bash
./target/release/zing-cdn serve
```

The HTTP API is available at `http://127.0.0.1:8080`.

### HTTP API Endpoints

```
GET  /api/v1/resolve?blob_id=<id>           # Resolve and return blob as JSON
GET  /api/v1/resolve/stream?blob_id=<id>     # SSE progress stream
GET  /api/v1/health                          # Node health status
GET  /api/v1/peers                           # List connected peers
GET  /api/v1/cache                           # List cached blobs
GET  /api/v1/staking                         # List all registered peers
GET  /api/v1/my_peer                         # My peer registration info
GET  /api/v1/balance                         # WAL balance
GET  /api/v1/my_vault                        # My vault info
GET  /api/v1/my_shares                       # My delegation shares
POST /api/v1/register                        # Register as a peer
POST /api/v1/update_peer_id                  # Update on-chain peer ID
POST /api/v1/create_vault                    # Create a delegation vault
POST /api/v1/claim_earnings                  # Claim commission earnings
POST /api/v1/delegate?vault_object_id=...&amount=...  # Delegate WAL to a peer
POST /api/v1/undelegate?cert_object_id=...   # Undelegate (burn ShareCertificate)
```

## GUI

zing-cdn includes a desktop GUI built with Tauri + Dioxus:

```bash
cd zing-cdn/zing-cdn-gui && cargo tauri dev
```

The GUI provides a dashboard, staking management (peer registration, delegate/undelegate, claim earnings), cache management, and peer browsing.

## Smart Contracts

Deployed on Sui mainnet:

| Contract | Object ID |
|----------|-----------|
| Package | `0x839f026743cb42e760b55a3a931ddbc7bf391ba5708ec303a4e9912b88fbff95` |
| Registry | `0x97b5153b9e9897ad1630cdd06e5caa81ebbf8865e96003f38e50c5f1d6752527` |
| Settlement | `0xc58e9b7417fdc83743b46a3f9009b10868f05bb1f2283f08c7021ac3e7f6c308` |
| PeerVaultRegistry | `0x9b96aa341bc3749283f9320ae783f2e6aff86b6393a45aeeedc53946f089d615` |
| PeerVault (shared) | `0x232e4605b0f81f8656f70451545ba279c5beb886f3e56796d3a3fa777f44e7ef` |
| Registry Peers Table | `0xbcd17d4df8489569fdca7bc9a795c16a73560efbde2355d91ef9195bf676ea00` |
| PeerVaults Table | `0x465bf3e99dff79a56705b111396ee5b9bd35f2a1aac70d118f466a7c581e0e07` |

### Contract Modules

- **`staking.move`** — Peer registration, self-stake bonds, peer ID updates, slashing (admin)
- **`settlement.move`** — On-chain payment routing, WAL fee collection, reward distribution
- **`peer_vault.move`** — Per-peer delegation vaults, ShareCertificate minting/burning, commission claims
- **`utils.move`** — `mul_div` helper for exchange rate calculations

### Fee Model

- **FetchBlob:** Fixed `READ_FEE_WAL_NANOS` per blob
- **FetchRange:** `FEE_PER_BYTE_NANOS × length` for partial blob reads

## Configuration

Create `~/.zing-cdn/config.toml`:

```toml
[settlement]
package = "0x839f026743cb42e760b55a3a931ddbc7bf391ba5708ec303a4e9912b88fbff95"
settlement_object = "0xc58e9b7417fdc83743b46a3f9009b10868f05bb1f2283f08c7021ac3e7f6c308"
vault_object = "0x232e4605b0f81f8656f70451545ba279c5beb886f3e56796d3a3fa777f44e7ef"
registry_object = "0x97b5153b9e9897ad1630cdd06e5caa81ebbf8865e96003f38e50c5f1d6752527"
registry_peers_table = "0xbcd17d4df8489569fdca7bc9a795c16a73560efbde2355d91ef9195bf676ea00"
peer_vaults_table = "0x465bf3e99dff79a56705b111396ee5b9bd35f2a1aac70d118f466a7c581e0e07"
peer_vault_registry = "0x9b96aa341bc3749283f9320ae783f2e6aff86b6393a45aeeedc53946f089d615"
registry_version = 921074118
settlement_version = 921074118
vault_version = 921074119
peer_vaults_version = 923306507
peer_vault_registry_version = 923306507
```

The wallet is loaded from `~/.sui/sui_config/sui.keystore` or the `ZING_CDN_SUI_PRIVATE_KEY` environment variable.

## Fly.io Deployment

The reference node is deployed at `213.188.208.246`:

| Protocol | Port | Purpose |
|----------|------|---------|
| UDP/QUIC | 34291 | libp2p P2P |
| TCP | 8080 | HTTP API |

```bash
fly deploy
```

## Roadmap

- [x] P2P blob discovery via Kademlia DHT
- [x] Multi-tier caching (L0 local → L1 peers → L3 Walrus)
- [x] Trustless blob verification via Walrus metadata
- [x] On-chain staking & peer registration (Sui)
- [x] WAL read fee payment via on-chain settlement
- [x] Per-peer delegation vaults with ShareCertificate receipt
- [x] Claim earnings & undelegate
- [x] Delegate WAL to peers (inline GUI)
- [x] Desktop GUI (Tauri + Dioxus)
- [x] HTTP API for programmatic blob resolution
- [ ] Routing priority by stake weight (planned)
- [ ] Per-peer liquid `Coin<PeerLST>` upgrade from ShareCertificate
- [ ] Admin slashing mechanism
