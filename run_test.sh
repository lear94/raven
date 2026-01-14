#!/bin/bash
set -e

# --- STYLING ---
BOLD='\033[1m'
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m' # No Color

# --- LOGGING FUNCTIONS ---
header() {
    echo -e "\n${BLUE}${BOLD}:: ${1} ${NC}"
}

echo -e "${RED}${BOLD}   RAVEN INTEGRATION TEST SUITE v3.1 ${NC}"
echo -e "${GRAY}   Validating: Core, Reactor, DB, SemVer, Upgrade${NC}"

# 1. CLEANUP
header "Step 1: Cleaning previous environment"
docker rm -f raven-test 2>/dev/null || true

# 2. BUILD
header "Step 2: Building Docker Image (Compiler Environment)"
docker build -t raven-env . > /dev/null 2>&1
echo -e "${GRAY}   -> Docker image built successfully.${NC}"

# 3. RUN TESTS
header "Step 3: Booting Container & Injecting Test Script"
docker run --privileged --name raven-test -i raven-env bash <<EOF
set -e

# --- INTERNAL CONTAINER STYLING ---
BOLD='\033[1m'
GREEN='\033[0;32m'
RED='\033[0;31m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
NC='\033[0m'

function pass() { echo -e "   \${GREEN}✔ PASS:\${NC} \$1"; }
function fail() { echo -e "   \${RED}✖ FAIL:\${NC} \$1"; exit 1; }
function info() { echo -e "\n\${CYAN}\${BOLD}➜ \${1}\${NC}"; }

# ==========================================================
# TEST PHASE 1: CONFIGURATION SYSTEM
# ==========================================================
info "PHASE 1: Configuration System"

mkdir -p /var/lib/raven/recipes

# Test Config Command persistence
raven config --set-repo "https://gitlab.com/test/repo.git"
if raven config --show | grep -q "gitlab.com"; then
    pass "Configuration updated and persisted correctly"
else
    fail "Config command failed"
fi

# Reset to local mode for following tests
raven config --set-repo "local-mode"

# ==========================================================
# TEST PHASE 2: DEPENDENCY CHAIN & INSTALL (User's Logic)
# ==========================================================
info "PHASE 2: Reactor & Installation (Complex Chain)"

# RECIPE A: libdummy (A fake library dependency)
# FIX: Cambiado de "1.0" a "1.0.0" para cumplir con SemVer estricto de Rust
cat > /var/lib/raven/recipes/libdummy.toml <<RECIPE
name = "libdummy"
version = "1.0.0"
description = "A dummy library dependency"
target_arch = "x86_64"
dependencies = []
source_url = "https://mirrors.kernel.org/gnu/hello/hello-2.10.tar.gz"
sha256_sum = "31e066137a962676e89f69d1b65382de95a7ef7d914b8cb956f41ea72e0f516b"
build_commands = ["echo 'Simulating lib build...'"]
install_commands = [
    "mkdir -p /out/usr/lib",
    "touch /out/usr/lib/libdummy.so"
]
RECIPE
echo "   -> Recipe created: libdummy.toml"

# RECIPE B: hello-app (Depends on libdummy)
# FIX: Cambiado de "2.10" a "2.10.0" para cumplir con SemVer estricto de Rust
cat > /var/lib/raven/recipes/hello.toml <<RECIPE
name = "hello"
version = "2.10.0"
description = "GNU Hello App"
target_arch = "x86_64"
dependencies = ["libdummy"]
source_url = "https://mirrors.kernel.org/gnu/hello/hello-2.10.tar.gz"
sha256_sum = "31e066137a962676e89f69d1b65382de95a7ef7d914b8cb956f41ea72e0f516b"
build_commands = [
    "./configure --prefix=/usr",
    "make -j\$(nproc)"
]
install_commands = [
    "make install"
]
RECIPE
echo "   -> Recipe created: hello.toml (Depends on libdummy)"

# TEST SEARCH
if raven search dummy | grep -q "libdummy"; then
    pass "Fuzzy search found 'libdummy'"
else
    fail "Search engine failed to find local recipe"
fi

# TEST INSTALL
echo -e "\${YELLOW}   Attempting to install 'hello'. Reactor MUST install 'libdummy' first.\${NC}"
raven install hello

# VALIDATION
if [ -f "/usr/lib/libdummy.so" ]; then
    pass "Dependency 'libdummy' installed (File found)"
else
    fail "Dependency 'libdummy' was NOT installed."
fi

if command -v hello &> /dev/null; then
    pass "Target 'hello' installed (Binary found)"
else
    fail "Target 'hello' binary missing."
fi

# ==========================================================
# TEST PHASE 3: DATABASE INTEGRITY & GUARDS (User's Logic)
# ==========================================================
info "PHASE 3: DB Integrity & Safety Guards"

# 1. DB Count Check
if command -v sqlite3 &> /dev/null; then
    COUNT=\$(sqlite3 /var/lib/raven/metadata.db "SELECT count(*) FROM packages;")
    # We expect 2: hello and libdummy
    if [ "\$COUNT" -eq "2" ]; then
        pass "Database registered exactly 2 packages"
    else
        fail "Database Integrity Error: Expected 2 packages, found \$COUNT"
    fi
else
    echo "   (Skipping explicit SQL check, sqlite3 tool not in container)"
fi

# 2. Safety Guard (Reverse Dependencies)
echo -e "\${YELLOW}   Attempting to remove 'libdummy' while 'hello' still needs it...\${NC}"
if raven remove libdummy 2>/dev/null; then
    fail "CRITICAL: Raven allowed removing a dependency that is in use!"
else
    pass "Raven correctly blocked removal (Dependency Guard active)"
fi

# ==========================================================
# TEST PHASE 4: SEMANTIC VERSIONING LOGIC (New Feature)
# ==========================================================
info "PHASE 4: Semantic Versioning (Math Check)"

# Create App requiring NEW Library (>= 2.0.0) but we only have 1.0.0
cat > /var/lib/raven/recipes/app_strict.toml <<RECIPE
name = "app_strict"
version = "1.0.0"
description = "Strict App"
target_arch = "x86_64"
dependencies = ["libdummy >= 2.0.0"]
source_url = "https://mirrors.kernel.org/gnu/hello/hello-2.10.tar.gz"
sha256_sum = "31e066137a962676e89f69d1b65382de95a7ef7d914b8cb956f41ea72e0f516b"
build_commands = ["echo 'Building app...'"]
install_commands = ["make install"]
RECIPE

echo -e "\${YELLOW}   [TEST] Installing 'app_strict' (Requires libdummy >= 2.0.0). Current is 1.0.0.\${NC}"
if raven install app_strict 2>&1 | grep -q "Version mismatch"; then
    pass "Raven BLOCKED installation due to version mismatch."
else
    fail "Raven allowed invalid version installation!"
fi

# ==========================================================
# TEST PHASE 5: SYSTEM UPGRADE (New Feature)
# ==========================================================
info "PHASE 5: Validating 'raven upgrade'"

# 1. Update libdummy recipe to 2.0.0 (Simulate git pull)
cat > /var/lib/raven/recipes/libdummy.toml <<RECIPE
name = "libdummy"
version = "2.0.0"
description = "A dummy library v2"
target_arch = "x86_64"
dependencies = []
source_url = "https://mirrors.kernel.org/gnu/hello/hello-2.10.tar.gz"
sha256_sum = "31e066137a962676e89f69d1b65382de95a7ef7d914b8cb956f41ea72e0f516b"
build_commands = ["echo 'Upgrading lib...'"]
install_commands = [
    "mkdir -p /out/usr/lib",
    "touch /out/usr/lib/libdummy_v2.so"
]
RECIPE

echo -e "\${YELLOW}   Running 'raven upgrade' (Should detect libdummy v1.0.0 -> v2.0.0)...\${NC}"
raven upgrade

if [ -f "/usr/lib/libdummy_v2.so" ]; then
    pass "Upgrade successful: New artifact found"
else
    fail "Upgrade failed: Artifact missing"
fi

# ==========================================================
# TEST PHASE 6: CLEAN REMOVAL (User's Logic)
# ==========================================================
info "PHASE 6: Clean Removal Sequence"

# 1. Remove the app
raven remove hello
if ! command -v hello &> /dev/null; then
    pass "Package 'hello' removed successfully"
else
    fail "Package 'hello' still exists after removal"
fi

# 2. Remove the lib (Should work now as hello is gone)
raven remove libdummy
if [ ! -f "/usr/lib/libdummy.so" ] && [ ! -f "/usr/lib/libdummy_v2.so" ]; then
    pass "Dependency 'libdummy' removed successfully"
else
    fail "Dependency file still exists on disk"
fi

echo -e "\n\${GREEN}\${BOLD}✨ ALL SYSTEMS NOMINAL. RAVEN IS READY.\${NC}"
EOF

header "Test Suite Completed Successfully"