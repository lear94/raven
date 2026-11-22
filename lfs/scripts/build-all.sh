#!/bin/bash
# Full unattended build: cross phase, chroot phase, disk image.
# Re-running resumes: Raven skips packages that are already installed.
source "$(dirname "$0")/common.sh"
need_root
S=$(dirname "$0")
"$S/01-cross.sh"
"$S/02-chroot-enter.sh" /bin/bash /usr/share/raven-lfs/scripts/03-in-chroot.sh
"$S/04-image.sh" "${OUTPUT:-/output}"
