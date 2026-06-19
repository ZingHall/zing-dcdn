FROM rust:slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev clang make protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

COPY . /app
WORKDIR /app

RUN cargo build --release --bin zing-cdn

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/zing-cdn /usr/local/bin/
COPY config.toml /root/.zing-cdn/config.toml

ENV RUST_LOG=info
EXPOSE 34291/udp

CMD ["zing-cdn", "--serve", "--listen", "/ip4/0.0.0.0/udp/34291/quic-v1", "--external-addr", "/ip4/213.188.208.246/udp/34291/quic-v1", "--cache-dir", "/data"]
