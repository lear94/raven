#!/usr/bin/env bash
# Raven integration test suite: runs the full package lifecycle inside a
# privileged Docker container (namespaces + chroot need CAP_SYS_ADMIN).
set -euo pipefail

BOLD='\033[1m'
RED='\033[0;31m'
BLUE='\033[0;34m'
GRAY='\033[0;90m'
NC='\033[0m'

header() { echo -e "\n${BLUE}${BOLD}:: ${1} ${NC}"; }

IMAGE="raven-env"
CONTAINER="raven-test"

echo -e "${RED}${BOLD}   RAVEN INTEGRATION TEST SUITE v4.0 ${NC}"
echo -e "${GRAY}   Validating: Config, Reactor, DB, SemVer, Sandbox, Conflicts, Upgrade, Removal${NC}"

command -v docker >/dev/null || { echo "docker is required"; exit 1; }

header "Step 1: Cleaning previous environment"
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true

header "Step 2: Building Docker image"
BUILD_LOG=$(mktemp)
trap 'rm -f "$BUILD_LOG"' EXIT
if ! docker build -t "$IMAGE" . >"$BUILD_LOG" 2>&1; then
    cat "$BUILD_LOG"
    echo -e "${RED}Docker build failed${NC}"
    exit 1
fi
echo -e "${GRAY}   -> Docker image built successfully.${NC}"

header "Step 3: Running test scenarios"
docker run --rm --privileged --name "$CONTAINER" -i "$IMAGE" bash -s <<'EOF'
set -euo pipefail

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; YELLOW='\033[0;33m'; BOLD='\033[1m'; NC='\033[0m'
pass() { echo -e "   ${GREEN}✔ PASS:${NC} $1"; }
fail() { echo -e "   ${RED}✖ FAIL:${NC} $1"; exit 1; }
info() { echo -e "\n${CYAN}${BOLD}➜ $1${NC}"; }
note() { echo -e "${YELLOW}   $1${NC}"; }

RECIPES=/var/lib/raven/recipes
DB=/var/lib/raven/metadata.db
HELLO_URL="https://mirrors.kernel.org/gnu/hello/hello-2.10.tar.gz"
HELLO_SHA="31e066137a962676e89f69d1b65382de95a7ef7d914b8cb956f41ea72e0f516b"

# Writes a recipe: recipe <name> <version> <deps-toml-array> <build-array> <install-array>
recipe() {
    cat > "$RECIPES/$1.toml" <<RECIPE
name = "$1"
version = "$2"
description = "Test package $1"
dependencies = $3
source_url = "$HELLO_URL"
sha256_sum = "$HELLO_SHA"
build_commands = $4
install_commands = $5
RECIPE
}

pkg_count() { sqlite3 "$DB" "SELECT count(*) FROM packages;"; }

# ==========================================================
info "PHASE 1: Configuration"
raven --version >/dev/null || fail "binary not runnable"
raven config --set-repo "https://gitlab.com/test/repo.git"
raven config --show | grep -q "gitlab.com" && pass "Configuration persisted" || fail "Config not persisted"
if raven config --set-repo "bad url" 2>/dev/null; then fail "Invalid repo URL accepted"; else pass "Invalid repo URL rejected"; fi
raven config --set-repo "local-mode"
raven update >/dev/null && pass "'update' in local mode is a no-op" || fail "'update' failed in local mode"

# ==========================================================
info "PHASE 2: Reactor & installation (dependency chain)"
mkdir -p "$RECIPES"
recipe libdummy "1.0.0" '[]' '["echo building lib"]' '["mkdir -p /out/usr/lib", "touch /out/usr/lib/libdummy.so"]'
recipe hello "2.10" '["libdummy"]' '["./configure --prefix=/usr", "make -j$(nproc)"]' '["make install"]'
echo 'name = ' > "$RECIPES/broken-syntax.toml"

raven search dummy 2>/dev/null | grep -q "libdummy" && pass "Fuzzy search found 'libdummy'" || fail "Search failed"
raven search dummy 2>&1 | grep -q "Skipping recipe broken-syntax.toml" \
    && pass "Broken recipe reported without blocking others" || fail "Broken recipe not reported"
rm "$RECIPES/broken-syntax.toml"

note "Installing 'hello'; the reactor must install 'libdummy' first."
raven install hello
[ -f /usr/lib/libdummy.so ] && pass "Dependency 'libdummy' installed" || fail "Dependency not installed"
command -v hello >/dev/null && pass "Target 'hello' installed" || fail "'hello' binary missing"
[ "$(hello)" = "Hello, world!" ] && pass "'hello' runs correctly" || fail "'hello' output wrong"

# ==========================================================
info "PHASE 3: Database integrity & guards"
[ "$(pkg_count)" -eq 2 ] && pass "Database registered exactly 2 packages" || fail "Expected 2 packages, found $(pkg_count)"
raven list | grep -q "hello 2.10" && pass "'raven list' shows installed packages" || fail "'raven list' wrong"
if raven remove libdummy 2>/dev/null; then fail "Removed a dependency still in use"; else pass "Dependency guard blocked removal"; fi
if raven remove not-installed 2>/dev/null; then fail "Removing an unknown package succeeded"; else pass "Unknown package removal rejected"; fi

# ==========================================================
info "PHASE 4: Semantic versioning"
recipe app_strict "1.0.0" '["libdummy >= 2.0.0"]' '["true"]' '["mkdir -p /out/usr/bin", "touch /out/usr/bin/app_strict"]'
if out=$(raven install app_strict 2>&1); then fail "Invalid version installation allowed"; fi
echo "$out" | grep -q "Version mismatch" && pass "Version mismatch blocked installation" || fail "Unexpected error: $out"
rm "$RECIPES/app_strict.toml"

# ==========================================================
info "PHASE 5: Build failure detection (set -e)"
recipe broken "1.0.0" '[]' '["false", "echo must-not-run"]' '["mkdir -p /out/usr/bin", "touch /out/usr/bin/broken"]'
if out=$(raven install broken 2>&1); then fail "Failed build was installed"; fi
echo "$out" | grep -q "Build of 'broken' failed" && pass "Failing build command aborted the build" || fail "Unexpected error: $out"
[ ! -e /usr/bin/broken ] && [ "$(pkg_count)" -eq 2 ] && pass "Nothing installed after failed build" || fail "Partial install after failure"
rm "$RECIPES/broken.toml"

# ==========================================================
info "PHASE 6: Sandbox isolation (read-only host)"
recipe escape "1.0.0" '[]' '["touch /usr/raven_escape_test"]' '["mkdir -p /out/usr/bin", "touch /out/usr/bin/escape"]'
if raven install escape >/dev/null 2>&1; then fail "Build wrote outside DESTDIR"; fi
[ ! -e /usr/raven_escape_test ] && pass "Build could not write into host /usr" || fail "Sandbox escape: host /usr modified"
rm "$RECIPES/escape.toml"

# ==========================================================
info "PHASE 7: File conflict detection"
recipe clash "1.0.0" '[]' '["true"]' '["mkdir -p /out/usr/lib", "echo evil > /out/usr/lib/libdummy.so"]'
if out=$(raven install clash 2>&1); then fail "Conflicting package installed"; fi
echo "$out" | grep -q "File conflict" && pass "Conflict with 'libdummy' detected" || fail "Unexpected error: $out"
[ ! -s /usr/lib/libdummy.so ] && pass "Owned file left untouched" || fail "Owned file was overwritten"
rm "$RECIPES/clash.toml"

# ==========================================================
info "PHASE 8: System upgrade"
recipe libdummy "2.0.0" '[]' '["echo upgrading lib"]' '["mkdir -p /out/usr/lib", "touch /out/usr/lib/libdummy_v2.so"]'
raven upgrade
[ -f /usr/lib/libdummy_v2.so ] && pass "New version artifact installed" || fail "Upgrade artifact missing"
[ ! -e /usr/lib/libdummy.so ] && pass "Stale file from v1 removed" || fail "Stale v1 file left behind"
raven list | grep -q "libdummy 2.0.0" && pass "Database records v2.0.0" || fail "Database version not updated"
raven upgrade | grep -q "up to date" && pass "Second upgrade is a no-op" || fail "Second upgrade not idempotent"

# ==========================================================
info "PHASE 9: Removal (multiple packages, dependency order)"
raven remove libdummy hello
command -v hello >/dev/null && fail "'hello' still present" || pass "'hello' removed"
[ ! -e /usr/lib/libdummy_v2.so ] && pass "'libdummy' removed" || fail "'libdummy' files remain"
[ "$(pkg_count)" -eq 0 ] && pass "Database is empty" || fail "Database still has packages"

echo -e "\n${GREEN}${BOLD}✨ ALL SYSTEMS NOMINAL. RAVEN IS READY.${NC}"
EOF

header "Test Suite Completed Successfully"
