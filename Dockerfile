# Dockerfile for a the server app

# 1: Build the exe
FROM rust:latest AS builder
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /usr/src

# 1b: Build the executable using the actual source code
WORKDIR /usr/src/throttle
COPY Cargo.toml Cargo.lock ./
COPY server ./server
COPY rust_client ./rust_client

# Build the application
RUN cargo build --release --bin throttle --target x86_64-unknown-linux-musl

# 2: Copy the executable to an empty Docker image
FROM scratch
COPY --from=builder /usr/src/throttle/target/x86_64-unknown-linux-musl/release/throttle .
USER 1000
ENV THROTTLE_LOG="INFO"
ENTRYPOINT ["./throttle", "--address", "0.0.0.0"]
