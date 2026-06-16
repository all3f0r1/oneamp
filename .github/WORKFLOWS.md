# GitHub Actions Workflows

This document describes the CI/CD workflows configured for OneAmp.

## Workflows

### Release Workflow (`release.yml`)

**Trigger:** Push of a tag matching `v*` (e.g., `v0.17.1`)

**Jobs:**

1. **Test Suite** — runs all unit tests and doc tests on the tagged commit.
2. **Rustfmt** — checks code formatting.
3. **Clippy** — runs the linter with `-D warnings`.
4. **Build Linux x86_64** — depends on test/fmt/clippy. Builds the release binary, strips it, produces a tarball + `.deb` + AppImage, and creates the GitHub Release.
5. **Build RPM (Fedora)** — depends on build-linux. Produces an `.rpm` and appends it to the release.
6. **Build Flatpak** — depends on build-linux. Produces a `.flatpak` bundle and appends it to the release.
7. **Create Release Notes** — generates the release body with download links and install instructions, runs as long as build-linux succeeded.

---

## Creating a Release

To create a new release:

1. **Update version numbers:**
   - Update `Cargo.toml` workspace version
   - Update `CHANGELOG.md` with new features

2. **Commit and push:**
   ```bash
   git add Cargo.toml CHANGELOG.md
   git commit -m "chore(release): vX.Y.Z"
   git push origin master
   ```

3. **Create tag:**
   ```bash
   git tag -a vX.Y.Z -m "Release vX.Y.Z"
   git push origin vX.Y.Z
   ```

4. **GitHub Actions will automatically:**
   - Run all checks (test, fmt, clippy)
   - Build Linux/RPM/Flatpak/AppImage artifacts
   - Create the GitHub Release with all artifacts attached

5. **Download release:**
   - Visit https://github.com/all3f0r1/oneamp/releases

---

## Release Artifacts

- `oneamp-vX.Y.Z-linux-x86_64.tar.gz` — portable tarball with binary, README, CHANGELOG
- `oneamp-vX.Y.Z-linux-x86_64.deb` — Debian/Ubuntu package
- `oneamp-vX.Y.Z-x86_64.rpm` — Fedora/openSUSE package
- `OneAmp-vX.Y.Z-x86_64.flatpak` — Flatpak bundle
- `OneAmp-vX.Y.Z-x86_64.AppImage` — portable AppImage

---

## Environment Variables

The workflow uses:
- `CARGO_TERM_COLOR: always` — colored output
- `RUST_BACKTRACE: 1` — better error messages

---

## Caching Strategy

The workflow caches:
- Cargo registry (`~/.cargo/registry`)
- Cargo git index (`~/.cargo/git`)
- Build artifacts (`target/`)

Cache keys include the `Cargo.lock` hash to invalidate on dependency changes.

---

## Dependencies

### Build Dependencies
- Rust stable toolchain
- `rustfmt` and `clippy` components

### System Dependencies (Linux)
- `libasound2-dev` (ALSA)
- `libxcb-render0-dev`, `libxcb-shape0-dev`, `libxcb-xfixes0-dev`
- `libxkbcommon-dev`
- `libwayland-dev` (build-linux only)

---

## Troubleshooting

### Tests Failing
1. Check the job log on GitHub Actions
2. Run locally: `cargo test --all`
3. Fix and push a new tag

### Rustfmt Failing
1. Run locally: `cargo fmt --all`
2. Commit and push a new tag

### Clippy Failing
1. Run locally: `cargo clippy --all --all-targets --all-features -- -D warnings`
2. Fix and push a new tag

### Build Failing
1. Check the build log on GitHub Actions
2. Run locally: `cargo build --release`
3. Fix and push a new tag

---

## References

- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [Rust GitHub Actions](https://github.com/dtolnay/rust-toolchain)
- [softprops/action-gh-release](https://github.com/softprops/action-gh-release)
