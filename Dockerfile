# Dockerfile for a the server app

# 1: Build the exe
FROM rust:latest AS chef
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /usr/src/throttle
RUN cargo install cargo-chef

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY server ./server
COPY rust_client ./rust_client
RUN cargo chef prepare --recipe-path recipe.json --bin throttle

FROM chef AS builder
COPY --from=planner /usr/src/throttle/recipe.json recipe.json
# Build dependencies
RUN cargo chef cook --release --recipe-path recipe.json --bin throttle --target x86_64-unknown-linux-musl
COPY server/src ./server/src
COPY Cargo.toml Cargo.lock ./
COPY rust_client/Cargo.toml ./rust_client/Cargo.toml
# Build the application
RUN cargo build --release --bin throttle --target x86_64-unknown-linux-musl

# 2: Copy the executable to an empty Docker image
FROM scratch
COPY --from=builder /usr/src/throttle/target/x86_64-unknown-linux-musl/release/throttle .
USER 1000
ENV THROTTLE_LOG="INFO"
ENTRYPOINT ["./throttle", "--address", "0.0.0.0"]
