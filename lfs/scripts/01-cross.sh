#!/bin/bash
# LFS chapters 4.2, 5 and 6: cross toolchain and temporary tools, built on the
# host with Raven and installed into $LFS.
source "$(dirname "$0")/common.sh"
need_root

say "Preparing $LFS"
mkdir -pv "$LFS/sources"
chmod -v a+wt "$LFS/sources"
# Sources come from lfs/sources (shared with the host when mounted at
# /lfs/sources); missing ones are downloaded. All are md5-verified.
"$LFS_DIR/fetch-sources.sh"
cp -uv "$LFS_DIR"/sources/*.tar.* "$LFS_DIR"/sources/*.patch "$LFS/sources/"

say "Raven (cross) data: $CROSS_HOME"
install_recipes "$LFS_DIR/recipes/cross" "$CROSS_HOME"
seed_cache "$LFS/sources" "$CROSS_HOME"

say "Building chapters 5 and 6"
if RAVEN_HOME=$CROSS_HOME raven list 2>/dev/null | grep -q "^gcc-pass2 "; then
  echo "Cross toolchain already built"
else
RAVEN_HOME=$CROSS_HOME \
RAVEN_PASSENV="LFS LFS_TGT MAKEFLAGS" \
RAVEN_SANDBOX_BINDS=$LFS \
RAVEN_NO_EXTRACT=$LFS/usr/share/info/dir \
  raven install gcc-pass2
fi

say "Cross phase done"
RAVEN_HOME=$CROSS_HOME raven list
