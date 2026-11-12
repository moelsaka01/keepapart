# Packaging

Recipes for package managers. The release workflow (`.github/workflows/release.yml`)
builds the archives and a `.sha256` file for each when a `v*` tag is pushed.

After the first release, replace every `REPLACE_WITH_SHA256_FROM_RELEASE` with the
matching `.sha256` value, then:

| Manager | Where it goes |
|---|---|
| Scoop | `scoop/keepapart.json` into a bucket repo (e.g. `moelsaka01/scoop-bucket`); later, a PR to `ScoopInstaller/Extras` |
| Homebrew | `homebrew/keepapart.rb` into `moelsaka01/homebrew-tap` as `Formula/keepapart.rb` |
| winget | `winget/*.yaml` as a PR to `microsoft/winget-pkgs` under `manifests/m/MohamedElsaka/keepapart/<version>/` (`wingetcreate` can do it) |
| crates.io | set the `PUBLISH_CRATE` repo variable and the `CARGO_REGISTRY_TOKEN` secret, or run `cargo publish` |
