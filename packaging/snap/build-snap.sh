#!/usr/bin/env bash
# Build the snap by repacking the released deb. Run packaging/build-deb-container.sh
# first so packaging/dist holds the deb matching snapcraft.yaml's version line.
set -euo pipefail
cd "$(dirname "$0")"

version=$(sed -n "s/^version: '\(.*\)'$/\1/p" snapcraft.yaml)
deb="../dist/xubamp_${version}-1_amd64.deb"
if [ ! -f "$deb" ]; then
    echo "missing $deb; build the deb first (packaging/build-deb-container.sh)" >&2
    exit 1
fi

rm -rf deb-root
dpkg-deb -x "$deb" deb-root

# Destructive mode builds directly on this host, which matches base core26
# (Ubuntu 26.04); the dump plugin only unpacks files, so nothing is installed
# on the host beyond snapcraft's own stage-package downloads.
snapcraft pack --destructive-mode --output "xubamp_${version}_amd64.snap"
