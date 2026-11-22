# Build host for LFS 13.1-systemd with Raven.
# Build from the repository root:  docker build -f lfs/Dockerfile -t raven-lfs .

FROM rust:1-trixie AS raven
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:trixie
# LFS §2.2 host requirements (+ tools used by the driver scripts).
RUN apt-get update && apt-get install -y --no-install-recommends \
        bash binutils bison coreutils diffutils findutils gawk gcc g++ grep \
        gzip m4 make patch perl python3 sed tar texinfo xz-utils \
        e2fsprogs util-linux ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    # LFS requires /bin/sh -> bash (Raven runs build scripts with /bin/sh).
    && ln -sf bash /bin/sh \
    && [ ! -e /etc/bash.bashrc ] || mv /etc/bash.bashrc /etc/bash.bashrc.NOUSE
COPY --from=raven /app/target/release/raven /usr/local/bin/raven
COPY lfs /lfs
CMD ["/lfs/scripts/build-all.sh"]
