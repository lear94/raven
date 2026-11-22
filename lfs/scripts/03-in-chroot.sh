#!/bin/bash
# Runs INSIDE the chroot: LFS chapters 7.5-7.13, 8, 9, 10.3 and 11.1 with Raven.
set -euo pipefail
umask 022
R=/usr/share/raven-lfs
SYSTEM_HOME=/var/lib/raven
TEMP_HOME=/var/lib/raven-lfs-temp
export MAKEFLAGS=${MAKEFLAGS:--j$(nproc)}
say() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }

prepare() {   # prepare <recipe dir> <raven home>
  mkdir -p "$2/recipes" "$2/cache"
  rm -f "$2/recipes"/*.toml && cp "$1"/*.toml "$2/recipes/"
  RAVEN_HOME=$2 raven config --set-repo local-mode >/dev/null
  for f in /sources/*; do
    [ -f "$f" ] || continue
    s=$(sha256sum "$f" | cut -d' ' -f1)
    [ -e "$2/cache/$s" ] || ln "$f" "$2/cache/$s" 2>/dev/null || cp "$f" "$2/cache/$s"
  done
}

# ensure <raven home> <package>: install unless already installed. Raven always
# rebuilds explicit targets, and re-running lfs-filesystem would reset files
# such as /etc/passwd that later packages modify.
ensure() {
  if RAVEN_HOME=$1 raven list 2>/dev/null | grep -q "^$2 "; then
    echo "$2 is already installed"
  else
    RAVEN_HOME=$1 RAVEN_PASSENV=MAKEFLAGS RAVEN_NO_EXTRACT=/usr/share/info/dir raven install "$2"
  fi
}

say "§7.5-7.6 Directories and essential files"
prepare $R/recipes/system $SYSTEM_HOME
ensure $SYSTEM_HOME lfs-filesystem

say "§7.7-7.12 Temporary tools (separate Raven database)"
if [ -d /tools ]; then   # chapter 7 not finished yet (§7.13 removes /tools)
  prepare $R/recipes/temp $TEMP_HOME
  ensure $TEMP_HOME util-linux
fi

if [ -d /tools ]; then
  say "§7.13 Cleaning up the temporary system"
  rm -rf /usr/share/{info,man,doc}/*
  find /usr/{lib,libexec} -name \*.la -delete
  rm -rf /tools
fi

say "§8-§11 Final system"
ensure $SYSTEM_HOME lfs-release

say "§8.86 Cleaning up"
rm -rf /tmp/{*,.*} 2>/dev/null || true
find /usr/lib /usr/libexec -name \*.la -delete
find /usr -depth -name "$(uname -m)-lfs-linux-gnu*" | xargs rm -rf
rm -rf $TEMP_HOME            # chapter 7 tools are all replaced by now
# Rebuild the info index that RAVEN_NO_EXTRACT kept out of the packages.
rm -f /usr/share/info/dir
for f in /usr/share/info/*; do install-info "$f" /usr/share/info/dir 2>/dev/null || true; done

say "Done: $(cat /etc/lfs-release) — $(RAVEN_HOME=$SYSTEM_HOME raven list | grep -c .) packages managed by Raven"
echo "Set a root password before booting:  passwd root"
