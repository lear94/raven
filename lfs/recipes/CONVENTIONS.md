# Raven recipe conventions for LFS 13.1-systemd

These recipes build Linux From Scratch 13.1 (systemd) with Raven. The commands
follow the book (`lfs/book/13.1`, text dumps in `lfs/book/txt`, command-only
dumps in `lfs/book/cmds`). Where a recipe differs from the book, a `#` comment
inside the command string explains why.

## Recipe sets

| Directory | Runs | `RAVEN_HOME` | Book chapters |
| --- | --- | --- | --- |
| `cross/` | on the host, as root | `/var/lib/raven-lfs-cross` (host) | 4.2, 5, 6 |
| `temp/` | inside the chroot | `/var/lib/raven-lfs-temp` | 7.7 – 7.12 |
| `system/` | inside the chroot, then on the booted system | `/var/lib/raven` | 7.5, 7.6, 8, 9, 10 |

`temp/` uses its own database on purpose: the chapter 7 tools are overwritten
by chapter 8, and Raven refuses to overwrite files owned by another package in
the *same* database. Files of untracked packages are simply replaced.

## Recipe format

```toml
name = "binutils"                       # book package name, lowercase
version = "2.47"                        # upstream version (see "Versions")
description = "LFS 13.1 §8.21 - GNU binary utilities"
dependencies = ["previous-package"]     # the package right before it in the book
source_url = "<exact URL from lfs/sources/wget-list>"
sha256_sum = "FILL"                     # filled by lfs/tools/fill-checksums.py
extra_sources = [                       # patches and bundled tarballs, unextracted
  { url = "<exact URL from wget-list>", sha256_sum = "FILL" },
]
build_commands = [ ... ]
install_commands = [ ... ]
post_install_commands = [ ... ]         # optional, see below
overwrites = ["libstdc++"]              # optional: packages it may replace files of
```

`overwrites` is for the places where the book deliberately reinstalls files
of an earlier package in the same set (e.g. `gcc-pass2` rebuilds the chapter 5
`libstdc++`). Any other file conflict is an error.

* **Order**: every recipe depends on the recipe that comes right before it in
  the book, so `raven install <last-package>` builds the chapter in book order.
  Each set's README lists the chain.
* **Versions** must parse as SemVer after padding (`2.47` → `2.47.0`). Use the
  real version when it fits (`1.26`, `5.48`, `6.6`, `2.10.0`). Otherwise
  translate: `expect5.45.4` → `5.45.4`, `tzdata2026c` → (not a recipe),
  `iana-etc-20260805` → `20260805.0.0`, `sqlite-autoconf-3530400` → `3.53.4`,
  `less-704` → `704.0.0`, `vim-9.2.1025` → `9.2.1025`. Never invent pre-release
  suffixes.
* **Commands**: all `build_commands`, then all `install_commands`, are joined
  with newlines into ONE script run by `/bin/sh` (bash in both the host image
  and the chroot) with `set -e` and `set -x`. `cd`, variables and functions
  persist between entries. Use TOML multi-line literal strings (`'''`) for
  heredocs and long `configure` lines. Bashisms (brace expansion, `[[ ]]`) are
  fine.
* The script starts in the single top-level directory of the source archive.
  `extra_sources` are available unextracted in `$RAVEN_SOURCES` (= `/sources`),
  so the book's `../foo.patch` becomes `$RAVEN_SOURCES/foo.patch` and
  `tar -xf ../mpfr-4.2.2.tar.xz` becomes `tar -xf $RAVEN_SOURCES/mpfr-4.2.2.tar.xz`.
* Packages without a tarball (directory layout, config files) omit
  `source_url` and `sha256_sum`; the script then starts in an empty `/src`.

## The sandbox, and what that means for the book's commands

The build runs in a chroot where the live system's `/usr`, `/etc`, ... are
**read-only**. Only `/src`, `/tmp`, `/sources` and `/out` are writable, and
`DESTDIR=/out` is exported. After a successful build Raven copies `/out` onto
`/` transactionally. Therefore:

1. Every install goes to `$DESTDIR`: `make DESTDIR=$DESTDIR install`
   (`make install` also works for automake, since `DESTDIR` is exported, but be
   explicit). Packages that don't support `DESTDIR` use their own variable
   (`make PREFIX=$DESTDIR/usr install` for bzip2, `--root=$DESTDIR` for
   `pip3 install`, `meson install --destdir $DESTDIR` / `ninja install` with the
   exported `DESTDIR`, etc.). Check the package's Makefile in
   `lfs/sources/<tarball>` when in doubt.
2. Every command that touches installed files is prefixed with `$DESTDIR`:
   `mv -v /usr/bin/chroot /usr/sbin` → `mv -v $DESTDIR/usr/bin/chroot $DESTDIR/usr/sbin`;
   `ln -sv gcc.1 /usr/share/man/man1/cc.1` →
   `ln -sv gcc.1 $DESTDIR/usr/share/man/man1/cc.1`. Create parent directories
   with `install -d`/`mkdir -p` when the target dir might not exist in `/out`.
   Relative symlink targets stay as in the book (`ln -sv bash $DESTDIR/usr/bin/sh`);
   absolute symlink targets also stay unprefixed (`ln -sv /proc/self/mounts $DESTDIR/etc/mtab`).
   `ln -svr` (relative) must be computed inside `$DESTDIR`: `ln -svr $DESTDIR/usr/bin/cpp $DESTDIR/usr/lib/`.
3. **Never install into `/bin`, `/lib`, `/sbin` or `/lib64` directly** – use
   `/usr/bin`, `/usr/lib`, `/usr/sbin` (they are symlinks on the target). The
   only exception is the x86_64 dynamic loader link in `/lib64`, which the book
   requires.
4. Commands that must modify the **live** system and cannot be expressed as
   files in `DESTDIR` go into `post_install_commands`. They run as root in `/`
   after the files are committed, *outside* the sandbox. Examples: `pwconv`,
   `grpconv`, `systemd-machine-id-setup`, `systemctl preset-all`,
   `udev-hwdb update`, `ldconfig`, editing an existing `/etc/passwd`. Keep them
   idempotent (re-running a failed install runs them again).
5. Files under `/etc` that the book creates with `cat > /etc/foo << EOF` are
   written to `$DESTDIR/etc/foo` (create `$DESTDIR/etc` first).
6. **Test suites are skipped** (`make check`, `make test`, `ninja test`, ...).
   Do not include them; mention them only in a short comment for the packages
   the book calls critical (glibc, gcc, binutils, gmp, mpfr).
7. Sanity checks that only print output for a human (`readelf ... | grep`,
   `grep ... dummy.log`) are left out.
8. Parallelism comes from `MAKEFLAGS`, forwarded by the driver scripts; just
   call `make`. `$RAVEN_JOBS` holds the CPU count if a tool needs it explicitly.
9. Optional documentation installs from the book are kept when they are cheap
   (`install -m644 doc/* $DESTDIR/usr/share/doc/...`). Skip anything that needs
   extra downloads not in `wget-list` (e.g. sqlite-doc .zip, which Raven cannot
   extract).
10. `/usr/share/info/dir` is written by many packages. The driver scripts set
    `RAVEN_NO_EXTRACT=/usr/share/info/dir` so no package owns it, and rebuild
    the index at the end. Don't delete it in recipes.
11. `pip3 install` always uses
    `--no-index --no-deps --ignore-installed --no-user --root=$DESTDIR --find-links dist <pkg>`:
    without `--ignore-installed` pip would skip ("already satisfied") or try to
    uninstall from the read-only live system.
12. Architecture: the book targets x86_64. Keep the book's `case $(uname -m)`
    blocks verbatim (adapted for `$DESTDIR`), and add an `aarch64)` branch only
    where it is required (GCC's `t-aarch64-linux` lib64 → lib, see the gcc
    recipes).

## Cross phase specifics (`cross/`)

The driver forwards `LFS`, `LFS_TGT` and `MAKEFLAGS` (`RAVEN_PASSENV`) and
mounts `$LFS` read-only into the sandbox (`RAVEN_SANDBOX_BINDS=$LFS`), so the
cross toolchain in `$LFS/tools` and the target headers/libraries are visible
at their real path. Every cross recipe starts with:

```sh
export PATH=$LFS/tools/bin:/usr/bin:/bin CONFIG_SITE=$LFS/usr/share/config.site LC_ALL=POSIX
```

`make install` with `--prefix=$LFS/tools` therefore becomes
`make DESTDIR=$DESTDIR install` (files land in `/out$LFS/tools`), and
`make DESTDIR=$LFS install` becomes `make DESTDIR=$DESTDIR$LFS install`.
Commands that run a just-installed tool (e.g. `$LFS_TGT-gcc -print-file-name`)
cannot see `/out` through `$LFS`; compute such paths from the build tree
instead.
