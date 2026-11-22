#!/bin/bash
# LFS §7.2-7.4: ownership, virtual kernel file systems and chroot.
# Usage: 02-chroot-enter.sh [command...]   (default: interactive shell)
source "$(dirname "$0")/common.sh"
need_root

# §7.2 (only what exists: /tools is gone after §7.13, so re-entering works).
for d in usr var etc tools lib64; do
  [ -d "$LFS/$d" ] && [ ! -L "$LFS/$d" ] && chown -R root:root "$LFS/$d"
done

# Raven's sandbox needs "/" to be a mount point inside the chroot. A real LFS
# partition already is one; a plain directory (e.g. in a Docker volume) is not.
mountpoint -q "$LFS" || mount -v --bind "$LFS" "$LFS"

mkdir -pv "$LFS"/{dev,proc,sys,run}
mountpoint -q "$LFS/dev"  || mount -v --bind /dev "$LFS/dev"
mountpoint -q "$LFS/dev/pts" || mount -vt devpts devpts -o gid=5,mode=0620 "$LFS/dev/pts"
mountpoint -q "$LFS/proc" || mount -vt proc proc "$LFS/proc"
mountpoint -q "$LFS/sys"  || mount -vt sysfs sysfs "$LFS/sys"
mountpoint -q "$LFS/run"  || mount -vt tmpfs tmpfs "$LFS/run"
if [ -h "$LFS/dev/shm" ]; then
  install -v -d -m 1777 "$LFS$(realpath /dev/shm)"
elif ! mountpoint -q "$LFS/dev/shm"; then
  mount -vt tmpfs -o nosuid,nodev tmpfs "$LFS/dev/shm"
fi

# Raven itself and the LFS scripts/recipes go into the new system. The binary
# is not tracked by any package; it stays in the final system as its package
# manager.
install -v -m755 "$(command -v raven)" "$LFS/usr/bin/raven"
rm -rf "$LFS/usr/share/raven-lfs" && mkdir -p "$LFS/usr/share/raven-lfs"
cp -r "$LFS_DIR"/{recipes,scripts} "$LFS/usr/share/raven-lfs/"

[ $# -gt 0 ] || set -- /bin/bash --login
chroot "$LFS" /usr/bin/env -i \
    HOME=/root TERM="${TERM:-linux}" \
    PS1='(lfs chroot) \u:\w\$ ' \
    PATH=/usr/bin:/usr/sbin \
    MAKEFLAGS="$MAKEFLAGS" \
    "$@"
