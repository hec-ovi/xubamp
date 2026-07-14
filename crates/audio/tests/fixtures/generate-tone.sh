#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
encoder=${FFMPEG:-ffmpeg}

LC_ALL=C "$encoder" \
    -hide_banner \
    -loglevel error \
    -nostdin \
    -y \
    -f lavfi \
    -i "aevalsrc=exprs='0.25*sin(2*PI*440*t)|0.25*sin(2*PI*440*t)':sample_rate=48000:duration=0.25:channel_layout=stereo" \
    -map_metadata -1 \
    -vn \
    -sn \
    -dn \
    -c:a libshine \
    -b:a 128k \
    -write_xing 0 \
    -id3v2_version 0 \
    -write_id3v1 0 \
    "$script_dir/tone.mp3"
