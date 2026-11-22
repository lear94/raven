# RAVEN Package Manager
**A transactional, sandboxed, source-based package manager for mission-critical systems.**

## Overview

**Raven** is a source-based package manager written in **Rust**, designed for Linux From Scratch (LFS) environments and embedded systems where reliability is non-negotiable.

Raven builds packages from source inside isolated **Linux namespaces** and records every change in **SQLite**, so a failed build, a file conflict or an interrupted installation never leaves the system half-modified.

## Key Features

- **Sandboxed builds**: each build runs in a private mount/UTS/IPC namespace inside a `chroot`. Host tool directories (`/usr`, `/bin`, `/lib`, `/etc`, ...) are bind-mounted **read-only**, the environment is cleared, and the build can only write to `/src`, `/tmp` and `/out` (`DESTDIR`).
- **Transactional installs**: files are staged next to their destination and swapped in with atomic `rename`s; the database is committed last. Any error rolls back every file that was replaced. Ctrl-C during this step is deferred until the transaction is complete.
- **File conflict detection**: a package can't overwrite files owned by another package.
- **Clean upgrades**: files that the new version no longer ships are removed; directories created by a package are removed with it.
- **Semantic versioning**: requirements such as `openssl >= 3.0.0` or `zlib >=1.2, <2.0`. Upstream versions like `2.10` are accepted (as `2.10.0`).
- **Smart resolution**: dependency graph with cycle detection; already-installed dependencies that satisfy the requirement are not rebuilt; upgrades that would break an installed dependent are refused.
- **Dependency guards**: a package can't be removed while another installed package depends on it.
- **Verified, cached downloads**: SHA-256 is verified for every source; sources are cached by checksum, so rebuilds don't download again. Downloads retry with backoff. `.tar`, `.tar.gz`, `.tar.xz` and `.tar.bz2` are supported, as well as local sources (`file:///path` or `/path`).
- **Strict builds**: build scripts run with `set -e`, so any failing command aborts the build; the tail of the build log is shown on failure.
- **Safe concurrency**: a global lock prevents two Raven processes from modifying the system at once.
- **Robust recipe loading**: an invalid recipe is reported and skipped instead of breaking the whole repository.

## Installation

### Requirements

- **OS:** Linux (kernel with namespace support). Building packages requires root.
- **Tools:** `git` (for `raven update`), and whatever toolchain your recipes need (e.g. `build-essential`).
- **Rust:** recent stable toolchain to build Raven itself. No OpenSSL or system SQLite needed (TLS is pure Rust and SQLite is bundled).

### Building from source

```sh
git clone https://github.com/lear94/raven.git
cd raven
cargo build --release
sudo install -m 755 target/release/raven /usr/local/bin/raven
```

## Usage

Raven stores its data in `/var/lib/raven` (override with the `RAVEN_HOME` environment variable):

| Path | Purpose |
| --- | --- |
| `metadata.db` | Installed packages, files and dependencies |
| `config.toml` | Configuration |
| `recipes/` | Recipe repository |
| `cache/` | Verified source archives (by SHA-256) |
| `build/` | Build roots (kept on failure for debugging, including `build.log`) |

### Configuration

```sh
raven config --set-repo "https://github.com/lear94/raven-recipes.git"   # git repository
raven config --set-repo local-mode                                      # manage recipes/ by hand
raven config                                                            # show configuration
```

### Commands

```sh
raven search hello            # fuzzy search by name and description
sudo raven install nginx      # resolve, download, build in sandbox, install transactionally
sudo raven update             # sync recipes from the git repository
sudo raven upgrade            # rebuild packages that have a newer recipe version
sudo raven remove vim         # remove (blocked if other packages depend on it)
raven list                    # list installed packages
```

`remove` accepts several packages and removes them in dependency order (`raven remove app libfoo`).

## Writing recipes

A recipe is a TOML file in the recipes directory:

```toml
name = "hello"
version = "2.10"
description = "GNU Hello"
dependencies = ["libdummy >= 1.0.0"]          # optional
source_url = "https://mirrors.kernel.org/gnu/hello/hello-2.10.tar.gz"
sha256_sum = "31e066137a962676e89f69d1b65382de95a7ef7d914b8cb956f41ea72e0f516b"
build_commands = ["./configure --prefix=/usr", "make -j$RAVEN_JOBS"]
install_commands = ["make install"]
# target_arch = "aarch64"                     # optional, for cross builds
```

Build environment:

- If the archive contains a single top-level directory, commands run inside it; otherwise in `/src`.
- `DESTDIR=/out` is exported: install everything under `/out`. Writing to host directories fails (they are read-only).
- Variables: `RAVEN_PKG_NAME`, `RAVEN_PKG_VERSION`, `RAVEN_JOBS` (CPU count), `PATH`, `HOME=/tmp`, `LC_ALL=C`.
- Requirement syntax follows SemVer: `>= 1.0`, `< 2`, `= 1.2.3`, `~1.2`; a bare version (`1.2.0`) means compatible (`^1.2.0`).
- `target_arch`: omit it (or use `any`) for native builds. `aarch64` on an x86_64 host (and vice versa) exports `CC`, `CXX`, `AR` and `CROSS_COMPILE` for the matching `*-linux-gnu` cross toolchain.

## Architecture

1. **Reactor** (`reactor.rs`): resolves the dependency DAG, validates SemVer constraints in both directions, and drives build and install.
2. **Builder** (`builder.rs`): downloads and verifies sources (cache, retries), extracts them and generates the build script.
3. **Sandbox** (`sandbox.rs`): namespaces, read-only bind mounts, private `/proc`, `chroot`.
4. **TransactionManager** (`transaction.rs`): staging, atomic swap, rollback and SQLite bookkeeping (WAL, `synchronous=FULL`, schema migrations).

## Testing

```sh
cargo test          # unit tests (any Unix, no root needed)
./run_test.sh       # full lifecycle in a privileged Docker container
```

The integration suite covers configuration, dependency chains, database integrity, SemVer conflicts, failed builds, sandbox isolation, file conflicts, upgrades and removal.

## Contributing

Contributions are welcome:

1. Fork the project
2. Create your feature branch (`git checkout -b feature/AmazingFeature`)
3. Make sure `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test` pass
4. Open a pull request

## License

Distributed under the MIT License. See `LICENSE` for more information.
