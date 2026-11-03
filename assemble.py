#!/usr/bin/env python3
"""Build the mod folders the launcher takes, in dist/. Each one carries its own copy of ntm-core."""
from pathlib import Path
import json
import shutil

ROOT = Path(__file__).resolve().parent
CRATES = ROOT / "crates"
GENERATED = ROOT / "generated"
TEXTURES = ROOT / "textures"
DIST = ROOT / "dist"
SKIP = shutil.ignore_patterns("target", "__pycache__")
# Bundle crate -> where ntm-core goes inside it.
BUNDLES = {"ntm-direct": "ntm_core", "ntm-api": "engine/ntm_core"}
# Files both trees may hold, merged rather than replaced: names, tags, creative tabs.
MERGED = ("/lang/", "/tags/", "/horizon/creative_tab/")


def merge(base, extra):
    """`extra` folded into `base`: base keeps its values, lists gain what they lack."""
    if isinstance(base, dict) and isinstance(extra, dict):
        return {**{k: merge(v, extra[k]) if k in extra else v for k, v in base.items()},
                **{k: v for k, v in extra.items() if k not in base}}
    if isinstance(base, list) and isinstance(extra, list):
        return base + [v for v in extra if v not in base]
    return base


def overlay(src, out):
    """Generated content under the hand-written: a file both have is the hand-written one, or both merged."""
    for path in sorted(src.rglob("*")):
        if not path.is_file():
            continue
        dst = out / path.relative_to(src)
        if not dst.exists():
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, dst)
        elif path.suffix == ".json" and any(m in "/" + dst.relative_to(out).as_posix() for m in MERGED):
            data = merge(json.loads(dst.read_text(encoding="utf-8")), json.loads(path.read_text(encoding="utf-8")))
            dst.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def main():
    for crate, core in BUNDLES.items():
        out = DIST / crate.replace("-", "_")
        shutil.rmtree(out, ignore_errors=True)
        shutil.copytree(CRATES / crate, out, ignore=SKIP)
        shutil.copytree(CRATES / "ntm-core", out / core, ignore=SKIP)
        print(out)
    # The content mod the game loads with --dev-mod: hand-written content, then the generated, then the textures.
    out = DIST / "ntm_content"
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(CRATES / "ntm-content", out, ignore=SKIP)
    if GENERATED.is_dir():
        overlay(GENERATED, out)
    shutil.copytree(TEXTURES / "assets", out / "assets", dirs_exist_ok=True)
    print(out)


if __name__ == "__main__":
    main()
