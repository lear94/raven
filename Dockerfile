FROM rust:latest

RUN apt-get update && apt-get install -y \
    build-essential \
    libssl-dev \
    pkg-config \
    cmake \
    libsqlite3-dev \
    git \
    curl \
    util-linux \
    qemu-user-static \
    binfmt-support \
    gcc-aarch64-linux-gnu \
    g++-aarch64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml .
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

COPY . .
RUN touch src/main.rs

RUN cargo build --release

ENV PATH="/app/target/release:${PATH}"

CMD ["/bin/bash"]