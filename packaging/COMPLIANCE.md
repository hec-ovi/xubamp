# Release compliance

This file records evidence available in the repository. It is a release gate, not a substitute for legal review.

## Resolved asset provenance

### Application icons

The application icon is original project artwork created as geometric SVG source on 2026-07-14. `icons/README.md` records its provenance, excludes third-party artwork, fonts, trademarks, and generated bitmap inputs, and licenses the source and render outputs under GPL-2.0-or-later. The SVG itself carries the same copyright and SPDX identifier.

`icons/render-icons.sh` generates every checked-in PNG from `icons/xubamp.svg` with FFmpeg's `librsvg` decoder. Run it from the repository root:

```sh
./icons/render-icons.sh
```

Verification used FFmpeg 8.0.1-3ubuntu2+esm1. A second invocation produced byte-identical files. `file` and `ffprobe` report the exact requested dimensions, RGBA pixel format, and non-interlaced PNG encoding for all six outputs. Alpha-channel signal analysis reports values from 0 through 255, so the transparent background is preserved. Visual inspection was performed on the 1024 and 32 pixel outputs.

SHA-256 evidence:

- `icons/xubamp.svg`: `9825d78f44ea328f6fe67415c967cebe105a8fe3ba26c4a845dd38cbb0580f55`
- `icons/README.md`: `c9e7739baebed90520597a4ee39ec539d745b85d97aa7c37fabae7b34f0fcc8f`
- `icons/render-icons.sh`: `cd09188b14d7df4475af4e0f7a847b3467f891a242dcdfa09c71d9fae9fb7a4e`
- `icons/32x32.png`: `5b75a8d2613af7ce5841b27c394b607f34a7c13d9d7b4f973d5fd670baad5345`
- `icons/48x48.png`: `ca629ebdeff716b2fd0073269366093c0494627ac9e184872c97a27f854ad567`
- `icons/64x64.png`: `a306de84a529ebd26a4541952dcb9c341dd485b3ceec177db667b6baeaa2cc16`
- `icons/128x128.png`: `f97fb72d36cc5bb2f36abff7c2c2f439b801f0e74bbd2da455ab728d539a76c7`
- `icons/256x256.png`: `57cbab091e71d1cbc15597945639066b9ebf586d0f16617809047fc386149539`
- `icons/1024x1024.png`: `2a6e795e471ed28c8decab78f846fbc91e80c5d91e667eb454ac6dcea6f36819`

### MP3 test fixture

The fixture is an original project-authored mathematical signal. `crates/audio/tests/fixtures/README.md` records the formula, parameters, provenance, and GPL-2.0-or-later license. No recorded or third-party audio is used.

`crates/audio/tests/fixtures/generate-tone.sh` evaluates `0.25 * sin(2 * PI * 440 * t)` for both stereo channels for 0.25 seconds at 48 kHz. FFmpeg's `libshine` encoder writes 128 kbit/s constant-bit-rate MPEG audio. The script removes inherited metadata and disables ID3v1, ID3v2, and Xing headers. Run it from the repository root:

```sh
./crates/audio/tests/fixtures/generate-tone.sh
```

A second invocation produced a byte-identical fixture. `file` and `ffprobe` report a 48 kHz stereo MP3 containing only MPEG audio frames, with no format tags. The encoded stream is 4,224 bytes and 0.264 seconds because MP3 uses fixed frame boundaries. `cargo test -p xubamp-audio --test decode` passes both real decoder tests, including the fixture.

SHA-256 evidence:

- `crates/audio/tests/fixtures/README.md`: `0532c002c85cb869c0bb0a148179d4bc4eac5432dc74d1d2788a6d4b5547950a`
- `crates/audio/tests/fixtures/generate-tone.sh`: `0380267eac19250eaad3d4000ad614f97ba6668746e15a67d3ef47b75016c70b`
- `crates/audio/tests/fixtures/tone.mp3`: `d1f1537999a061a7998eea57d4b5009024d8736bcba46f2f1afbec2d136a02f7`

## Third-party skins

Commit `21f01fae2c9d4a28a677e9a81d04e76918769eed` removed the seven third-party `.wsz` and `.zip` skins from the current tracked release tree. Local ignored files under `skins/` are development inputs and must never be copied into a package.

Historical Git objects containing those files are not package inputs. Build source archives from the current tracked tree, such as with `git archive`, instead of copying the working directory or the `.git` object database.

## Packager checks

- Build from a clean tracked tree and exclude `target/`, `.git/`, ignored `skins/`, and local session files.
- Include the project `LICENSE` and `THIRD_PARTY_NOTICES.md` in binary and source packages.
- Retain the exact `Cargo.lock`, update the dependency audit when it changes, vendor the corresponding Rust sources, and satisfy the Symphonia MPL-2.0 source availability notice described in `THIRD_PARTY_NOTICES.md`.
- Inspect the final binary with `readelf -d` or `ldd`, then declare its dynamically linked system libraries in package metadata.
- Regenerate the icons and MP3 fixture with their checked-in scripts when auditing a release, then compare the results with the tracked files.
