# Distribution channels

Status and requirements per channel, researched July 2026. Four channels total: the GitHub Releases deb, apt via a Launchpad PPA, Flatpak via Flathub, and Snap via the Snap Store.

## 1. GitHub Releases deb (live)

Every release tags `vX.Y.Z` and attaches `xubamp_X.Y.Z-1_amd64.deb` at
https://github.com/hec-ovi/xubamp/releases. Install with `sudo apt install ./xubamp_*_amd64.deb`.

About "published packages" on GitHub: the repo Packages sidebar (GitHub Packages) supports npm, Docker/OCI, Maven, NuGet, and RubyGems, but has no apt/deb registry; that remains an open feature request ([discussion](https://github.com/orgs/community/discussions/56445)). The Releases page is GitHub's deb channel. Optional extra reach: [deb-get](https://github.com/wimpysworld/deb-get) installs and updates debs straight from GitHub Releases; adding xubamp there is one small PR to their repo.

## 2. apt via Launchpad PPA (blocked on registration)

A PPA gives real `apt install xubamp` with updates. Launchpad builds from a signed
source package on offline builders, so every crate must be vendored into the source
tarball (`cargo vendor` plus a `debian/rules` that builds with `--offline`).

Owner registration (one time):
1. Create a Launchpad account: https://launchpad.net/+login
2. Sign the Ubuntu Code of Conduct (required to activate a PPA).
3. Create a GPG key, publish it to `keyserver.ubuntu.com`, and add it to the account.
4. Create the PPA (suggested name `xubamp`), giving `ppa:<launchpad-id>/xubamp`.

After that exists, the missing repo piece is a `packaging/build-ppa-source.sh` that
produces the vendored, signed source package for the `resolute` series and uploads
with `dput`. Users then run:

    sudo add-apt-repository ppa:<launchpad-id>/xubamp
    sudo apt install xubamp

## 3. Flatpak via Flathub (draft manifest committed)

Draft at [packaging/flatpak/io.github.hec_ovi.xubamp.yml](../packaging/flatpak/io.github.hec_ovi.xubamp.yml). Current targets: runtime `org.freedesktop.Platform` branch 25.08 (the recommended branch for new apps, two year support, new branch every August) with the `rust-stable` SDK extension. Builders are offline, so Cargo sources come from a pregenerated `generated-sources.json` ([flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools)).

Submission is a pull request against the `new-pr` branch of [flathub/flathub](https://github.com/flathub/flathub/wiki/App-Submission), manifest named after the app ID (`io.github.hec_ovi.xubamp`, verifiable through the GitHub account, no domain needed).

Important: [Flathub's submission policy](https://docs.flathub.org/docs/for-app-authors/submission) forbids pull requests generated, opened, or automated by AI tools. Review the draft, build and test it locally (commands are in the manifest header), adjust it to taste, and open the PR yourself.

## 4. Snap via the Snap Store (draft recipe committed)

Draft at [packaging/snap/snapcraft.yaml](../packaging/snap/snapcraft.yaml). Targets base `core26` (built from Ubuntu 26.04 LTS, matching the app's only supported platform), strict confinement, `wayland` plus `audio-playback` plugs ([interface reference](https://snapcraft.io/docs/reference/interfaces/audio-playback-interface/); auto-connects). The snap bundles the PipeWire client libraries it links against.

Owner registration (one time):
1. Ubuntu One account: https://login.ubuntu.com
2. `snap install snapcraft --classic`, then `snapcraft login`.
3. `snapcraft register xubamp` (name grants are first come, first served).

Then build, test, and upload with the commands in the recipe header. The first
strictly confined upload is usually auto-approved; only classic confinement or
special interfaces need manual review.

## Suggested order

Releases deb is live. Snap is the shortest path (registration is minutes, core26
matches the target platform exactly). Flathub reaches the widest audience but the
PR must be authored personally per their AI policy. The PPA is the most work
(crate vendoring) and lands last unless apt distribution is a priority.
