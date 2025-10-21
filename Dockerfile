# 1: Build the exe
FROM rust:latest AS chef
WORKDIR /usr/src/throttle
RUN HOST_TUPLE=$(rustc --print host-tuple); \
    TARGET=$(echo "$HOST_TUPLE" | cut -d'-' -f1); \
    TARGET_TRIPLE="${TARGET}-unknown-linux-musl"; \
    # Save target triple to file, so we can use it in other stages
    echo "$TARGET_TRIPLE" > "/target_triple.txt"; \
    rustup target add "$TARGET_TRIPLE";
# Cargo Chef is used to cache the building of dependencies
RUN cargo install cargo-chef
# Cargo Auditable is used to enrich the executable with metainformation about our build
# dependencies. These can be used to scan for vulnerabilities later on
RUN cargo install cargo-auditable

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY server ./server
COPY rust_client ./rust_client
RUN cargo chef prepare --recipe-path recipe.json --bin throttle

FROM chef AS builder
COPY --from=chef /target_triple.txt target_triple.txt
COPY --from=planner /usr/src/throttle/recipe.json recipe.json
# Build dependencies
RUN cargo chef cook --release --recipe-path recipe.json --bin throttle --target $(cat target_triple.txt)
COPY server/src ./server/src
COPY Cargo.toml Cargo.lock ./
COPY rust_client/Cargo.toml ./rust_client/Cargo.toml
# Build the application
RUN cargo auditable build --release --bin throttle --target $(cat target_triple.txt)

# 2: Copy the executable to an empty Docker image
FROM scratch
COPY --from=builder /usr/src/throttle/target/*/release/throttle .
USER 1000
ENV THROTTLE_LOG="INFO"
ENTRYPOINT ["./throttle", "--address", "0.0.0.0"]
