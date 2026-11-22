#!/bin/sh
# Downloads every source in sources/wget-list (LFS 13.1-systemd) and verifies
# them against the book's md5sums. Safe to re-run: verified files are kept.
# GNU files are tried on several mirrors (the ftpmirror redirector and
# ftp.gnu.org are sometimes unreachable). Override with GNU_MIRRORS="url ...".
set -u
cd "$(dirname "$0")/sources" || exit 1
fetch() {
  url=$1; f=$(basename "$url")
  sum=$(awk -v f="$f" '$2==f {print $1}' md5sums)
  if [ -s "$f" ]; then
    have=$( (md5sum "$f" 2>/dev/null || md5 -r "$f") | awk '{print $1}')
    [ "$have" = "$sum" ] && return 0
  fi
  urls=$url
  case "$url" in https://ftpmirror.gnu.org/*)
    p=${url#https://ftpmirror.gnu.org/}
    urls=""
    for m in ${GNU_MIRRORS:-https://mirrors.ocf.berkeley.edu/gnu https://mirror.csclub.uwaterloo.ca/gnu https://ftpmirror.gnu.org https://ftp.gnu.org/gnu}; do
      urls="$urls $m/$p"
    done ;;
  esac
  for u in $urls; do
    if curl -fsSL --retry 2 --connect-timeout 20 --speed-limit 20000 --speed-time 30 \
         -o "$f.part" "$u"; then
      mv "$f.part" "$f"; return 0
    fi
  done
  rm -f "$f.part"; echo "FAILED: $url"; return 1
}
export -f fetch 2>/dev/null
fails=0
while read -r url; do
  case "$url" in   # not used by the systemd recipes (docs zip, System V only)
    *.zip|*lfs-bootscripts*|*sysvinit*|*sysklogd*|*udev-lfs*) continue ;;
  esac
  fetch "$url" & 
  while [ "$(jobs -p | wc -l)" -ge 6 ]; do sleep 1; done
done < wget-list
wait
bad=0
while read -r sum f; do
  case "$f" in *.zip|lfs-bootscripts*|sysvinit*|sysklogd*|udev-lfs*) continue ;; esac
  [ -f "$f" ] || { echo "MISSING: $f"; bad=1; continue; }
  have=$( (md5sum "$f" 2>/dev/null || md5 -r "$f") | awk '{print $1}')
  [ "$have" = "$sum" ] || { echo "BAD MD5: $f"; bad=1; }
done < md5sums
[ $bad -eq 0 ] && echo "All sources downloaded and verified."
exit $bad
