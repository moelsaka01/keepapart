#!/usr/bin/env python3
"""Render the package-manager recipes with a release's real version and sha256s.

    python packaging/render.py 0.1.0 dist/ out/

`dist/` holds the release archives and their `.sha256` files, as built by
.github/workflows/release.yml. Every {{VERSION}} and {{SHA256:<target>}} in the
templates is filled in; a placeholder left unfilled is an error, so a recipe is
never published with a missing or made-up checksum.
"""

import pathlib
import re
import sys

TEMPLATES = [
    "scoop/keepapart.json",
    "homebrew/keepapart.rb",
    "winget/MohamedElsaka.keepapart.yaml",
    "winget/MohamedElsaka.keepapart.installer.yaml",
    "winget/MohamedElsaka.keepapart.locale.en-US.yaml",
]


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__)
        return 2
    version, dist, out = sys.argv[1].lstrip("v"), pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
    here = pathlib.Path(__file__).parent

    hashes = {}
    for f in dist.glob("keepapart-v*.sha256"):
        m = re.match(rf"keepapart-v{re.escape(version)}-(.+?)\.(zip|tar\.gz)\.sha256$", f.name)
        if m:
            digest = f.read_text().split()[0].lower()
            if not re.fullmatch(r"[0-9a-f]{64}", digest):
                raise SystemExit(f"{f}: not a sha256: {digest!r}")
            hashes[m.group(1)] = digest

    for rel in TEMPLATES:
        text = (here / rel).read_text(encoding="utf-8").replace("{{VERSION}}", version)

        def fill(m: re.Match) -> str:
            target = m.group(1)
            if target not in hashes:
                raise SystemExit(f"{rel}: no .sha256 in {dist} for target {target}")
            return hashes[target]

        text = re.sub(r"\{\{SHA256:([^}]+)\}\}", fill, text)
        if "{{" in text:
            raise SystemExit(f"{rel}: unfilled placeholder left")
        dest = out / pathlib.Path(rel).name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(text, encoding="utf-8", newline="\n")
        print(f"rendered {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
