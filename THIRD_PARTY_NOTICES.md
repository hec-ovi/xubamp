# Third-party notices

This notice applies to the dependency versions recorded in `Cargo.lock`. The license terms shipped with each dependency remain authoritative. This summary does not replace those terms.

## Webamp equalizer preset data

`crates/dsp/src/presets.rs` derives its built-in preset names and values from Webamp's `packages/webamp/presets/builtin.json`. The reference file and license were verified at Webamp revision `0882aa7a312e671934d8ab04bc195f538e8c58a9`.

Webamp is licensed under the following MIT notice:

```text
The MIT License (MIT)

Copyright (c) [2015] [Jordan Eldredge]

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Reference files:

- Presets: <https://github.com/captbaritone/webamp/blob/0882aa7a312e671934d8ab04bc195f538e8c58a9/packages/webamp/presets/builtin.json>
- License: <https://github.com/captbaritone/webamp/blob/0882aa7a312e671934d8ab04bc195f538e8c58a9/LICENSE.txt>

## Symphonia

The lockfile contains these MPL-2.0 packages at version 0.5.5:

- `symphonia`
- `symphonia-bundle-mp3`
- `symphonia-codec-pcm`
- `symphonia-core`
- `symphonia-format-riff`
- `symphonia-metadata`

Their source headers carry this notice:

```text
Symphonia
Copyright (c) 2019-2022 The Project Symphonia Developers.

This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/.
```

The crates.io archives identify upstream commit `6d533f26150953a882a6a111ebd13f0abf7129d5`, which is the commit behind tag `v0.5.5`.

A distributor of an executable containing this code must make the corresponding Symphonia source available under MPL-2.0 and tell recipients how to obtain it. Notices in covered source files must be preserved. Modifications to those covered files must also be made available in source form under MPL-2.0. Include a copy of MPL-2.0 with the distribution or provide the license URL above.

Packagers can obtain the exact locked source, including all other Rust dependencies, from the repository root:

```sh
cargo vendor --locked --versioned-dirs vendor
```

Archive the six resulting `symphonia*-0.5.5` directories at a durable source URL and put that URL in the product's notice. `Cargo.lock` contains the crates.io checksums used to verify the archives. The upstream source is also available at <https://github.com/pdeljanov/Symphonia/tree/v0.5.5>.

## Locked Rust dependency license families

`cargo metadata --locked --all-features` currently resolves 115 registry packages across runtime, build, optional, and target-specific dependency edges. Their manifests declare these license families: MIT, Apache-2.0, BSD-3-Clause, ISC, MPL-2.0, Unicode-3.0, Zlib, 0BSD, Unlicense, and Apache-2.0 with LLVM-exception. Most packages offer MIT or Apache-2.0 as alternatives. Cases that need separate attention are:

| License case | Locked packages |
| --- | --- |
| MPL-2.0 | The six Symphonia 0.5.5 packages listed above |
| BSD-3-Clause only | `bindgen 0.72.1` |
| Apache-2.0 only | `clang-sys 1.8.1` |
| ISC only | `libloading 0.8.9` |
| Apache-2.0 WITH LLVM-exception only | `target-lexicon 0.13.5` |
| Additional BSD-3-Clause term | `encoding_rs 0.8.35` declares `(Apache-2.0 OR MIT) AND BSD-3-Clause` |
| Additional Unicode-3.0 term | `unicode-ident 1.0.24` declares `(MIT OR Apache-2.0) AND Unicode-3.0` |
| LLVM exception offered with permissive alternatives | `linux-raw-sys 0.4.15`, `linux-raw-sys 0.12.1`, `rustix 0.38.44`, `rustix 1.1.4` |
| Zlib offered as an alternative | `bytemuck 1.25.1`, `bytemuck_derive 1.11.0`, `cursor-icon 1.2.0`, `miniz_oxide 0.8.9`, `xkeysym 0.2.1` |
| 0BSD offered as an alternative | `adler2 2.0.1` |
| Unlicense offered as an alternative | `aho-corasick 1.1.4`, `memchr 2.8.3` |

This table is a compact audit, not a package-by-package license bundle. Binary and source packagers should ship the applicable license files from the vendored crates, or obtain them from the authoritative upstream when a crate archive omits them, and retain the exact `Cargo.lock` used for the build. Refresh this audit whenever `Cargo.lock` changes.

## System libraries

System libraries are not Rust crate source and are not copied by `cargo vendor`:

- The normal Wayland path loads `libwayland-client.so.0` at runtime through `dlopen`.
- The `audio` feature dynamically links the system `libpipewire-0.3` library. PipeWire's SPA headers and package metadata are also build inputs, and PipeWire loads its system SPA modules at runtime.
- The `keyboard` feature dynamically links `libxkbcommon`.
- `pkg-config`, development headers, and `libclang` are build inputs for feature-enabled builds. They are not application runtime libraries.
- The platform C runtime, dynamic loader, `libm`, and Rust's compiler runtime dependencies come from the target system toolchain.

Packagers must declare the system runtime dependencies actually reported by the final release binary and comply with the distribution's corresponding system-package notices. Do not represent these libraries as bundled Rust dependencies.
