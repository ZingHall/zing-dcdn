# Zing: The Decentralized Edge CDN for Walrus

Zing is a high-performance, decentralized Content Delivery Network (dCDN) built natively on top of the Walrus storage protocol. While Walrus provides permanent, highly resilient data storage using two-dimensional erasure coding (Red Stuff), retrieving that data directly from core storage nodes introduces computational overhead and multi-node network latency. Zing solves this by introducing a lightning-fast, peer-to-peer edge caching layer.

Built in Rust, Zing allows node operators to aggregate, cache, and redistribute fully reconstructed Walrus blobs directly to end users over high-speed libp2p QUIC connections. By decoupling data availability (handled by Walrus) from data delivery (handled by Zing), developers get the decentralized permanence of blockchain storage with the zero-latency delivery of a commercial CDN.

### Core Architecture

Zing operates on a seamless multi-tiered caching architecture that optimizes for speed, bandwidth, and cost:

* 
**The Hot Path (L1 Edge Cache):** When a user requests a file, Zing queries a gasless Kademlia Distributed Hash Table (DHT) to instantly discover nearby edge nodes holding the fully reconstructed file. The file is then streamed 1:1 via a direct QUIC connection, completely bypassing the computational cost of erasure decoding.


* 
**The Cold Path (L3 Walrus Fallback):** If an asset is entirely missing from the edge network, Zing seamlessly falls back to acting as a standard Walrus client. It queries the Sui blockchain for the active storage committee, pulls the required cryptographic slivers, reconstructs the blob, and promotes it to the local L1 cache to become a new seeder for the network.


* 
**Trustless Edge Verification:** To ensure malicious edge nodes cannot serve tampered data, Zing utilizes a "Metadata Pre-Fetch" mechanism. Before streaming a cached file from an unknown peer, the client fetches the tiny, authenticated metadata payload directly from the Walrus committee. The incoming edge stream is then cryptographically verified against this trusted baseline, ensuring complete data integrity without sacrificing speed.



### Built for Operators & Developers

Zing is designed to support localized commercial edge caches, empowering operators to monetize reads or allowing dApp developers to explicitly "pin" their web assets (like frontends or NFT media) for permanent, wire-speed delivery to their users.
