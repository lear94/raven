# Shared settings for the LFS build scripts (sourced, not executed).
set -euo pipefail
umask 022

# One work directory (a Docker volume) holds the LFS tree, the host-side Raven
# data and, while imaging, the stashed sources: everything survives restarts.
WORK=${WORK:-/work}
export LFS=${LFS:-$WORK/lfs}
export LFS_TGT=$(uname -m)-lfs-linux-gnu
export MAKEFLAGS=${MAKEFLAGS:--j$(nproc)}
LFS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)   # the lfs/ directory
CROSS_HOME=${CROSS_HOME:-$WORK/raven-cross}                # host-side Raven data

say() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }

need_root() { [ "$(id -u)" -eq 0 ] || { echo "run as root" >&2; exit 1; }; }

# seed_cache <sources dir> <raven home>: put every source into Raven's
# content-addressed cache, so builds never download anything.
seed_cache() {
  local src=$1 home=$2 f sum
  mkdir -p "$home/cache"
  for f in "$src"/*; do
    [ -f "$f" ] || continue
    sum=$(sha256sum "$f" | cut -d' ' -f1)
    [ -e "$home/cache/$sum" ] || ln "$f" "$home/cache/$sum" 2>/dev/null || cp "$f" "$home/cache/$sum"
  done
}

# install_recipes <recipe dir> <raven home>: local-mode repository with the recipes.
install_recipes() {
  local dir=$1 home=$2
  mkdir -p "$home/recipes"
  rm -f "$home/recipes"/*.toml
  cp "$dir"/*.toml "$home/recipes/"
  RAVEN_HOME=$home raven config --set-repo local-mode >/dev/null
}
