# Local patch to symphonia-bundle-mp3 0.5.5

This is the crates.io source of `symphonia-bundle-mp3` 0.5.5 (MPL-2.0, upstream
<https://github.com/pdeljanov/Symphonia>) with one change, applied through
`[patch.crates-io]` in the workspace `Cargo.toml`. MPL-2.0 requires modified covered files to
be available in source form, which is what this directory is.

## The change

`src/layer3/mod.rs`, in `Layer3::read_main_data`: skip a granule channel whose
`part2_3_length` is 0 instead of trying to read it.

An empty granule channel carries no scale factors, no Huffman-coded samples, and consumes no
main data. Encoders emit one whenever a channel is digitally silent, which happens at track
intros, fade-outs, and gaps. Upstream reads it anyway, and that fails two ways:

- `read_scale_factors_mpeg1` returns a non-zero `part2_len` against a `part2_3_length` of 0, so
  the `part2_len > part2_3_length` check rejects the frame ("mpa: part2_3_length is not valid").
- When the previous granule ended exactly on the last byte of main data, `byte_index` equals
  `main_data.len()`, which is a legitimate position for a zero-length read, and the
  `byte_index < main_data.len()` guard rejects the frame ("mpa: invalid main_data offset").

Either error clears the bit reservoir, so the next frame that reuses the reservoir fails too,
and the failure cascades to the end of the track. In a 5720-file library this dropped audio
from 137 files: the whole tail of several tracks (4 to 5 seconds), and up to 31 percent of one
of them.

With the patch, output matches `ffmpeg -f s16le` sample for sample (max difference 1 LSB of
32768, the expected float-to-integer rounding) over a full track.

## Regenerating

Fetch the pristine 0.5.5 source, re-apply the hunk marked `xubamp patch` in
`src/layer3/mod.rs`, and re-run `cargo test -p xubamp-audio`:

    cargo fetch
    diff -ru "$(dirname "$(cargo info symphonia-bundle-mp3 2>/dev/null)")" vendor/symphonia-bundle-mp3

`Cargo.toml` here is the registry-normalized manifest with `Cargo.lock`, `Cargo.toml.orig`, and
the extraction stamps removed. `LICENSE` is the MPL-2.0 text the upstream README refers to;
upstream does not package it in the crate.

## Upstream

Worth reporting; the fix is small and self-contained. Nothing in this directory diverges from
upstream apart from the one hunk above, so dropping the patch is a matter of deleting this
directory and the `[patch.crates-io]` section once a release carries the fix.
