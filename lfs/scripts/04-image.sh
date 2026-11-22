#!/bin/bash
# Leaves the chroot environment and turns $LFS into a bootable ext4 disk image
# (label "lfsroot", no partition table) plus the kernel for direct booting.
# Usage: 04-image.sh [output dir] [size]   (defaults: /output, 12G)
source "$(dirname "$0")/common.sh"
need_root
OUT=${1:-/output}
SIZE=${2:-12G}

say "Unmounting virtual file systems"
for m in dev/shm dev/pts sys proc run dev; do
  mountpoint -q "$LFS/$m" && umount "$LFS/$m"
done

say "Creating $OUT/lfs-13.1.img ($SIZE, ext4, label lfsroot)"
mkdir -p "$OUT"
# Keep sources and Raven's source cache out of the image (moved, not copied).
mv "$LFS/sources" "$WORK/sources.stash"
[ -d "$LFS/var/lib/raven/cache" ] && mv "$LFS/var/lib/raven/cache" "$WORK/cache.stash"
trap 'mv "$WORK/sources.stash" "$LFS/sources"
      [ -d "$WORK/cache.stash" ] && mv "$WORK/cache.stash" "$LFS/var/lib/raven/cache"' EXIT
mkdir -p "$LFS/sources"
rm -f "$OUT/lfs-13.1.img"
mke2fs -q -t ext4 -L lfsroot -d "$LFS" "$OUT/lfs-13.1.img" "$SIZE"
rmdir "$LFS/sources"

cp -v "$LFS"/boot/vmlinuz-* "$OUT/"
say "Image ready"
ls -lh "$OUT"
