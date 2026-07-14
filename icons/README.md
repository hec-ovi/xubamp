# Application icon

`xubamp.svg` is original geometric artwork created for xubamp on 2026-07-14. It was drawn as SVG source in this repository without third-party artwork, fonts, trademarks, or generated bitmap inputs. The rounded player face, equalizer bars, and playback controls are generic audio interface symbols.

Copyright 2026 xubamp contributors. The SVG, render script, and generated PNG files are licensed under GPL-2.0-or-later, as stated in the repository `LICENSE`.

The PNG files are deterministic render outputs, not source artwork. Generate every checked-in size with FFmpeg built with `librsvg` support:

```sh
./icons/render-icons.sh
```

The script removes input metadata, renders RGBA output, and writes 32, 48, 64, 128, 256, and 1024 pixel square PNG files from the same SVG source.
