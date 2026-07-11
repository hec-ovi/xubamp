#!/usr/bin/env bash
# Build and run xubamp inside the Ubuntu 26.04 dev container so the PipeWire build deps stay
# off the host. The window still appears on your GNOME and audio plays through your host
# PipeWire: the Wayland and PipeWire sockets under $XDG_RUNTIME_DIR are mounted, and the
# container user is uid 1000 to match their ownership.
#
# Usage:
#   scripts/dev-docker.sh image                 # build the dev image (run once, or after
#                                               # editing Dockerfile.dev)
#   scripts/dev-docker.sh build [cargo args]    # e.g. build --workspace
#   scripts/dev-docker.sh test  [cargo args]    # e.g. test -p xubamp-audio
#   scripts/dev-docker.sh run   [xubamp args]   # cargo run -p xubamp -- <args>
#   scripts/dev-docker.sh shell                 # interactive shell in the container
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
img="xubamp-dev"
runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

mkdir -p "$root/.docker/cargo-registry" "$root/.docker/target"

# Allocate a TTY only when stdin is one, so automated (non-interactive) runs still work.
if [ -t 0 ]; then tty=(-it); else tty=(-i); fi

# Keep container build artifacts out of the host target/ (different glibc/paths).
common=(
    "${tty[@]}" --rm
    -v "$root:/work" -w /work
    -v "$root/.docker/cargo-registry:/home/dev/.cargo/registry"
    -v "$root/.docker/target:/target" -e CARGO_TARGET_DIR=/target
)

# Extra mounts so a running xubamp can reach the compositor and audio server.
session=(
    -v "$runtime:$runtime" --ipc=host
    -e "XDG_RUNTIME_DIR=$runtime"
    -e "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0}"
)

cmd="${1:-}"
[ $# -gt 0 ] && shift || true
case "$cmd" in
    image) docker build -f "$root/Dockerfile.dev" --build-arg UID="$(id -u)" -t "$img" "$root" ;;
    build) docker run "${common[@]}" "$img" cargo build "$@" ;;
    test) docker run "${common[@]}" "$img" cargo test "$@" ;;
    run) docker run "${common[@]}" "${session[@]}" "$img" cargo run -p xubamp -- "$@" ;;
    shell) docker run "${common[@]}" "${session[@]}" "$img" bash ;;
    *)
        echo "usage: $0 {image|build|test|run|shell} [args]" >&2
        exit 1
        ;;
esac
