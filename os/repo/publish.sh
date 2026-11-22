#!/bin/bash
# Publishes debs to our apt repo, a static site for GitHub Pages.
#
#   publish.sh SUITE SITE DEBDIR...
#
# SUITE is stable or edge. SITE is the output tree; it may already hold pool
# files (CI restores them from a cache). DEBDIR holds the new debs.
#
# Environment:
#   REPO_PREV   URL (or file:// URL) of the live site. Its indexes say what
#               each suite holds now. Pool files missing from SITE come from
#               there. Unset: an empty repo.
#   REPO_KEY    The armored secret signing key. Unset: GNUPGHOME already
#               holds it.
#   REPO_KEEP   Versions kept per package per suite (2).
#   REPO_DAYS   Days until Valid-Until (30).
#   REPO_MAX_MB Fail when the site grows past this (800).
#
# Rules (docs/updates.md, Our apt repo):
# - A pool file is never replaced. Two builds of one commit, or of an
#   unchanged kernel set, give one version with different bytes. The first
#   bytes stay, so every index that names a file agrees on its hash.
# - The other suite's index is kept as it was.
# - Indexes are also served by hash, the previous ones too, so a cached
#   InRelease never points at a missing file.
set -euo pipefail

suite=${1:?suite: stable or edge}
site=${2:?site directory}
shift 2
case $suite in stable | edge) ;; *) echo "unknown suite $suite" >&2; exit 2 ;; esac
keep=${REPO_KEEP:-2}
days=${REPO_DAYS:-30}
max_mb=${REPO_MAX_MB:-800}
suites=(stable edge)
arch=arm64

log() { printf '== %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

mkdir -p "$site"
site=$(cd "$site" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

index() { echo "dists/$1/main/binary-$arch"; }

fetch() {
    curl -fsSL --retry 3 -o "$2" "$1"
}

# A suite's live index into $2. Returns 1 only when the site says it isn't
# there (404, or no file): anything else stops the publish, since reading a
# flaky fetch as "empty" would publish that suite empty.
fetch_index() {
    local url=$1 out=$2 code
    if [[ $url == file://* ]]; then
        [ -f "${url#file://}" ] || return 1
        cp "${url#file://}" "$out"
        return 0
    fi
    code=$(curl -sSL --retry 3 -o "$out" -w '%{http_code}' "$url") || die "$url: fetch failed"
    case $code in
        200) return 0 ;;
        404)
            rm -f "$out"
            return 1
            ;;
        *) die "$url: HTTP $code" ;;
    esac
}

# Pool path for a deb: pool/main/<initial>/<package>/<file>, lib* by four
# letters as Debian does.
pool_path() {
    local pkg file=$2
    pkg=$1
    local init=${pkg:0:1}
    [[ $pkg == lib* ]] && init=${pkg:0:4}
    echo "pool/main/$init/$pkg/$file"
}

# 1. What each suite holds now: its Filename list, from the live index.
for s in "${suites[@]}"; do
    : > "$tmp/$s.list"
    [ -n "${REPO_PREV:-}" ] || continue
    if fetch_index "$REPO_PREV/$(index "$s")/Packages.xz" "$tmp/$s.prev.xz"; then
        xz -dc "$tmp/$s.prev.xz" > "$tmp/$s.prev"
        sed -n 's/^Filename: //p' "$tmp/$s.prev" > "$tmp/$s.list"
        log "$s: $(wc -l < "$tmp/$s.list") files live"
    fi
done

# 2. The new debs join the pool, unless their name is there already.
new=0 kept=0
for d in "$@"; do
    for deb in "$d"/*.deb; do
        [ -e "$deb" ] || continue
        pkg=$(dpkg-deb -f "$deb" Package)
        rel=$(pool_path "$pkg" "$(basename "$deb")")
        if [ -e "$site/$rel" ] || grep -qxF "$rel" "$tmp/stable.list" "$tmp/edge.list" 2>/dev/null; then
            kept=$((kept + 1))
        else
            mkdir -p "$(dirname "$site/$rel")"
            cp "$deb" "$site/$rel"
            new=$((new + 1))
        fi
        echo "$rel" >> "$tmp/$suite.list"
    done
done
log "$suite: $new new, $kept already in the pool"

# 3. Missing pool files come from the live site.
sort -u "$tmp"/stable.list "$tmp"/edge.list | while read -r rel; do
    [ -e "$site/$rel" ] && continue
    [ -n "${REPO_PREV:-}" ] || die "$rel is listed but not in the pool"
    mkdir -p "$(dirname "$site/$rel")"
    fetch "$REPO_PREV/$rel" "$site/$rel.part" || die "$rel: not on the live site"
    mv "$site/$rel.part" "$site/$rel"
done

# 4. Keep the newest $keep versions of each package in the published suite.

# Rows of "version<TAB>path", newest first by dpkg's own order. Insertion
# sort: a package has a handful of rows.
sort_dpkg() {
    local rows=() row i j
    mapfile -t rows < "$1"
    for ((i = 1; i < ${#rows[@]}; i++)); do
        row=${rows[i]}
        j=$((i - 1))
        while ((j >= 0)) && dpkg --compare-versions "${rows[j]%%$'\t'*}" lt "${row%%$'\t'*}"; do
            rows[j + 1]=${rows[j]}
            j=$((j - 1))
        done
        rows[j + 1]=$row
    done
    if ((${#rows[@]})); then printf '%s\n' "${rows[@]}"; fi
}

prune() {
    local list=$1 out=$2 rel pkg
    : > "$out"
    sort -u "$list" | while read -r rel; do
        printf '%s\t%s\t%s\n' "$(dpkg-deb -f "$site/$rel" Package)" \
            "$(dpkg-deb -f "$site/$rel" Version)" "$rel"
    done > "$tmp/rows"
    cut -f1 "$tmp/rows" | sort -u | while read -r pkg; do
        awk -F'\t' -v p="$pkg" '$1 == p {print $2 "\t" $3}' "$tmp/rows" > "$tmp/vers"
        sort_dpkg "$tmp/vers" | head -n "$keep" | cut -f2 >> "$out"
    done
}

prune "$tmp/$suite.list" "$tmp/$suite.final"
sort -o "$tmp/$suite.final" "$tmp/$suite.final"
other=edge
[ "$suite" = edge ] && other=stable
sort -u "$tmp/$other.list" > "$tmp/$other.final"

# 5. Drop pool files no suite names any more.
sort -u "$tmp/stable.final" "$tmp/edge.final" > "$tmp/all"
if [ -d "$site/pool" ]; then
    (cd "$site" && find pool -type f -name '*.deb') | sort |
        comm -23 - "$tmp/all" | while read -r rel; do rm -f "$site/$rel"; done
    find "$site/pool" -type d -empty -delete
fi

# 6. Indexes. A stanza is the deb's control plus where it is and its hash.
stanza() {
    local rel=$1 f=$site/$1
    dpkg-deb -f "$f"
    echo "Filename: $rel"
    echo "Size: $(stat -c %s "$f")"
    echo "SHA256: $(sha256sum "$f" | cut -d' ' -f1)"
    echo
}

LC_ALL=C
export LC_ALL
now=$(date -u +%s)
for s in "${suites[@]}"; do
    dir=$site/$(index "$s")
    mkdir -p "$dir/by-hash/SHA256"
    # Both suites are written and signed each time, which also renews
    # Valid-Until.
    while read -r rel; do stanza "$rel"; done < "$tmp/$s.final" > "$dir/Packages"
    xz -9e -T1 -c "$dir/Packages" > "$dir/Packages.xz"
    # The previous index stays reachable by its hash.
    if [ -f "$tmp/$s.prev.xz" ] && [ -f "$tmp/$s.prev" ]; then
        cp "$tmp/$s.prev.xz" "$dir/by-hash/SHA256/$(sha256sum "$tmp/$s.prev.xz" | cut -d' ' -f1)"
        cp "$tmp/$s.prev" "$dir/by-hash/SHA256/$(sha256sum "$tmp/$s.prev" | cut -d' ' -f1)"
    fi
    for f in Packages Packages.xz; do
        cp "$dir/$f" "$dir/by-hash/SHA256/$(sha256sum "$dir/$f" | cut -d' ' -f1)"
    done

    rel=$site/dists/$s/Release
    {
        echo "Origin: Gh0st"
        echo "Label: Gh0st"
        echo "Suite: $s"
        echo "Codename: $s"
        echo "Date: $(date -u -d "@$now" '+%a, %d %b %Y %H:%M:%S UTC')"
        echo "Valid-Until: $(date -u -d "@$((now + days * 86400))" '+%a, %d %b %Y %H:%M:%S UTC')"
        echo "Architectures: $arch"
        echo "Components: main"
        echo "Acquire-By-Hash: yes"
        echo "Description: Gh0st OS $s"
        echo "SHA256:"
        for f in Packages Packages.xz; do
            p=$dir/$f
            printf ' %s %16s %s\n' "$(sha256sum "$p" | cut -d' ' -f1)" "$(stat -c %s "$p")" \
                "main/binary-$arch/$f"
        done
    } > "$rel"
done

# 7. Sign.
if [ -n "${REPO_KEY:-}" ]; then
    GNUPGHOME=$tmp/gnupg
    export GNUPGHOME
    mkdir -m 700 "$GNUPGHOME"
    printf '%s\n' "$REPO_KEY" | gpg --batch --quiet --import
fi
gpg=(gpg --batch --yes --quiet --pinentry-mode loopback --digest-algo SHA512)
for s in "${suites[@]}"; do
    d=$site/dists/$s
    "${gpg[@]}" --clearsign -o "$d/InRelease" "$d/Release"
    "${gpg[@]}" --armor --detach-sign -o "$d/Release.gpg" "$d/Release"
done
fpr=$(gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" {print $10; exit}')
gpg --batch --export --export-options export-minimal "$fpr" > "$site/gh0st-archive-keyring.pgp"

cat > "$site/index.html" <<EOF
<!doctype html>
<meta charset="utf-8">
<title>Gh0st OS apt</title>
<p>The Gh0st OS apt repo, for the CardputerZero. Suites: stable, edge.</p>
<p>Signing key <a href="gh0st-archive-keyring.pgp">gh0st-archive-keyring.pgp</a>,
fingerprint <code>$fpr</code>.</p>
EOF
# Pages serves the tree as is.
touch "$site/.nojekyll"

mb=$(du -sm "$site" | cut -f1)
log "site: $mb MB"
[ "$mb" -le "$max_mb" ] || die "the site is $mb MB, over $max_mb MB: move the pool to release assets"
log "$suite: $(wc -l < "$tmp/$suite.final") packages published"
