# The Gh0st OS build container: Debian trixie arm64 with the image tools.
# Same image on the Mac (Docker Desktop, native arm64) and on GitHub's
# ubuntu-24.04-arm runners. The base is pinned by digest in lock.json.
ARG BASE
FROM ${BASE}

ARG RUST_VERSION
ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8

# Image tools, the kernel module toolchain (the rpt headers want gcc-14,
# pahole and python3), cc for cargo build scripts, g++, libjpeg, ffmpeg's
# libraries and libpulse for the camera and media helpers, libheif, LibRaw,
# libwebp and libass for the media helper, what fetch and the deb builder use, QEMU for the
# first-boot smoke test, and Pillow and sqv for the Store's catalog
# (os/store).
RUN apt-get update && apt-get install -y --no-install-recommends \
        mmdebstrap apt-utils dpkg-dev ca-certificates curl git jq gnupg \
        xz-utils zstd file \
        e2fsprogs libarchive13t64 dosfstools mtools fdisk bmap-tools \
        make gcc gcc-14 gcc-14-for-host linux-base libc6-dev bc bison flex kmod device-tree-compiler cpp \
        binutils pahole python3 libelf1t64 libssl3t64 \
        g++ pkg-config libjpeg62-turbo-dev \
        libavformat-dev libavcodec-dev libswscale-dev libswresample-dev libavutil-dev \
        libavfilter-dev libpulse-dev \
        libheif-dev libraw-dev libwebp-dev libass-dev \
        qemu-system-arm \
        python3-pil sqv \
    && rm -rf /var/lib/apt/lists/*

# Rust for gh0st, static musl, linked by rust-lld as on the Mac.
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal \
        --default-toolchain "${RUST_VERSION}" \
        --target aarch64-unknown-linux-musl \
    && chmod -R a+rX /opt/rustup /opt/cargo

WORKDIR /work
