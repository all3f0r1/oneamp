# GitHub Actions Workflows

This document describes the CI/CD workflows configured for OneAmp.

## Workflows

### 1. CI Workflow (`ci.yml`)

**Trigger:** Push to `master`, `main`, or `develop` branches, or pull requests to these branches

**Jobs:**

1. **Test Suite**
   - Runs all unit tests and doc tests
   - Caches dependencies for faster builds
   - Ensures code correctness

2. **Rustfmt**
   - Checks code formatting
   - Fails if code is not properly formatted
   - Ensures consistent code style

3. **Clippy**
   - Runs Rust linter
   - Fails on warnings (strict mode)
   - Catches common mistakes and performance issues

4. **Build**
   - Builds release binary
   - Depends on all previous jobs passing
   - Uploads artifacts for 7 days

**Status Checks:**
- All checks must pass before merging PRs
- Provides immediate feedback on code quality

---

### 2. Release Workflow (`release.yml`)

**Trigger:** Push of a tag matching `v*` (e.g., `v0.14.1`)

**Jobs:**

1. **Test Suite**
   - Same as CI workflow
   - Ensures release code is tested

2. **Rustfmt**
   - Same as CI workflow
   - Ensures release code is properly formatted

3. **Clippy**
   - Same as CI workflow
   - Ensures release code passes linting

4. **Build Linux**
   - Builds optimized release binary for Linux x86_64
   - Installs required dependencies
   - Strips binary to reduce size
   - Creates tarball with binary, README, and CHANGELOG
   - Uploads to GitHub Releases

5. **Create Release Notes**
   - Generates release notes
   - Includes download links
   - Provides installation instructions

---

## Creating a Release

To create a new release:

1. **Update version numbers:**
   - Update `Cargo.toml` version in all packages
   - Update `CHANGELOG.md` with new features

2. **Commit changes:**
   ```bash
   git add Cargo.toml CHANGELOG.md
   git commit -m "chore: Bump version to v0.14.1"
   git push origin master
   ```

3. **Create tag:**
   ```bash
   git tag -a v0.14.1 -m "Release v0.14.1"
   git push origin v0.14.1
   ```

4. **GitHub Actions will automatically:**
   - Run all tests
   - Run rustfmt check
   - Run clippy check
   - Build release binary
   - Create GitHub Release with artifacts
   - Generate release notes

5. **Download release:**
   - Visit https://github.com/all3f0r1/oneamp/releases
   - Download `oneamp-v0.14.1-linux-x86_64.tar.gz`

---

## Artifacts

### CI Artifacts
- **Name:** `oneamp-linux-x86_64`
- **Location:** `target/release/oneamp`
- **Retention:** 7 days
- **Purpose:** Quick testing of builds

### Release Artifacts
- **Name:** `oneamp-v0.14.1-linux-x86_64.tar.gz`
- **Contents:**
  - `oneamp` binary
  - `README.md`
  - `CHANGELOG.md`
- **Purpose:** Distribution to users

---

## Environment Variables

All workflows use:
- `CARGO_TERM_COLOR: always` - Colored output
- `RUST_BACKTRACE: 1` - Better error messages

---

## Caching Strategy

Workflows cache:
- Cargo registry (`~/.cargo/registry`)
- Cargo git index (`~/.cargo/git`)
- Build artifacts (`target/`)

Cache keys include `Cargo.lock` hash to invalidate on dependency changes.

---

## Dependencies

### Build Dependencies
- Rust stable toolchain
- `rustfmt` component (for formatting)
- `clippy` component (for linting)

### System Dependencies (Linux)
- `libxcb-render0-dev`
- `libxcb-shape0-dev`
- `libxcb-xfixes0-dev`
- `libxkbcommon-dev`

---

## Troubleshooting

### Tests Failing
1. Check test output in GitHub Actions
2. Run locally: `cargo test --all`
3. Fix issues and push again

### Rustfmt Failing
1. Run locally: `cargo fmt --all`
2. Commit formatted code
3. Push again

### Clippy Failing
1. Run locally: `cargo clippy --all --all-targets --all-features`
2. Fix warnings
3. Push again

### Build Failing
1. Check build output in GitHub Actions
2. Run locally: `cargo build --release`
3. Fix issues and push again

---

## Future Enhancements

- [ ] Add Windows build
- [ ] Add macOS build
- [ ] Add code coverage reporting
- [ ] Add performance benchmarks
- [ ] Add security scanning
- [ ] Add dependency updates via Dependabot

---

## References

- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [Rust GitHub Actions](https://github.com/dtolnay/rust-toolchain)
- [softprops/action-gh-release](https://github.com/softprops/action-gh-release)
