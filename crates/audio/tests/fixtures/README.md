# MP3 decode fixtures

Both files are original mathematical test signals created for xubamp. Their source is the formula `0.25 * sin(2 * PI * 440 * t)`, evaluated independently for the left and right channels.

`tone.mp3` (2026-07-14) has a requested duration of 0.25 seconds at 48 kHz and is encoded as a 128 kbit/s constant-bit-rate MP3. MP3 frame boundaries make the encoded stream 0.264 seconds long.

`silence.mp3` (2026-07-25) is 0.4 seconds at 44.1 kHz, 128 kbit/s constant bit rate, with the tone gated off after the first 0.1 seconds so the rest is exact digital silence. The encoder writes that stretch as granules with `part2_3_length == 0`, which is the case the vendored Symphonia patch fixes (see `vendor/symphonia-bundle-mp3/PATCHES.md`); against stock Symphonia the file decodes to its first 0.13 seconds and stops.

Copyright 2026 xubamp contributors. The generators and generated fixtures are licensed under GPL-2.0-or-later, as stated in the repository `LICENSE`. No recorded or third-party audio is used.

Regenerate them with FFmpeg built with the `libshine` encoder:

```sh
./crates/audio/tests/fixtures/generate-tone.sh
./crates/audio/tests/fixtures/generate-silence.sh
```

Both commands disable ID3v1, ID3v2, and Xing headers and remove inherited metadata. The resulting files contain only MPEG audio frames.
