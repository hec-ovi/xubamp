#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
encoder=${FFMPEG:-ffmpeg}

# 0.1 s of a 440 Hz sine followed by 0.3 s of exact digital silence. libshine encodes the silent
# part as granules with part2_3_length == 0, which is the case symphonia's Layer III decoder
# rejects (see vendor/symphonia-bundle-mp3/PATCHES.md).
LC_ALL=C "$encoder" \
    -hide_banner \
    -loglevel error \
    -nostdin \
    -y \
    -f lavfi \
    -i "aevalsrc=exprs='0.25*sin(2*PI*440*t)*lt(t,0.1)|0.25*sin(2*PI*440*t)*lt(t,0.1)':sample_rate=44100:duration=0.4:channel_layout=stereo" \
    -map_metadata -1 \
    -vn \
    -sn \
    -dn \
    -c:a libshine \
    -b:a 128k \
    -write_xing 0 \
    -id3v2_version 0 \
    -write_id3v1 0 \
    "$script_dir/silence.mp3"
