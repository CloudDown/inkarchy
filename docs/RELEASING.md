# Releasing

1. Bump `version` in `Cargo.toml`.
2. Add a `## <version>` section in `CHANGELOG.md` with the user-facing notes.
3. Commit and push to `main`.
4. `auto-tag` creates `v<version>` and calls `release`, which attaches `inkarchy-x86_64-unknown-linux-gnu`.
5. `install.sh` downloads that asset from the latest GitHub Release; existing notes and folders are untouched.
