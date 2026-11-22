FROM rust:1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        git \
        curl \
        ca-certificates \
        sqlite3 \
        util-linux \
    && rm -rf /var/lib/apt/lists/*

# Optional cross-compilation toolchain (not available for every host architecture).
RUN apt-get update \
    && (apt-get install -y --no-install-recommends \
            qemu-user-static binfmt-support gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
        || echo "Cross toolchain not available on this architecture, skipping") \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies in their own layer.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY . .
RUN touch src/main.rs && cargo build --release --locked

ENV PATH="/app/target/release:${PATH}"

CMD ["/bin/bash"]
