#!/bin/sh
# Builds Gh0st OS in the build container. Same on the Mac and on CI.
#   os/build.sh fetch             download and verify the pinned inputs
#   os/build.sh debs              build our packages
#   os/build.sh image release     build the disk image into os/out/
#   os/build.sh check release     tier 1 checks
#   os/build.sh smoke             first boot of the image in QEMU
#   os/build.sh relock release    accept the packages and units of the last build
#   os/build.sh publish edge      add the built debs to the apt site in os/out/pages/apt
#   os/build.sh resign            sign the apt site again, nothing added
#   os/build.sh shell             a shell in the container
# Work happens on the Docker volume gh0st-work, never on the bind-mounted
# repo: macOS filesystems lose ownership and sparseness.
set -eu

os=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$os")
lock=$os/lock.json

docker info >/dev/null 2>&1 || {
    echo "Docker is not running. On the Mac, start Docker Desktop." >&2
    exit 1
}

base=$(jq -r .base_image "$lock")
rust=$(jq -r .rust "$lock")
tag=gh0st-build:$(cat "$os/Dockerfile" | { cat; echo "$base $rust"; } | shasum | cut -c1-12)
if ! docker image inspect "$tag" >/dev/null 2>&1; then
    docker build -q -t "$tag" --build-arg BASE="$base" --build-arg RUST_VERSION="$rust" "$os" >/dev/null
fi

# The container can't read a worktree's .git, so the commit comes as env.
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}
GH0ST_GIT_REV=$(git -C "$root" rev-parse --short HEAD)
git -C "$root" diff --quiet HEAD || GH0ST_GIT_REV=$GH0ST_GIT_REV.dirty
# Each of our packages takes its version from the last commit that touched
# its sources, so a push that changes only docs publishes nothing new to
# devices on edge (docs/updates.md). "epoch.rev", or "epoch" alone.
changed() {
    fmt=$1
    shift
    git -C "$root" log -1 --format="$fmt" -- "$@"
}
GH0ST_STAMP=$(changed '%ct.%h' crates assets packaging Cargo.toml Cargo.lock \
    os/packages/gh0st os/packages/gh0st-term)
case $GH0ST_GIT_REV in *.dirty) GH0ST_STAMP=$SOURCE_DATE_EPOCH.$GH0ST_GIT_REV ;; esac
GH0ST_BASE_STAMP=$(changed '%ct' os/packages/gh0st-base os/repo/gh0st.sources \
    os/repo/gh0st.pref os/repo/gh0st-archive-keyring.pgp)
# The kernel set: our driver and its packages, and the firmware pins its
# Depends name exactly. The kernel and M5 commit are in its version already;
# other lock.json edits (a stock app) must not reinstall it.
panel=$(changed '%ct' os/kernel os/packages/gh0st-panel-modules os/packages/gh0st-kernel)
firmware=$(git -C "$root" log -1 --format=%ct \
    -G'(raspi-firmware|firmware-brcm80211|bluez-firmware)' -- os/lock.json)
GH0ST_PANEL_STAMP=$((${panel:-0} > ${firmware:-0} ? ${panel:-0} : ${firmware:-0}))
# gh0st-wifi: its own files and the firmware packages it takes files from.
wifi=$(changed '%ct' os/packages/gh0st-wifi)
wifi_fw=$(git -C "$root" log -1 --format=%ct \
    -G'firmware-(atheros|mediatek|realtek)' -- os/lock.json)
GH0ST_WIFI_STAMP=$((${wifi:-0} > ${wifi_fw:-0} ? ${wifi:-0} : ${wifi_fw:-0}))
export SOURCE_DATE_EPOCH GH0ST_GIT_REV GH0ST_STAMP GH0ST_BASE_STAMP GH0ST_PANEL_STAMP GH0ST_WIFI_STAMP

envs=
for v in SOURCE_DATE_EPOCH GH0ST_GIT_REV GH0ST_STAMP GH0ST_BASE_STAMP GH0ST_PANEL_STAMP GH0ST_WIFI_STAMP \
    GH0ST_LOG TRACE REPO_PREV REPO_KEY REPO_KEEP REPO_DAYS REPO_MAX_MB; do
    envs="$envs -e $v"
done

mkdir -p "$os/out"
tty=
[ -t 0 ] && tty=-it
# mmdebstrap mounts /proc, /sys and /dev in its chroot, which needs
# CAP_SYS_ADMIN and no seccomp or AppArmor mount filter. No loop devices.
# shellcheck disable=SC2086
docker run --rm $tty $envs \
    --cap-add SYS_ADMIN --security-opt apparmor=unconfined --security-opt seccomp=unconfined \
    -v gh0st-work:/work -v "$root:/src:ro" -v "$os/out:/out" \
    "$tag" /src/os/image/build-image.sh "$@"

if [ "${1:-}" = relock ]; then
    v=${2:?variant}
    cp "$os/out/relock/$v"/* "$os/image/variants/$v/"
    rm -rf "$os/out/relock"
    git -C "$root" diff --stat -- "os/image/variants/$v"
fi
