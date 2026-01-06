# cargo-chef and the Rust toolchain
FROM lukemathwalker/cargo-chef:latest-rust-1.86.0 AS chef
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY crates/ crates/
COPY xtask/ xtask/
RUN cargo chef prepare --recipe-path recipe.json --bin ralph

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY crates/ crates/
COPY xtask/ xtask/

RUN cargo build --release --bin ralph
# We do not need the Rust toolchain to run the binary!
FROM debian:bookworm-slim AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/ralph /usr/local/bin
ENTRYPOINT ["/usr/local/bin/ralph"]
