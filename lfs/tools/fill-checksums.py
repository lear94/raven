#!/usr/bin/env python3
"""Fill `sha256_sum` values in LFS recipes from the downloaded sources.

For every `source_url = "..."` / `url = "..."` in lfs/recipes/**/*.toml, the
following `sha256_sum` is replaced by the SHA-256 of lfs/sources/<basename>.
Each file is first checked against the book's md5sums, so a corrupted or
tampered download is never trusted.

Usage: fill-checksums.py [--check]   (--check: only report, exit 1 on mismatch)
"""
import hashlib
import pathlib
import re
import sys

LFS_DIR = pathlib.Path(__file__).resolve().parent.parent
SOURCES = LFS_DIR / "sources"
RECIPES = LFS_DIR / "recipes"
PAIR = re.compile(r'((?:source_url|url)\s*=\s*"([^"]+)"[^"]*?sha256_sum\s*=\s*")([^"]*)(")')


def load_md5s():
    md5s = {}
    for line in (SOURCES / "md5sums").read_text().splitlines():
        parts = line.split()
        if len(parts) == 2:
            md5s[parts[1]] = parts[0]
    return md5s


def digest(path, algo):
    h = hashlib.new(algo)
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    check_only = "--check" in sys.argv[1:]
    md5s = load_md5s()
    cache = {}
    errors = []

    def sha_for(url):
        name = url.rstrip("/").rsplit("/", 1)[-1]
        if name in cache:
            return cache[name]
        path = SOURCES / name
        if not path.is_file():
            raise LookupError(f"{name}: not downloaded (run lfs/fetch-sources.sh)")
        expected = md5s.get(name)
        if expected is None:
            raise LookupError(f"{name}: not listed in the book's md5sums")
        if digest(path, "md5") != expected:
            raise LookupError(f"{name}: md5 mismatch, delete it and download again")
        cache[name] = digest(path, "sha256")
        return cache[name]

    for recipe in sorted(RECIPES.rglob("*.toml")):
        text = recipe.read_text()

        def repl(m):
            try:
                sha = sha_for(m.group(2))
            except LookupError as e:
                errors.append(f"{recipe.relative_to(LFS_DIR)}: {e}")
                return m.group(0)
            if m.group(3) != sha:
                if check_only:
                    errors.append(f"{recipe.relative_to(LFS_DIR)}: wrong sha256 for {m.group(2)}")
                    return m.group(0)
            return m.group(1) + sha + m.group(4)

        new = PAIR.sub(repl, text)
        if new != text and not check_only:
            recipe.write_text(new)

    for e in errors:
        print("error:", e, file=sys.stderr)
    print(f"{len(cache)} source files verified")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
