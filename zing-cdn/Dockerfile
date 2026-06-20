FROM rust:slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev clang make protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# --- Layer 1: Cache dependencies (rebuilds only when Cargo.toml/Cargo.lock change) ---
COPY Cargo.toml Cargo.lock ./
COPY zing-cdn-core/Cargo.toml zing-cdn-core/
COPY zing-cdn/Cargo.toml zing-cdn/

# Dummy GUI Cargo.toml (workspace member, not built)
RUN mkdir -p zing-cdn-gui/src-tauri/src && \
    echo '[package]' > zing-cdn-gui/src-tauri/Cargo.toml && \
    echo 'name = "dummy-gui"' >> zing-cdn-gui/src-tauri/Cargo.toml && \
    echo 'version = "0.0.0"' >> zing-cdn-gui/src-tauri/Cargo.toml && \
    echo 'edition = "2021"' >> zing-cdn-gui/src-tauri/Cargo.toml && \
    echo 'fn main() {}' > zing-cdn-gui/src-tauri/src/main.rs

# Dummy source files so cargo can resolve dependencies
RUN mkdir -p zing-cdn-core/src zing-cdn/src && \
    echo '#![allow(unused)]' > zing-cdn-core/src/lib.rs && \
    echo 'fn main() {}' > zing-cdn/src/main.rs

# Remove GUI from workspace — not needed for CLI build
RUN sed -i '/zing-cdn-gui/d' Cargo.toml

# Build dependencies (cached between deploys — only rebuilds when Cargo.toml changes)
RUN cargo build --release --bin zing-cdn || echo "Dep build complete (link errors expected with dummy src)"

# --- Layer 2: Real source (rebuilds on any code change) ---
COPY . /app
RUN sed -i '/zing-cdn-gui/d' Cargo.toml
RUN cargo build --release --bin zing-cdn

# --- Runtime ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/zing-cdn /usr/local/bin/
COPY config.toml /root/.zing-cdn/config.toml

ENV RUST_LOG=info
EXPOSE 34291/udp

CMD ["zing-cdn", "--serve", "--listen", "/ip4/0.0.0.0/udp/34291/quic-v1", "--external-addr", "/ip4/213.188.208.246/udp/34291/quic-v1", "--cache-dir", "/data"]
