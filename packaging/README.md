# Packaging

Templates for package managers. When a `v*` tag is pushed, the release workflow
(`.github/workflows/release.yml`) builds the archives and their `.sha256` files, then
runs `render.py`, which fills in the version and the **real** checksums and attaches
the finished recipes to the GitHub release. A placeholder it can't fill stops the
release, so a recipe is never published with a missing or made-up checksum.

| Manager | What to do with the rendered file from the release |
|---|---|
| Scoop | put `keepapart.json` in a bucket repo (e.g. `moelsaka01/scoop-bucket`); it has `checkver`/`autoupdate`, so later versions update themselves. Once it's popular, PR it to `ScoopInstaller/Extras` |
| Homebrew | put `keepapart.rb` in `moelsaka01/homebrew-tap` as `Formula/keepapart.rb` |
| winget | PR the three `MohamedElsaka.keepapart*.yaml` files to `microsoft/winget-pkgs` under `manifests/m/MohamedElsaka/keepapart/<version>/` (or use `wingetcreate submit`) |
| crates.io | set the `PUBLISH_CRATE` repo variable to `true` and add a `CARGO_REGISTRY_TOKEN` secret; or run `cargo publish` once |

Try a render locally: `python packaging/render.py 0.1.0 <folder with archives + .sha256> out/`.
