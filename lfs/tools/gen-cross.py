#!/usr/bin/env python3
"""Generates lfs/recipes/cross/*.toml (LFS 13.1 chapters 4.2, 5 and 6).

The recipes are plain TOML and can be edited by hand; this script only exists
to keep the 23 cross recipes consistent. Re-running it overwrites them (run
fill-checksums.py afterwards)."""
import pathlib

OUT = pathlib.Path(__file__).resolve().parent.parent / "recipes" / "cross"
WGET = (OUT.parent.parent / "sources" / "wget-list").read_text().split()

PRELUDE = ("umask 022; export PATH=$LFS/tools/bin:/usr/bin:/bin "
           "CONFIG_SITE=$LFS/usr/share/config.site LC_ALL=POSIX")
GMP_MPFR_MPC = [
    "tar -xf $RAVEN_SOURCES/mpfr-4.2.2.tar.xz\nmv -v mpfr-4.2.2 mpfr\n"
    "tar -xf $RAVEN_SOURCES/gmp-6.3.0.tar.xz\nmv -v gmp-6.3.0 gmp\n"
    "tar -xf $RAVEN_SOURCES/mpc-1.4.1.tar.xz\nmv -v mpc-1.4.1 mpc",
    # The book only handles x86_64; on aarch64 GCC would also use lib64.
    "case $(uname -m) in\n"
    "  x86_64)  sed -e '/m64=/s/lib64/lib/' -i.orig gcc/config/i386/t-linux64 ;;\n"
    "  aarch64) sed -e '/mabi.lp64=/s/lib64/lib/' -i.orig gcc/config/aarch64/t-aarch64-linux ;;\n"
    "esac",
]


def url(fname):
    hits = [u for u in WGET if u.rsplit("/", 1)[-1] == fname]
    assert len(hits) == 1, fname
    return hits[0]


def q(cmd):
    """TOML string: multi-line literal for anything with newlines/quotes."""
    if "\n" in cmd or "'" in cmd or '"' in cmd or "\\" in cmd:
        assert "'''" not in cmd
        return "'''\n" + cmd + "'''"
    return "'" + cmd + "'"


recipes = []


def recipe(name, version, section, desc, tarball, build, install, extras=(), overwrites=()):
    recipes.append(dict(name=name, version=version, section=section, desc=desc,
                        tarball=tarball, build=build, install=install, extras=extras,
                        overwrites=overwrites))


def autotools(name, version, section, desc, tarball, configure, pre=(), post=()):
    recipe(name, version, section, desc, tarball,
           [PRELUDE, *pre, configure, "make"],
           ["make DESTDIR=$DESTDIR$LFS install", *post])


# ---- Chapter 4.2 -------------------------------------------------------------
recipe("lfs-layout", "13.1", "4.2", "Limited directory layout of $LFS", None, [PRELUDE], [
    "mkdir -pv $DESTDIR$LFS/{etc,var} $DESTDIR$LFS/usr/{bin,lib,sbin}\n"
    "for i in bin lib sbin; do\n  ln -sv usr/$i $DESTDIR$LFS/$i\ndone\n"
    "case $(uname -m) in\n  x86_64) mkdir -pv $DESTDIR$LFS/lib64 ;;\nesac",
    "mkdir -pv $DESTDIR$LFS/tools $DESTDIR$LFS/tools/bin",
    # Book §6.2 (M4) creates this file; it must exist before any chapter 6
    # configure runs, so it lives in the layout package.
    "mkdir -pv $DESTDIR$LFS/usr/share\n"
    "cat > $DESTDIR$LFS/usr/share/config.site << EOF\n"
    "ac_cv_func_posix_spawn_file_actions_addchdir=yes\n"
    "ac_cv_func_posix_spawn_file_actions_addfchdir=yes\nEOF",
])

# ---- Chapter 5 ---------------------------------------------------------------
recipe("binutils-pass1", "2.47", "5.2", "Binutils, pass 1 (cross)", "binutils-2.47.tar.xz",
       [PRELUDE, "mkdir -v build\ncd       build",
        "../configure --prefix=$LFS/tools \\\n"
        "             --with-sysroot=$LFS \\\n"
        "             --target=$LFS_TGT   \\\n"
        "             --disable-nls       \\\n"
        "             --enable-gprofng=no \\\n"
        "             --disable-werror    \\\n"
        "             --enable-new-dtags  \\\n"
        "             --enable-default-hash-style=gnu",
        "make"],
       ["make DESTDIR=$DESTDIR install"])

recipe("gcc-pass1", "16.2.0", "5.3", "GCC, pass 1 (cross)", "gcc-16.2.0.tar.xz",
       [PRELUDE, *GMP_MPFR_MPC, "mkdir -v build\ncd       build",
        "../configure                  \\\n"
        "    --target=$LFS_TGT         \\\n"
        "    --prefix=$LFS/tools       \\\n"
        "    --with-glibc-version=2.44 \\\n"
        "    --with-sysroot=$LFS       \\\n"
        "    --with-newlib             \\\n"
        "    --without-headers         \\\n"
        "    --enable-default-pie      \\\n"
        "    --enable-default-ssp      \\\n"
        "    --disable-fixincludes     \\\n"
        "    --disable-nls             \\\n"
        "    --disable-shared          \\\n"
        "    --disable-multilib        \\\n"
        "    --disable-threads         \\\n"
        "    --disable-libatomic       \\\n"
        "    --disable-libgomp         \\\n"
        "    --disable-libquadmath     \\\n"
        "    --disable-libssp          \\\n"
        "    --disable-libvtv          \\\n"
        "    --disable-libstdcxx       \\\n"
        "    --enable-languages=c,c++",
        "make"],
       ["make DESTDIR=$DESTDIR install",
        # Book: $($LFS_TGT-gcc -print-file-name=include)/limits.h. The new gcc
        # is still in $DESTDIR, so the include dir is computed directly.
        "cat ../gcc/{limitx,glimits,limity}.h > \\\n"
        "  $DESTDIR$LFS/tools/lib/gcc/$LFS_TGT/$(cat ../gcc/BASE-VER)/include/limits.h"],
       extras=["mpfr-4.2.2.tar.xz", "gmp-6.3.0.tar.xz", "mpc-1.4.1.tar.xz"])

recipe("linux-headers", "7.1.8", "5.4", "Linux API headers", "linux-7.1.8.tar.xz",
       [PRELUDE, "make mrproper",
        "make headers\nfind usr/include -type f ! -name '*.h' -delete"],
       ["mkdir -pv $DESTDIR$LFS/usr\ncp -rv usr/include $DESTDIR$LFS/usr"])

recipe("glibc", "2.44", "5.5", "Glibc (temporary, cross-compiled)", "glibc-2.44.tar.xz",
       [PRELUDE,
        "patch -Np1 -i $RAVEN_SOURCES/glibc-fhs-1.patch",
        "patch -Np1 -i $RAVEN_SOURCES/glibc-2.44-upstream_fixes-1.patch",
        "mkdir -v build\ncd       build",
        'echo "rootsbindir=/usr/sbin" > configparms',
        "../configure                             \\\n"
        "      --prefix=/usr                      \\\n"
        "      --host=$LFS_TGT                    \\\n"
        "      --build=$(../scripts/config.guess) \\\n"
        "      --disable-nscd                     \\\n"
        "      libc_cv_slibdir=/usr/lib           \\\n"
        "      --enable-kernel=5.10",
        "make"],
       ["make DESTDIR=$DESTDIR$LFS install",
        "sed '/RTLDLIST=/s@/usr@@g' -i $DESTDIR$LFS/usr/bin/ldd",
        # Book runs this before building; the links point into the new glibc.
        "case $(uname -m) in\n"
        "    i?86)   ln -sfv ld-linux.so.2 $DESTDIR$LFS/usr/lib/ld-lsb.so.3\n    ;;\n"
        "    x86_64) mkdir -pv $DESTDIR$LFS/lib64\n"
        "            ln -sfv ../lib/ld-linux-x86-64.so.2 $DESTDIR$LFS/lib64\n"
        "            ln -sfv ../lib/ld-linux-x86-64.so.2 $DESTDIR$LFS/lib64/ld-lsb-x86-64.so.3\n    ;;\n"
        "esac"],
       extras=["glibc-fhs-1.patch", "glibc-2.44-upstream_fixes-1.patch"])

recipe("libstdc++", "16.2.0", "5.6", "Libstdc++ from GCC (cross)", "gcc-16.2.0.tar.xz",
       [PRELUDE, "mkdir -v build\ncd       build",
        "../libstdc++-v3/configure      \\\n"
        "    --host=$LFS_TGT            \\\n"
        "    --build=$(../config.guess) \\\n"
        "    CXX=$LFS_TGT-gcc           \\\n"
        "    --prefix=/usr              \\\n"
        "    --disable-multilib         \\\n"
        "    --disable-nls              \\\n"
        "    --disable-libstdcxx-pch    \\\n"
        "    --with-gxx-include-dir=/tools/$LFS_TGT/include/c++/16.2.0",
        "make"],
       ["make DESTDIR=$DESTDIR$LFS install",
        "rm -v $DESTDIR$LFS/usr/lib/lib{stdc++{,exp,fs},supc++}.la"])

# ---- Chapter 6 ---------------------------------------------------------------
HB = "            --host=$LFS_TGT \\\n            --build=$(build-aux/config.guess)"
autotools("m4", "1.4.21", "6.2", "M4 (temporary)", "m4-1.4.21.tar.xz",
          "./configure --prefix=/usr   \\\n" + HB)

recipe("ncurses", "6.6", "6.3", "Ncurses (temporary)", "ncurses-6.6.tar.gz",
       [PRELUDE,
        "mkdir build\npushd build\n  ../configure --prefix=$LFS/tools AWK=gawk\n"
        "  make -C include\n  make -C progs tic\npopd",
        "./configure --prefix=/usr                \\\n"
        "            --host=$LFS_TGT              \\\n"
        "            --build=$(./config.guess)    \\\n"
        "            --mandir=/usr/share/man      \\\n"
        "            --with-manpage-format=normal \\\n"
        "            --with-shared                \\\n"
        "            --without-normal             \\\n"
        "            --with-cxx-shared            \\\n"
        "            --without-debug              \\\n"
        "            --without-ada                \\\n"
        "            --disable-stripping          \\\n"
        "            AWK=gawk",
        "make"],
       ["mkdir -pv $DESTDIR$LFS/tools/bin\ninstall build/progs/tic $DESTDIR$LFS/tools/bin",
        # The install step needs that tic in PATH (book: it is in $LFS/tools/bin).
        "export PATH=$DESTDIR$LFS/tools/bin:$PATH",
        "make DESTDIR=$DESTDIR$LFS install\n"
        "ln -sv libncursesw.so $DESTDIR$LFS/usr/lib/libncurses.so\n"
        "sed -e 's/^#if.*XOPEN.*$/#if 1/' \\\n    -i $DESTDIR$LFS/usr/include/curses.h"])

recipe("bash", "5.3", "6.4", "Bash (temporary)", "bash-5.3.tar.gz",
       [PRELUDE,
        "./configure --prefix=/usr                      \\\n"
        "            --build=$(sh support/config.guess) \\\n"
        "            --host=$LFS_TGT                    \\\n"
        "            --without-bash-malloc              \\\n"
        "            --docdir=/usr/share/doc/bash-5.3",
        "make"],
       ["make DESTDIR=$DESTDIR$LFS install",
        "ln -sv bash $DESTDIR$LFS/usr/bin/sh  # book: $LFS/bin/sh (same file)"])

autotools("coreutils", "9.11", "6.5", "Coreutils (temporary)", "coreutils-9.11.tar.xz",
          "./configure --prefix=/usr                     \\\n"
          "            --host=$LFS_TGT                   \\\n"
          "            --build=$(build-aux/config.guess) \\\n"
          "            --enable-install-program=hostname",
          post=["mkdir -pv $DESTDIR$LFS/usr/sbin\n"
                "mv -v $DESTDIR$LFS/usr/bin/chroot              $DESTDIR$LFS/usr/sbin\n"
                "mkdir -pv $DESTDIR$LFS/usr/share/man/man8\n"
                "mv -v $DESTDIR$LFS/usr/share/man/man1/chroot.1 $DESTDIR$LFS/usr/share/man/man8/chroot.8\n"
                "sed -i 's/\"1\"/\"8\"/'                    $DESTDIR$LFS/usr/share/man/man8/chroot.8"])

autotools("diffutils", "3.12", "6.6", "Diffutils (temporary)", "diffutils-3.12.tar.xz",
          "./configure --prefix=/usr   \\\n            --host=$LFS_TGT \\\n"
          "            gl_cv_func_strcasecmp_works=yes \\\n"
          "            --build=$(./build-aux/config.guess)")

recipe("file", "5.48", "6.7", "File (temporary)", "file-5.48.tar.gz",
       [PRELUDE,
        "mkdir build\npushd build\n  ../configure --disable-bzlib      \\\n"
        "               --disable-libseccomp \\\n               --disable-xzlib      \\\n"
        "               --disable-zlib\n  make\npopd",
        "./configure --prefix=/usr --host=$LFS_TGT --build=$(./config.guess)",
        "make FILE_COMPILE=$(pwd)/build/src/file"],
       ["make DESTDIR=$DESTDIR$LFS install", "rm -v $DESTDIR$LFS/usr/lib/libmagic.la"])

autotools("findutils", "4.11.0", "6.8", "Findutils (temporary)", "findutils-4.11.0.tar.xz",
          "./configure --prefix=/usr                   \\\n"
          "            --localstatedir=/var/lib/locate \\\n" + HB)
autotools("gawk", "5.4.1", "6.9", "Gawk (temporary)", "gawk-5.4.1.tar.xz",
          "./configure --prefix=/usr   \\\n" + HB, pre=["sed -i 's/extras//' Makefile.in"])
autotools("grep", "3.12", "6.10", "Grep (temporary)", "grep-3.12.tar.xz",
          "./configure --prefix=/usr   \\\n            --host=$LFS_TGT \\\n"
          "            --build=$(./build-aux/config.guess)")
autotools("gzip", "1.14", "6.11", "Gzip (temporary)", "gzip-1.14.tar.xz",
          "./configure --prefix=/usr --host=$LFS_TGT")
autotools("make", "4.4.1", "6.12", "Make (temporary)", "make-4.4.1.tar.gz",
          "./configure --prefix=/usr   \\\n" + HB)
autotools("patch", "2.8", "6.13", "Patch (temporary)", "patch-2.8.tar.xz",
          "./configure --prefix=/usr   \\\n" + HB)
autotools("sed", "4.10", "6.14", "Sed (temporary)", "sed-4.10.tar.xz",
          "./configure --prefix=/usr   \\\n            --host=$LFS_TGT \\\n"
          "            --build=$(./build-aux/config.guess)")
autotools("tar", "1.35", "6.15", "Tar (temporary)", "tar-1.35.tar.xz",
          "./configure --prefix=/usr   \\\n" + HB)
autotools("xz", "5.8.3", "6.16", "Xz (temporary)", "xz-5.8.3.tar.xz",
          "./configure --prefix=/usr                     \\\n"
          "            --host=$LFS_TGT                   \\\n"
          "            --build=$(build-aux/config.guess) \\\n"
          "            --disable-static                  \\\n"
          "            --docdir=/usr/share/doc/xz-5.8.3",
          post=["rm -v $DESTDIR$LFS/usr/lib/liblzma.la"])

recipe("binutils-pass2", "2.47", "6.17", "Binutils, pass 2 (temporary)", "binutils-2.47.tar.xz",
       [PRELUDE, "sed '6031s/$add_dir//' -i ltmain.sh", "mkdir -v build\ncd       build",
        "../configure                   \\\n"
        "    --prefix=/usr              \\\n"
        "    --build=$(../config.guess) \\\n"
        "    --host=$LFS_TGT            \\\n"
        "    --disable-nls              \\\n"
        "    --enable-shared            \\\n"
        "    --enable-gprofng=no        \\\n"
        "    --disable-werror           \\\n"
        "    --enable-64-bit-bfd        \\\n"
        "    --enable-new-dtags         \\\n"
        "    --enable-default-hash-style=gnu",
        "make"],
       ["make DESTDIR=$DESTDIR$LFS install",
        "rm -v $DESTDIR$LFS/usr/lib/lib{bfd,ctf,ctf-nobfd,opcodes,sframe}.{a,la}"])

recipe("gcc-pass2", "16.2.0", "6.18", "GCC, pass 2 (temporary)", "gcc-16.2.0.tar.xz",
       [PRELUDE, *GMP_MPFR_MPC, "mkdir -v build\ncd       build",
        "../configure                   \\\n"
        "    --build=$(../config.guess) \\\n"
        "    --host=$LFS_TGT            \\\n"
        "    --target=$LFS_TGT          \\\n"
        "    --prefix=/usr              \\\n"
        "    --with-build-sysroot=$LFS  \\\n"
        "    --enable-default-pie       \\\n"
        "    --enable-default-ssp       \\\n"
        "    --disable-fixincludes      \\\n"
        "    --disable-nls              \\\n"
        "    --disable-multilib         \\\n"
        "    --disable-libatomic        \\\n"
        "    --disable-libgomp          \\\n"
        "    --disable-libquadmath      \\\n"
        "    --disable-libsanitizer     \\\n"
        "    --disable-libssp           \\\n"
        "    --disable-libvtv           \\\n"
        "    --enable-languages=c,c++   \\\n"
        "    CXX_FOR_TARGET=\"$LFS_TGT-gcc -nostdinc++\" \\\n"
        "    LDFLAGS_FOR_TARGET=-L$PWD/$LFS_TGT/libgcc \\\n"
        "    target_configargs=gcc_cv_target_thread_file=posix",
        "make"],
       ["make DESTDIR=$DESTDIR$LFS install", "ln -sv gcc $DESTDIR$LFS/usr/bin/cc"],
       extras=["mpfr-4.2.2.tar.xz", "gmp-6.3.0.tar.xz", "mpc-1.4.1.tar.xz"],
       # The book rebuilds libstdc++ here, replacing the chapter 5 one.
       overwrites=["libstdc++"])

prev = None
for r in recipes:
    lines = [f'name = "{r["name"]}"', f'version = "{r["version"]}"',
             f'description = "LFS 13.1 §{r["section"]} - {r["desc"]}"']
    lines.append(f'dependencies = ["{prev}"]' if prev else "dependencies = []")
    if r["tarball"]:
        lines += [f'source_url = "{url(r["tarball"])}"', 'sha256_sum = "FILL"']
    if r["overwrites"]:
        lines.append("overwrites = [" + ", ".join(f'"{o}"' for o in r["overwrites"]) + "]")
    if r["extras"]:
        lines.append("extra_sources = [")
        lines += [f'  {{ url = "{url(e)}", sha256_sum = "FILL" }},' for e in r["extras"]]
        lines.append("]")
    for key in ("build", "install"):
        lines.append(f"{key}_commands = [")
        lines += [f"  {q(c)}," for c in r[key]]
        lines.append("]")
    (OUT / f'{r["name"]}.toml').write_text("\n".join(lines) + "\n")
    prev = r["name"]
print(f"{len(recipes)} recipes written to {OUT}")
