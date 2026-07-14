#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
renderer=${FFMPEG:-ffmpeg}

for size in 32 48 64 128 256 1024; do
    "$renderer" \
        -hide_banner \
        -loglevel error \
        -nostdin \
        -y \
        -i "$script_dir/xubamp.svg" \
        -vf "scale=${size}:${size}:flags=lanczos,format=rgba" \
        -frames:v 1 \
        -map_metadata -1 \
        -c:v png \
        -compression_level 9 \
        -pred mixed \
        "$script_dir/${size}x${size}.png"
done
