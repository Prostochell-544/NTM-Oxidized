#!/usr/bin/env python3
"""Build the mod folders the launcher takes, in dist/. Each one carries its own copy of ntm-core."""
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parent
CRATES = ROOT / "crates"
TEXTURES = ROOT / "textures"
DIST = ROOT / "dist"
SKIP = shutil.ignore_patterns("target", "__pycache__")
# Bundle crate -> where ntm-core goes inside it.
BUNDLES = {"ntm-direct": "ntm_core", "ntm-api": "engine/ntm_core"}


def main():
    for crate, core in BUNDLES.items():
        out = DIST / crate.replace("-", "_")
        shutil.rmtree(out, ignore_errors=True)
        shutil.copytree(CRATES / crate, out, ignore=SKIP)
        shutil.copytree(CRATES / "ntm-core", out / core, ignore=SKIP)
        print(out)
    # The content mod the game loads with --dev-mod: data and models, plus the texture pack's assets.
    out = DIST / "ntm_content"
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(CRATES / "ntm-content", out, ignore=SKIP)
    shutil.copytree(TEXTURES / "assets", out / "assets", dirs_exist_ok=True)
    print(out)


if __name__ == "__main__":
    main()
