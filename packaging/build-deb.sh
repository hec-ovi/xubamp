#!/usr/bin/env bash
# Build a reproducible xubamp binary package for the current Ubuntu architecture.
set -euo pipefail

umask 022
export LC_ALL=C.UTF-8
export TZ=UTC

root="$(cd "$(dirname "$0")/.." && pwd)"
build="$root/packaging/build"
dist="$root/packaging/dist"

for command in appstreamcli cargo desktop-file-validate dpkg dpkg-deb \
    dpkg-shlibdeps gzip python3 rustc strip; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "xubamp package: required command not found: $command" >&2
        exit 1
    fi
done

rust_version="$(rustc --version | awk '{print $2}')"
IFS=. read -r rust_major rust_minor _ <<<"$rust_version"
if [ "$rust_major" -lt 1 ] || { [ "$rust_major" -eq 1 ] && [ "$rust_minor" -lt 96 ]; }; then
    echo "xubamp package: Rust 1.96 or newer is required, found $rust_version" >&2
    exit 1
fi

if git -C "$root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    if [ "${XUBAMP_ALLOW_DIRTY:-0}" != 1 ] && \
        { ! git -C "$root" diff --quiet || \
          ! git -C "$root" diff --cached --quiet || \
          [ -n "$(git -C "$root" ls-files --others --exclude-standard)" ]; }; then
        echo "xubamp package: tracked source must be clean (set XUBAMP_ALLOW_DIRTY=1 for a local test)" >&2
        exit 1
    fi
    source_commit="${XUBAMP_SOURCE_COMMIT:-$(git -C "$root" rev-parse HEAD)}"
    source_epoch="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}"
else
    source_commit="${XUBAMP_SOURCE_COMMIT:?set XUBAMP_SOURCE_COMMIT outside a Git checkout}"
    source_epoch="${SOURCE_DATE_EPOCH:?set SOURCE_DATE_EPOCH outside a Git checkout}"
fi
export SOURCE_DATE_EPOCH="$source_epoch"

mkdir -p "$build" "$dist"
export CARGO_HOME="${CARGO_HOME:-$build/cargo-home}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$build/target}"
host_target="$(rustc -vV | sed -n 's/^host: //p')"
remap="--remap-path-prefix=$root=/usr/src/xubamp"
if [ -n "${RUSTFLAGS:-}" ]; then
    export RUSTFLAGS="$RUSTFLAGS $remap"
else
    export RUSTFLAGS="$remap"
fi

cd "$root"
metadata="$(cargo metadata --locked --no-deps --format-version 1)"
version="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(next(p["version"] for p in d["packages"] if p["name"] == "xubamp"))' <<<"$metadata")"
revision="${XUBAMP_DEBIAN_REVISION:-1}"
package_version="$version-$revision"
architecture="$(dpkg --print-architecture)"
dpkg --validate-version "$package_version"
appstream_version="$(python3 -c 'import sys,xml.etree.ElementTree as E; print(E.parse(sys.argv[1]).find("./releases/release").attrib["version"])' packaging/io.github.hec_ovi.xubamp.metainfo.xml)"
if [ "$appstream_version" != "$version" ]; then
    echo "xubamp package: AppStream version $appstream_version does not match $version" >&2
    exit 1
fi

cargo build --locked --release -p xubamp --features audio,keyboard
binary="$CARGO_TARGET_DIR/release/xubamp"
if [ ! -x "$binary" ]; then
    echo "xubamp package: release binary was not produced" >&2
    exit 1
fi

work="$build/package-$architecture"
stage="$work/debian/xubamp"
rm -rf "$work"
mkdir -p "$stage/DEBIAN"

install -Dm755 "$binary" "$stage/usr/bin/xubamp"
strip --remove-section=.comment "$stage/usr/bin/xubamp"
install -Dm644 packaging/xubamp.desktop \
    "$stage/usr/share/applications/xubamp.desktop"
install -Dm644 packaging/io.github.hec_ovi.xubamp.metainfo.xml \
    "$stage/usr/share/metainfo/io.github.hec_ovi.xubamp.metainfo.xml"
install -Dm644 icons/xubamp.svg \
    "$stage/usr/share/icons/hicolor/scalable/apps/xubamp.svg"
for icon in icons/*x*.png; do
    size="$(basename "$icon" .png)"
    case "$size" in
        [0-9]*x[0-9]*)
            install -Dm644 "$icon" \
                "$stage/usr/share/icons/hicolor/$size/apps/xubamp.png"
            ;;
        *)
            echo "xubamp package: invalid icon filename: $icon" >&2
            exit 1
            ;;
    esac
done

doc="$stage/usr/share/doc/xubamp"
install -Dm644 LICENSE "$doc/LICENSE"
install -Dm644 THIRD_PARTY_NOTICES.md "$doc/THIRD_PARTY_NOTICES.md"
install -Dm644 packaging/COMPLIANCE.md "$doc/COMPLIANCE.md"
install -Dm644 Cargo.lock "$doc/Cargo.lock"
install -Dm644 packaging/debian/copyright "$doc/copyright"
changelog_version="$(sed -n '1s/^xubamp (\([^)]*\)).*/\1/p' packaging/debian/changelog)"
if [ "$changelog_version" != "$package_version" ]; then
    echo "xubamp package: changelog version $changelog_version does not match $package_version" >&2
    exit 1
fi
gzip -n -9 -c packaging/debian/changelog >"$doc/changelog.Debian.gz"
chmod 0644 "$doc/changelog.Debian.gz"
python3 packaging/collect-cargo-licenses.py \
    --target "$host_target" --output "$doc/third-party-rust" --lockfile Cargo.lock \
    --fallback-dir packaging/licenses
cat >"$doc/BUILD_INFO" <<EOF
Source: https://github.com/hec-ovi/xubamp
Commit: $source_commit
Cargo.lock SHA-256: $(sha256sum Cargo.lock | awk '{print $1}')
Rust: $(rustc --version)
Cargo: $(cargo --version)
Target: $host_target
Features: audio,keyboard
EOF
chmod 0644 "$doc/BUILD_INFO"

mkdir -p "$stage/usr/share/man/man1"
gzip -n -9 -c packaging/xubamp.1 >"$stage/usr/share/man/man1/xubamp.1.gz"
chmod 0644 "$stage/usr/share/man/man1/xubamp.1.gz"

desktop-file-validate packaging/xubamp.desktop
appstreamcli validate --strict --no-net packaging/io.github.hec_ovi.xubamp.metainfo.xml
if command -v groff >/dev/null 2>&1; then
    groff -man -z packaging/xubamp.1
fi

mkdir -p "$work/debian"
install -m644 packaging/debian/shlibdeps-control "$work/debian/control"
shlibs="$(cd "$work" && dpkg-shlibdeps -O -e"$stage/usr/bin/xubamp")"
shlibs="${shlibs#shlibs:Depends=}"
depends="$shlibs, libpipewire-0.3-common"
installed_size="$(find "$stage/usr" -printf '%y\t%s\n' | awk -F '\t' '
    $1 == "f" || $1 == "l" {
        size += int(($2 + 1023) / 1024)
        next
    }
    { size++ }
    END { print size }
')"

cat >"$stage/DEBIAN/control" <<EOF
Package: xubamp
Version: $package_version
Section: sound
Priority: optional
Architecture: $architecture
Maintainer: Hector Oviedo <hector.ernesto.oviedo@gmail.com>
Installed-Size: $installed_size
Depends: $depends
Recommends: xdg-desktop-portal-gtk | xdg-desktop-portal-backend
Homepage: https://github.com/hec-ovi/xubamp
Description: native Wayland audio player with classic skin support
 xubamp decodes MP3 and WAV audio, sends it to PipeWire, and renders
 classic Winamp skin archives on Wayland.
EOF

(
    cd "$stage"
    find usr -type f -print0 | sort -z | xargs -0 md5sum >DEBIAN/md5sums
)
find "$stage" -print0 | xargs -0 touch -h -d "@$SOURCE_DATE_EPOCH"
output="$dist/xubamp_${package_version}_${architecture}.deb"
dpkg-deb --root-owner-group --threads-max=1 --uniform-compression \
    -Zxz -z9 --build "$stage" "$output"

contents="$(dpkg-deb --contents "$output")"
if grep -Eiq '/(skins?|video|cd-rip|plugins?|setup|session|\.research|fixtures?)(/|$)|\.(wsz|zip)$' <<<"$contents"; then
    echo "xubamp package: forbidden release asset found in archive" >&2
    grep -Ei '/(skins?|video|cd-rip|plugins?|setup|session|\.research|fixtures?)(/|$)|\.(wsz|zip)$' <<<"$contents" >&2
    exit 1
fi

printf '%s  %s\n' "$(sha256sum "$output" | awk '{print $1}')" "$output"
dpkg-deb --info "$output"
