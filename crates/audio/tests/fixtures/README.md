# MP3 decode fixture

`tone.mp3` is an original mathematical test signal created for xubamp on 2026-07-14. Its source is the formula `0.25 * sin(2 * PI * 440 * t)`, evaluated independently for the left and right channels. It has a requested duration of 0.25 seconds at 48 kHz and is encoded as a 128 kbit/s constant-bit-rate MP3. MP3 frame boundaries make the encoded stream 0.264 seconds long.

Copyright 2026 xubamp contributors. The generator and generated fixture are licensed under GPL-2.0-or-later, as stated in the repository `LICENSE`. No recorded or third-party audio is used.

Regenerate the fixture with FFmpeg built with the `libshine` encoder:

```sh
./crates/audio/tests/fixtures/generate-tone.sh
```

The command disables ID3v1, ID3v2, and Xing headers and removes inherited metadata. The resulting file contains only MPEG audio frames.
