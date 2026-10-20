# Windows release process

The semantic application version has one source: `package.version` in
`Cargo.toml`. The build script embeds the Cargo version, short Git commit,
dirty-source marker, build channel, target triple, and build profile. Use a
clean source tag `v<version>` to create a stable release.

## Prepare a release

1. Update `Cargo.toml` and `Cargo.lock` only when a version/dependency change
   requires it. Do not maintain a second version constant.
2. Create `release-notes/v<version>.md` from `release-notes/TEMPLATE.md` and
   write human-reviewed changes, compatibility notes, and known issues.
   Verify distribution rights and attribution for every packaged asset under
   `assets/`; Cargo notices do not cover that media pack.
3. Run `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets`,
   `cargo test --locked --no-fail-fast`, and a release build. Complete the
   representative real-engine regression and GUI checks that are available;
   record fixture and environment limitations explicitly.
4. Create a clean annotated Git tag exactly matching `v<version>` and push it.
   `.github/workflows/release-windows.yml` rejects a dirty tree, mismatched
   version, missing notes, and test/build failures.

The workflow builds the same `renderer` binary as development, stages only the
application, shipped asset/profile directories, notices, and user guides,
creates an x86_64 Windows per-user Inno Setup installer plus a portable ZIP,
and writes SHA-256 checksums and a JSON release manifest. It tests portable
launch from a directory containing spaces, with a different current working
directory and a restricted `PATH`; then it installs, runs CLI commands,
reinstalls, uninstalls, and checks that app data and an external Unicode-path
project survive. GitHub build-provenance attestations are generated before
creating a **draft** GitHub release. The draft must be reviewed before it is
made public, including asset redistribution rights and Windows Authenticode
signing status.

## Release contents and limits

- `Sono-Renderer-<version>-windows-x86_64-setup.exe`
- `Sono-Renderer-<version>-windows-x86_64-portable.zip`
- `SHA256SUMS.txt`
- `Sono-Renderer-<version>-release-manifest.json`
- The tag-specific release notes

Only Windows x86_64 is packaged. Linux and macOS remain unverified. Release
executables are unsigned; configure and verify Authenticode signing before
claiming an official signed installer. Do not publish an artifact until its
tag, manifest, hashes, provenance attestation, dependency notices, and manual
GUI acceptance results have been reviewed.

## Maintainer checklist

- [ ] Set the Cargo package version and prepare `release-notes/v<version>.md`.
- [ ] Verify the packaged assets' redistribution rights and attribution.
- [ ] Run formatting, Clippy, the full tests, and the optimized test suite.
- [ ] Run real-engine render/VM-GCC checks and review visual output.
- [ ] Validate the default Quick Render workflow, Program Preview, and export.
- [ ] If a release build registers Chart Studio, separately validate its
      standalone LevelData import and export; it is not in the default UI.
- [ ] Build the Windows installer and portable ZIP from the clean matching tag.
- [ ] Exercise first launch, Quick Render, Program Preview, and export on
      a clean Windows user or VM with no Rust toolchain or FFmpeg on PATH.
- [ ] Exercise the portable archive, upgrade, reinstall, uninstall, and
      preservation of user data and external projects.
- [ ] Review FFmpeg behavior, missing license texts, asset rights, diagnostics,
      and secret redaction.
- [ ] Verify SHA-256 values and GitHub provenance attestations.
- [ ] Configure/verify Authenticode signing before describing an installer as
      signed.
- [ ] Review the draft release, then publish it explicitly.

## Local packaging

On Windows with Rust stable, run:

```powershell
./scripts/package-windows.ps1
```

The script creates a development-channel portable archive in
`target/distribution`. Existing `distribution` and `release-stage` outputs are
preserved: choose a fresh `-OutputRoot` for another package run, for example:

```powershell
./scripts/package-windows.ps1 -OutputRoot target/rc-review-<build-id>
```

If Inno Setup 6 is installed, the script also creates the installer. Use
`-RequireInstaller` to make a missing Inno compiler fatal. Do not pass
`-Channel stable` except from a clean `v<version>` release tag. CI is the
release publisher; local output is not an official release.

The checksum list covers the installer and portable ZIP. The manifest records
source/build identity, target, size, and hash. Reproducible byte-for-byte
builds are not currently claimed because the complete Windows toolchain,
environment, and packaging image are not pinned.
