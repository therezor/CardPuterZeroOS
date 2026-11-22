#!/bin/bash
# Builds one of our .debs from a package directory. Usage:
#   build-deb.sh PKGDIR OUTDIR
# PKGDIR holds DEBIAN/ (control and maintainer scripts), an optional root/
# tree copied as is, and an optional stage.sh that adds built files. stage.sh
# runs with the staging root as $1. @VERSION@-style words in DEBIAN/ files
# are replaced from the environment (see subst below).
set -euo pipefail

pkgdir=$(cd "$1" && pwd)
outdir=$2
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

if [ -d "$pkgdir/root" ]; then
    cp -a "$pkgdir/root/." "$stage/"
fi
if [ -x "$pkgdir/stage.sh" ]; then
    "$pkgdir/stage.sh" "$stage"
fi

# @NAME@ becomes $NAME for each name below (set by build-image.sh), and
# @V:pkg@ the version of pkg in lock.json, so pins live in one place.
subst() {
    local args=() v p
    for v in KVER KVERSION PANEL_VERSION GH0ST_VERSION BASE_VERSION M5COMMIT; do
        args+=(-e "s|@$v@|${!v:-}|g")
    done
    for p in $(grep -o '@V:[^@]*@' "$1" | sort -u | sed 's/^@V://; s/@$//'); do
        v=$(jq -r --arg p "$p" '[.rpt[], .apps[], .repack[] | select(.name == $p) | .version][0] // empty' "${LOCK:?}")
        [ -n "$v" ] || { echo "error: $p is not in lock.json" >&2; exit 1; }
        args+=(-e "s|@V:$p@|$v|g")
    done
    sed "${args[@]}" "$1"
}

mkdir -p "$stage/DEBIAN"
for f in "$pkgdir"/DEBIAN/*; do
    name=$(basename "$f")
    subst "$f" > "$stage/DEBIAN/$name"
    case $name in
        control|conffiles|triggers|shlibs) chmod 644 "$stage/DEBIAN/$name" ;;
        *) chmod 755 "$stage/DEBIAN/$name" ;;
    esac
done

# Everything under /etc is a conffile, so local edits survive upgrades.
if [ -d "$stage/etc" ]; then
    (cd "$stage" && find etc -type f -printf '/%p\n' | sort) >> "$stage/DEBIAN/conffiles"
    [ -s "$stage/DEBIAN/conffiles" ] || rm "$stage/DEBIAN/conffiles"
fi

size=$(du -sk --exclude=DEBIAN "$stage" | cut -f1)
echo "Installed-Size: $size" >> "$stage/DEBIAN/control"

if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
    find "$stage" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
fi

mkdir -p "$outdir"
dpkg-deb --root-owner-group -Zxz --build "$stage" "$outdir" >/dev/null
pkg=$(sed -n 's/^Package: //p' "$stage/DEBIAN/control")
ver=$(sed -n 's/^Version: //p' "$stage/DEBIAN/control")
echo "built $pkg $ver"
