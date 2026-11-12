# Packaging

Templates for package managers. When a `v*` tag is pushed, the release workflow
(`.github/workflows/release.yml`):

1. builds the archives and their `.sha256` files,
2. runs `render.py`, which fills in the version and the **real** checksums (a
   placeholder it can't fill stops the release),
3. attaches the finished recipes to the GitHub release,
4. commits the Scoop manifest to [`../bucket/`](../bucket/) and the Homebrew formula to
   [`../Formula/`](../Formula/) on `main`, so this repo is its own Scoop bucket and Homebrew tap,
5. installs keepapart through both on fresh runners (`.github/workflows/install.yml`).

| Manager | How users get it |
|---|---|
| Scoop | `scoop bucket add keepapart https://github.com/moelsaka01/keepapart` then `scoop install keepapart/keepapart` |
| Homebrew | `brew tap moelsaka01/keepapart https://github.com/moelsaka01/keepapart` then `brew install keepapart` |
| winget | PR the three `MohamedElsaka.keepapart*.yaml` files from the release to `microsoft/winget-pkgs` under `manifests/m/MohamedElsaka/keepapart/<version>/` (or `wingetcreate submit`) |
| crates.io | set the `PUBLISH_CRATE` repo variable to `true` and add a `CARGO_REGISTRY_TOKEN` secret; or run `cargo publish` once |

Try a render locally: `python packaging/render.py 0.1.0 <folder with archives + .sha256> out/`.
