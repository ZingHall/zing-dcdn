# zing-cdn Server API

**Base URL:** `http://zing-cdn.fly.dev:8080`

## 1. Fetch Blob (SSE progress)

Streams real-time progress events via Server-Sent Events.

```bash
curl -N "http://zing-cdn.fly.dev:8080/api/v1/resolve/stream?blob_id=g_gcy5hprX7e3W-JyjomAgMoKSABMPF2K0tUNY8Y12w"
```

**Events:**
```
{"type":"status","status":"Checking local cache...","layer":"L0"}
{"type":"status","status":"Searching P2P network...","layer":"L1"}
{"type":"result","info":{"blob_id":"...","size":123,"source":"L1 peer","cached":true,"content":"...","mime_type":"text/plain","data_base64":"","payment_error":null}}
```

| Event Type | Description |
|------------|-------------|
| `status` | Progress update (L0 = local cache, L1 = P2P, L3 = Walrus) |
| `result` | Final resolved blob data with metadata |
| `error` | Error message |

## 2. Fetch Blob (blocking)

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/resolve?blob_id=g_gcy5hprX7e3W-JyjomAgMoKSABMPF2K0tUNY8Y12w"
```

**Response:**
```json
{
  "blob_id": "g_gcy5hprX7e3W-JyjomAgMoKSABMPF2K0tUNY8Y12w",
  "size": 123,
  "source": "L1 peer",
  "cached": true,
  "content": "Blob content (truncated at 2000 chars for text, binary summary for images)",
  "mime_type": "text/plain",
  "data_base64": "",
  "payment_error": null
}
```

| Field | Description |
|-------|-------------|
| `source` | Resolution source: `L0 local cache`, `L1 peer`, or `L3 Walrus` |
| `cached` | Whether blob is now cached locally |
| `payment_error` | If payment failed during fetch (e.g. insufficient WAL balance) |

## 3. Health Check

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/health"
```

**Response:**
```json
{
  "status": "ok",
  "peer_id": "12D3KooWKVF9AyVPbo9uWh37rvQVcSHc5b6JPs5f3kp16CT3yMWs",
  "connected_peers": 2
}
```

## 4. List Peers

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/peers"
```

**Response:**
```json
{
  "bootstrap": ["/ip4/213.188.208.246/udp/34291/quic-v1/p2p/12D3KooWKVF9AyVPbo9uWh37rvQVcSHc5b6JPs5f3kp16CT3yMWs"],
  "connected": ["12D3KooWK...", "12D3KooWL..."],
  "listen_addr": "/ip4/0.0.0.0/udp/34291/quic-v1",
  "cache_dir": "/root/.zing-cdn/cache",
  "peer_id": "12D3KooWK...",
  "p2p_addr": "/ip4/127.0.0.1/udp/34291/quic-v1/p2p/12D3KooWK..."
}
```

| Field | Description |
|-------|-------------|
| `bootstrap` | Configured bootstrap peers (addresses) |
| `connected` | Currently connected peer IDs |
| `p2p_addr` | Full multiaddr for other nodes to dial this node |

## 5. Add Peer

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/peers/add?addr=/ip4/1.2.3.4/udp/34291/quic-v1/p2p/12D3KooW..."
```

## 6. List Cache

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/cache"
```

**Response:**
```json
[
  {"blob_id": "g_gcy5hprX7e3W-JyjomAgMoKSABMPF2K0tUNY8Y12w", "size": 123, "pinned": false},
  {"blob_id": "abc123...", "size": 456, "pinned": true}
]
```

## 7. Staking — List Registered Peers

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/staking"
```

**Response:**
```json
[
  {
    "sui_address": "0x6e81458695e92a4da0c888f2ad9df9dd6e95d4e4d7611793a1d1fb370c30c304",
    "peer_id_short": "12D3KooW...goZN2Bwa",
    "bond": 4454000000,
    "is_active": true,
    "is_live": false
  },
  {
    "sui_address": "0x0b3fc768f8bb3c772321e3e7781cac4a45585b4bc64043686beb634d65341798",
    "peer_id_short": "12D3KooW...6CT3yMWs",
    "bond": 1000000000,
    "is_active": true,
    "is_live": true
  }
]
```

| Field | Description |
|-------|-------------|
| `sui_address` | Wallet address of the registered peer |
| `peer_id_short` | Truncated PeerId (`{first 8}...{last 8}`) |
| `bond` | Stake amount in WAL nanos |
| `is_active` | On-chain active flag (registered / not withdrawn) |
| `is_live` | Currently connected to this node via P2P |

## 8. My Peer — Check Own Registration

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/my_peer"
```

**Response (registered):**
```json
{
  "wallet_address": "0x199ca42b8c437bd4e1a440e0d15667bfc27291725680bb7a51bc4248165f8603",
  "peer_id_short": "12D3KooW...VVKGFE3R",
  "bond": 1000000000,
  "is_active": true,
  "is_live": false,
  "is_registered": true
}
```

**Response (not registered):**
```json
{
  "wallet_address": "0x199c...",
  "peer_id_short": null,
  "bond": null,
  "is_active": null,
  "is_live": null,
  "is_registered": false
}
```

## 9. WAL Balance

```bash
curl "http://zing-cdn.fly.dev:8080/api/v1/balance"
```

**Response:**
```json
{
  "balance": 1009934456,
  "balance_wal": "1.009934456"
}
```

| Field | Description |
|-------|-------------|
| `balance` | Raw WAL balance in nanos (1 WAL = 1,000,000,000 nanos) |
| `balance_wal` | Formatted balance with 9 decimal places |

## 10. Register Peer

Registers the node's PeerId on-chain with the configured wallet.

```bash
curl -X POST "http://zing-cdn.fly.dev:8080/api/v1/register"
```

**Response:**
```json
{
  "success": true,
  "message": "Peer registered successfully"
}
```
