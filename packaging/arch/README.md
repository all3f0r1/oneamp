# OneAmp on Arch Linux

OneAmp does not ship a prebuilt binary for Arch — Arch is rolling, so a
source PKGBUILD picks up whatever Rust toolchain and libc version the
user already has installed.

## Install from this repository

```bash
git clone https://github.com/all3f0r1/oneamp.git
cd oneamp/packaging/arch
makepkg -si
```

`makepkg` will:
- Download the tagged release tarball from GitHub.
- Fetch all Cargo dependencies into `$srcdir/cargo-home` (locked).
- Build `oneamp-desktop` with the workspace release profile.
- Install the binary to `/usr/bin/oneamp`, the desktop file under
  `/usr/share/applications/`, icons under `/usr/share/icons/hicolor/`,
  AppStream metainfo under `/usr/share/metainfo/`.

## Updating

Bump `pkgver` in `PKGBUILD` to match the upstream tag (e.g. `0.22.0`),
optionally replace `sha256sums=('SKIP')` with the real hash from the
GitHub release page, then `makepkg -si` again.

## Future: AUR publication

This recipe is suitable as-is for publication on the AUR under the
name `oneamp`. If/when someone volunteers to maintain that submission,
they should:

1. Replace the placeholder `Maintainer:` line with their own email.
2. Pin a real `sha256sums` hash per release.
3. Use [aurpublish](https://github.com/eli-schwartz/aurpublish) (or
   manual `git push aur:oneamp`) to upload from this directory.

No CI job covers Arch — the rolling-release model means
`ubuntu-latest`'s frozen Cargo registry would not predict a real Arch
build's outcome.
