#!/usr/bin/env bash
# Build the Ubuntu 26.04 package without installing build dependencies on the host.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
image="xubamp-deb-builder"
cache="$root/.docker/deb-package"
mkdir -p "$cache/cargo" "$cache/target"

docker build \
    --file "$root/packaging/Dockerfile.deb" \
    --build-arg "UID=$(id -u)" \
    --tag "$image" \
    "$root"

docker run --rm \
    --volume "$root:/work" \
    --volume "$cache/cargo:/cache/cargo" \
    --volume "$cache/target:/cache/target" \
    --env CARGO_HOME=/cache/cargo \
    --env CARGO_TARGET_DIR=/cache/target \
    --env SOURCE_DATE_EPOCH \
    --env XUBAMP_ALLOW_DIRTY \
    --env XUBAMP_DEBIAN_REVISION \
    --env XUBAMP_SOURCE_COMMIT \
    --workdir /work \
    "$image"
