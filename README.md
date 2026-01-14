# RAVEN Package Manager
**A High-Performance, Transactional Package Manager for Mission-Critical Systems.**



## 🦅 Overview

**Raven** is a modern, source-based package manager written in **Rust**, designed for Linux From Scratch (LFS) environments and embedded systems where reliability is non-negotiable.

Unlike traditional package managers, Raven builds packages from source inside strictly isolated **Linux Namespaces (Sandboxes)** and uses **SQLite** to enforce **ACID properties** on every installation. It ensures that your system remains consistent, even in the event of failure (power loss, crash, etc.).

## ✨ Key Features

-   **🛡️ Sandboxed Builds:** Compiles software inside a hermetic container using Linux Namespaces (`CLONE_NEWNS`, `unshare`) and private bind mounts. This prevents host system pollution during compilation.
    
-   **⚛️ ACID Transactions:** Uses SQLite to track every file and state change. Installations are atomic; if a transaction fails, the database rolls back, ensuring integrity.
    
-   **🧠 Semantic Versioning (SemVer):** Full support for complex dependency requirements (e.g., `openssl >= 3.0.0`). Raven calculates version compatibility mathematically.
    
-   **🔄 Auto-Upgrade System:** Detects outdated packages by comparing local database versions against remote recipes and rebuilds them automatically via `raven upgrade`.
    
-   **📦 Dependency Guards:** Implements Reverse Dependency protection. You cannot remove a library if an installed application still relies on it.
    
-   **⚙️ Dynamic Configuration:** Easily switch between package repositories (e.g., GitHub, GitLab, or local mirrors) without recompiling.
    
-   **🚀 High Performance:** Async I/O (Tokio), concurrent downloads with retries, and a reactor pattern for dependency graph resolution.
    
-   **💅 Modern UI:** Professional CLI experience with animated progress bars, spinners, and colored output.
    

## 🛠️ Installation

### Prerequisites

-   **OS:** Linux (Requires Kernel support for Namespaces)
    
-   **Dependencies:** `build-essential`, `libssl-dev`, `pkg-config`, `sqlite3`
    
-   **Rust:** Stable toolchain (1.70+)
    

### Building from Source

```
# 1. Clone the repository
git clone https://github.com/lear94/raven.git
cd raven

# 2. Build in release mode
cargo build --release

# 3. Install binary (optional)
sudo cp target/release/raven /usr/local/bin/

```

## 📖 Usage

### Initialization

Raven automatically creates its database at `/var/lib/raven/metadata.db` on the first run.

### Configuration

Set your recipe repository (Git) or use a local path:

```
raven config --set-repo "https://github.com/lear94/raven-recipes.git"

```

### Commands

**1. Search for a package** Fuzzy search allows you to find packages even with typos.

```
raven search hello

```

**2. Install a package** Resolves dependencies (SemVer), downloads source, builds in sandbox, and installs transactionally.

```
sudo raven install nginx

```

**3. Update & Upgrade** Sync recipes from the remote git repo and upgrade the entire system based on version comparison.

```
sudo raven update
sudo raven upgrade

```

**4. Remove a package** Safely removes a package (blocked if other packages depend on it).

```
sudo raven remove vim

```

## 🏗️ Architecture

Raven consists of four main architectural pillars:

1.  **Reactor (The Brain):** Builds a Directed Acyclic Graph (DAG) of dependencies. It validates SemVer constraints (`>=`, `<`) to ensure compatibility before any build starts.
    
2.  **Builder (The Muscle):** Downloads sources with robust retry logic, verifies SHA256 hashes, and orchestrates the compilation process.
    
3.  **Sandbox**d integration test suite that simulates a full system lifecycle (Install, Remove, Upgrade, Crash recovery).
    
    To run the full suite:
    
    ```
    ./run_test.sh
    
    ```
    
    ## 🤝 Contributing
    
    Contributions are welcome! Please feel free to submit a Pull Request.
    
    1.  Fork the Project
        
    2.  Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
        
    3.  Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
        
    4.  Push to the Branch (`git push origin feature/AmazingFeature`)
        
    5.  Open a Pull Request
        
    
    ## 📄 License
    
    Distributed under the MIT License. See `LICENSE` for more information. **(The Shield):** Sets up a `chroot` environment with private `/proc`, `/dev`, and `/sys` mounts. It removes `CLONE_NEWPID` complexity to ensure build scripts (Makefiles) can spawn processes reliably while remaining isolated.
    
4.  **TransactionManager (The Memory):** Records every file path, version, and dependency in SQLite. It handles the "Staging -> Final" file move operation atomically.
    

## 🧪 Testing

Raven includes a robust, Docker-base
