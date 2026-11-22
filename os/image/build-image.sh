#!/bin/bash
# Runs inside the build container (see os/build.sh). The repo is mounted
# read-only at /src, the work volume at /work, the output dir at /out.
#   build-image.sh fetch          download and verify every pinned input
#   build-image.sh debs           build our packages, index both local repos
#   build-image.sh image VARIANT  rootfs, partitions, disk image (release)
#   build-image.sh check VARIANT  tier 1 checks on the last image
#   build-image.sh smoke          first boot of the release image in QEMU
#   build-image.sh relock VARIANT write the last build's lock and units to /out
set -euo pipefail

SRC=/src
OS=$SRC/os
LOCK=$OS/lock.json
WORK=/work
CACHE=$WORK/cache
export CARGO_HOME=$WORK/cargo-home CARGO_TARGET_DIR=$WORK/target

KVER=$(jq -r .kernel.release "$LOCK")
KVERSION=$(jq -r .kernel.version "$LOCK")
M5COMMIT=$(jq -r '.git[] | select(.name=="m5stack-linux-dtoverlays") | .commit' "$LOCK")
SNAPSHOT=$(jq -r .debian.snapshot "$LOCK")
# os/build.sh passes the commit time and hash: a worktree's .git is not
# reachable from inside the container.
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(date +%s)}
CPZ_GIT_REV=${CPZ_GIT_REV:-unknown}
GH0ST_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$SRC/Cargo.toml" | head -1)+$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y%m%d%H%M).$CPZ_GIT_REV"
# Our driver package moves with the kernel and the M5 commit it was built from.
PANEL_VERSION="${KVERSION#*:}+m5.${M5COMMIT:0:7}+cpz1"
export KVER KVERSION M5COMMIT PANEL_VERSION GH0ST_VERSION SOURCE_DATE_EPOCH SRC OS WORK CACHE LOCK

log() { printf '\033[1m== %s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# Every pinned deb: kind (rpt, app, repack), use, name, url, sha256.
lock_debs() {
    jq -r '(.rpt[] | ["rpt", .use, .name, .url, .sha256]),
           (.apps[] | ["app", "image", .name, .url, .sha256]),
           (.repack[] | ["repack", "build", .name, .url, .sha256]) | @tsv' "$LOCK"
}

deb_path() { echo "$CACHE/debs/$(basename "$1")"; }

step_fetch() {
    log "fetch: pinned debs"
    mkdir -p "$CACHE/debs"
    lock_debs | while IFS=$'\t' read -r _kind _use name url sha; do
        f=$(deb_path "$url")
        if [ -f "$f" ] && echo "$sha  $f" | sha256sum -c --quiet 2>/dev/null; then
            continue
        fi
        echo "  $name"
        curl -fsSL --retry 3 -o "$f.part" "$url"
        echo "$sha  $f.part" | sha256sum -c --quiet || die "$name: SHA-256 mismatch"
        mv "$f.part" "$f"
    done

    log "fetch: git sources"
    jq -r '.git[] | [.name, .url, .commit] | @tsv' "$LOCK" |
    while IFS=$'\t' read -r name url commit; do
        d=$CACHE/git/$name
        if [ "$(git -C "$d" rev-parse HEAD 2>/dev/null)" != "$commit" ]; then
            echo "  $name $commit"
            rm -rf "$d"
            git init -q "$d"
            git -C "$d" fetch -q --depth 1 "$url" "$commit"
            git -C "$d" -c advice.detachedHead=false checkout -q FETCH_HEAD
        fi
        [ "$(git -C "$d" rev-parse HEAD)" = "$commit" ] || die "$name: wrong commit"
    done

    log "fetch: cargo"
    cargo fetch --locked --manifest-path "$SRC/Cargo.toml" >/dev/null
    log "fetch: done"
}

# A flat local apt repo over a directory of debs. Its contents were checked
# by SHA-256 on fetch, so apt may trust it without a signature.
index_repo() {
    (cd "$1" && apt-ftparchive packages . > Packages &&
        apt-ftparchive -o APT::FTPArchive::Release::Origin="$2" release . > Release)
}

step_vendor() {
    log "vendor repo"
    rm -rf "$WORK/repo-vendor"
    mkdir -p "$WORK/repo-vendor"
    lock_debs | while IFS=$'\t' read -r _kind use _name url _sha; do
        if [ "$use" = image ]; then cp "$(deb_path "$url")" "$WORK/repo-vendor/"; fi
    done
    index_repo "$WORK/repo-vendor" cpzeroos-vendor
}

# The pinned headers go into this throwaway container, for the module builds.
install_kernel_headers() {
    [ -d "/usr/src/linux-headers-$KVER" ] && return
    log "kernel headers $KVER"
    local debs=()
    while IFS= read -r url; do debs+=("$(deb_path "$url")"); done < <(
        jq -r --arg k "$KVER" '.rpt[] | select(.use == "build" or .name == "linux-base-\($k)") | .url' "$LOCK")
    dpkg -i "${debs[@]}" >/dev/null
}

step_debs() {
    step_vendor
    install_kernel_headers
    log "cargo build"
    cargo build -q --release --frozen --target aarch64-unknown-linux-musl \
        --manifest-path "$SRC/Cargo.toml" -p gh0st -p gh0st-settings -p gh0st-term -p gh0st-update
    log "debs"
    rm -rf "$WORK/repo-ours"
    mkdir -p "$WORK/repo-ours"
    for p in "$OS"/packages/*/; do
        [ -d "$p/DEBIAN" ] || continue
        "$OS/packages/build-deb.sh" "$p" "$WORK/repo-ours"
    done
    index_repo "$WORK/repo-ours" cpzeroos
}

# Package names from a manifest list: the first word of each line.
list_words() { grep -hv '^#' "$@" 2>/dev/null | awk 'NF {print $1}'; }

step_image() {
    local variant=${1:?variant: release}
    [ -d "$OS/image/variants/$variant" ] || die "no variant $variant"
    # shellcheck source=/dev/null
    . "$OS/image/layout.conf"
    local id=${DISK_ID#0x}
    export PARTUUID_BOOT=$id-01 PARTUUID_ROOT=$id-02 PARTUUID_DATA=$id-03
    local build=$WORK/build/image-$variant
    export REPORTS=$build/reports
    rm -rf "$build"
    mkdir -p "$build"

    log "rootfs ($variant)"
    local roots dpkgopts=() snap=https://snapshot.debian.org/archive
    roots=$(list_words "$OS/image/manifest/roots.list" "$OS/image/variants/$variant/roots.list" | paste -sd, -)
    export ROOTS=$roots
    while IFS= read -r line; do
        dpkgopts+=(--dpkgopt="$line")
    done < <(grep -E '^path-' "$OS/packages/cpzeroos-base/root/etc/dpkg/dpkg.cfg.d/50cpzeroos-excludes")
    mmdebstrap --mode=root --variant=minbase --architectures=arm64 \
        --aptopt='APT::Install-Recommends "false"' \
        --aptopt='Acquire::Languages "none"' \
        "${dpkgopts[@]}" \
        --include="$roots" \
        --setup-hook="install -D -m 644 $OS/packages/cpzeroos-base/root/etc/apt/preferences.d/cpzeroos-deny.pref \"\$1/etc/apt/preferences.d/cpzeroos-deny.pref\"" \
        --customize-hook="bash ${TRACE:+-x} $OS/image/hooks/customize.sh \"\$1\"" \
        --customize-hook="sync-out /boot/firmware $build/boot" \
        --customize-hook="sync-out /home $build/home" \
        --customize-hook='rm -rf "$1"/boot/firmware/* "$1"/home/*' \
        trixie "$build/rootfs.tar" \
        "deb $snap/debian/$SNAPSHOT trixie trixie-updates main" \
        "deb $snap/debian-security/$SNAPSHOT trixie-security main" \
        "deb [trusted=yes] copy://$WORK/repo-vendor ./" \
        "deb [trusted=yes] copy://$WORK/repo-ours ./" > "$build/mmdebstrap.log" 2>&1 || {
        tail -40 "$build/mmdebstrap.log"
        die "mmdebstrap failed, log in $build/mmdebstrap.log"
    }
    grep -E '^W:' "$build/mmdebstrap.log" || true

    log "boot partition"
    local bootdir=$build/bootfs
    mkdir -p "$bootdir"
    list_words "$OS/image/manifest/boot-files.list" | while read -r f; do
        [ -e "$build/boot/$f" ] || die "boot file $f missing"
        cp -a "$build/boot/$f" "$bootdir/"
    done
    cp "$OS/image/boot/config.txt" "$bootdir/config.txt"
    sed "s|@PARTUUID@|$PARTUUID_ROOT|" "$OS/image/boot/cmdline.txt.in" > "$bootdir/cmdline.txt"
    mkfs.vfat -F 32 -n BOOT -i "$id" -C "$build/boot.vfat" $((BOOT_MIB * 1024)) >/dev/null
    mcopy -s -i "$build/boot.vfat" "$bootdir"/* ::/

    log "ext4 partitions"
    mke2fs -q -t ext4 -L rootfs -m 2 -d "$build/rootfs.tar" "$build/root.ext4" "${ROOT_MIB}M"
    # From the directory, not a tar: the data partition's root is /home.
    # It grows online at first boot, from 256 MiB to the whole card. At
    # this size mke2fs picks 1 KiB blocks, whose reserved descriptors run
    # out near 33 GiB; past that the kernel's resize corrupted the group
    # descriptors on a 64 GB card. 4 KiB blocks, with descriptors reserved
    # for 1 TiB, grow the common way.
    mke2fs -q -t ext4 -T default -b 4096 -E resize=$((1 << 28)) -L data -m 1 \
        -d "$build/home" "$build/data.ext4" "${DATA_MIB}M"

    log "disk image"
    local img=$build/cpzeroos-$variant.img
    local s1=$((ALIGN_MIB * 2048)) b=$((BOOT_MIB * 2048)) r=$((ROOT_MIB * 2048)) d=$((DATA_MIB * 2048))
    truncate -s $(( (ALIGN_MIB + BOOT_MIB + ROOT_MIB + DATA_MIB) * 1024 * 1024 )) "$img"
    sfdisk -q "$img" <<SFDISK
label: dos
label-id: $DISK_ID
start=$s1, size=$b, type=c, bootable
start=$((s1 + b)), size=$r, type=83
start=$((s1 + b + r)), size=$d, type=83
SFDISK
    dd if="$build/boot.vfat" of="$img" bs=512 seek=$s1 conv=notrunc,sparse status=none
    dd if="$build/root.ext4" of="$img" bs=512 seek=$((s1 + b)) conv=notrunc,sparse status=none
    dd if="$build/data.ext4" of="$img" bs=512 seek=$((s1 + b + r)) conv=notrunc,sparse status=none

    log "pack"
    e2fsck -fn "$build/root.ext4" >/dev/null
    e2fsck -fn "$build/data.ext4" >/dev/null
    fsck.vfat -n "$build/boot.vfat" >/dev/null
    bmaptool -q create -o "$img.bmap" "$img"
    xz -T0 -6 -f -k "$img"
    local name
    name=cpzeroos-$variant-$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y%m%d)
    install -m 644 "$img.xz" "/out/$name.img.xz"
    install -m 644 "$img.bmap" "/out/$name.bmap"
    install -m 644 "$REPORTS/packages.tsv" "/out/$name.packages.tsv"
    (cd /out && sha256sum "$name.img.xz" "$name.bmap" > "$name.sha256")
    log "image: os/out/$name.img.xz ($(du -h "$img.xz" | cut -f1), $(wc -l < "$REPORTS/packages.tsv") packages)"
}

# name<TAB>version, with our own packages' versions masked: they move with
# every commit, and the lock is for what comes from outside.
lock_lines() {
    awk -F'\t' -v OFS='\t' '$1 ~ /^(cpzeroos-|gh0st)/ {$2 = "(ours)"} {print $1, $2}' "$1"
}
export -f lock_lines

# The repo is read-only in here: relock writes to /out and os/build.sh
# copies the files into os/image/variants/VARIANT.
step_relock() {
    local variant=${1:?variant}
    local R=$WORK/build/image-$variant/reports
    [ -f "$R/packages.tsv" ] || die "no image for $variant: run os/build.sh image $variant"
    local d=/out/relock/$variant
    rm -rf "$d"
    mkdir -p "$d"
    {
        echo "# Every package in the $variant image, with its version. Generated by"
        echo "# os/build.sh relock $variant; review the diff before committing."
        lock_lines "$R/packages.tsv"
    } > "$d/packages.lock"
    cp "$R/units-system.txt" "$d/units-system.expected"
    cp "$R/units-user.txt" "$d/units-user.expected"
    log "relock: $(wc -l < "$R/packages.tsv") packages"
}

case ${1:-} in
    fetch) step_fetch ;;
    debs) step_debs ;;
    image) step_image "${2:-}" ;;
    check) "$OS/image/checks/run.sh" "${2:?variant}" ;;
    smoke) "$OS/image/checks/smoke.sh" ;;
    relock) step_relock "${2:-}" ;;
    publish) "$OS/repo/publish.sh" "${2:?suite: stable or edge}" /out/apt "$WORK/repo-ours" "$WORK/repo-vendor" ;;
    shell) exec bash ;;
    *) die "usage: build-image.sh fetch|debs|image VARIANT|check VARIANT|smoke|relock VARIANT|publish SUITE|shell" ;;
esac
