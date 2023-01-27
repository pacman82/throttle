# Dockerfile for a the server app

# 1: Build the exe
FROM rust:1.67 as builder
WORKDIR /usr/src

# 1a: Prepare for static linking
RUN apt-get update && \
    apt-get dist-upgrade -y && \
    apt-get install -y musl-tools && \
    rustup target add x86_64-unknown-linux-musl

# 1b: Build the executable using the actual source code
WORKDIR /usr/src/throttle
COPY Cargo.toml Cargo.lock ./
COPY server ./server
COPY rust_client ./rust_client
RUN cargo install --target x86_64-unknown-linux-musl --path ./server

# 2: Copy the executable to an empty Docker image
FROM scratch
COPY --from=builder /usr/local/cargo/bin/throttle .
USER 1000
ENV THROTTLE_LOG="INFO"
ENTRYPOINT ["./throttle", "--address", "0.0.0.0"]