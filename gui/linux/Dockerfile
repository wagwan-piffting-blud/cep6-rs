# Ceps for Linux: .deb and .AppImage, built in Ubuntu 22.04 so the result runs
# on glibc 2.35 and newer (Ubuntu 22.04+, Debian 12+, Raspberry Pi OS
# bookworm+). The context is the folder above ceps-rs, so the voice folders
# the bundle ships are in reach; Dockerfile.dockerignore keeps the rest out.
#
# build_targets.ps1 runs it; by hand, from ceps:
#     docker build --platform linux/amd64 -f ceps-rs/gui/linux/Dockerfile \
#         --output type=local,dest=out .
#
# Built natively on each host (Docker Desktop for amd64, the Pi 5 for arm64):
# under qemu the release build with LTO takes hours.
#
# --build-arg FLAVOR=voiceless leaves the voices out (tauri.voiceless.json).
# BuildKit only sends the context the chosen stages COPY, so a voiceless build
# does not ship the 440 MB of voices to the builder either.

ARG FLAVOR=full

FROM ubuntu:22.04 AS base

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl wget file pkg-config \
        libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
        xdg-utils \
    && rm -rf /var/lib/apt/lists/*

RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH=/root/.cargo/bin:$PATH
RUN --mount=type=cache,target=/root/.cargo/registry \
    cargo install tauri-cli --version "^2" --locked

WORKDIR /src

FROM base AS src-full
COPY William William
COPY Allison Allison
COPY David David
COPY Jean-Pierre Jean-Pierre
COPY ceps-rs ceps-rs
ENV TAURI_EXTRA=""

FROM base AS src-voiceless
COPY ceps-rs ceps-rs
ENV TAURI_EXTRA="--config tauri.voiceless.json"

FROM src-${FLAVOR} AS build
# linuxdeploy is itself an AppImage, and a container has no FUSE to mount it;
# its strip is too old for current toolchains' sections, hence NO_STRIP. The
# bundle folder goes first: /target is a cache shared by both flavors, and a
# stale AppDir would carry one flavor's voices into the other.
ENV APPIMAGE_EXTRACT_AND_RUN=1 \
    NO_STRIP=true \
    CARGO_TARGET_DIR=/target
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/target \
    cd ceps-rs/gui \
    && rm -rf /target/release/bundle \
    && cargo tauri build --bundles deb,appimage $TAURI_EXTRA \
    && mkdir /out \
    && cp /target/release/bundle/deb/*.deb /target/release/bundle/appimage/*.AppImage /out/

FROM scratch
COPY --from=build /out/ /
