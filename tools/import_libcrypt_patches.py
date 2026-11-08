#!/usr/bin/env python3
"""Import unambiguous PPF mappings from an upstream libcrypt-patcher checkout."""

import argparse
import json
import re
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("upstream", type=Path, help="checkout of alex-free/libcrypt-patcher")
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    source = upstream / "lcp.c"
    patches_dir = upstream / "ppf-patches"
    patch_header = upstream / "patches.h"
    license_file = upstream / "license.md"
    for required in (source, patches_dir, patch_header, license_file):
        if not required.exists():
            parser.error(f"upstream checkout is missing {required.name}")

    macro_to_file = {}
    for name in re.findall(r'#include\s+"ppf-c/([^"/]+)\.PPF\.h"', patch_header.read_text()):
        macro_to_file[name.replace(".", "_")] = f"{name}.PPF"

    text = source.read_text(errors="replace")
    starts = list(re.finditer(r"(?m)^\t(?:else )?if\s*\(", text))
    mapping = {}
    skipped = []
    for index, match in enumerate(starts):
        block = text[match.start():starts[index + 1].start() if index + 1 < len(starts) else len(text)]
        identifiers = re.findall(
            r'strcmp\(boot_file,\s*"([A-Z]{4}_[0-9]{3}\.[0-9]{2})"\)', block
        )
        patches = set(re.findall(r"apply_ppf\(\s*([A-Z0-9_]+)_PPF\s*,", block))
        if not identifiers or not patches:
            continue
        if "id_rev(" in block or len(patches) != 1:
            skipped.extend(identifiers)
            continue
        patch_macro = next(iter(patches))
        filename = macro_to_file.get(patch_macro)
        if filename is None or not (patches_dir / filename).is_file():
            raise RuntimeError(f"cannot resolve upstream patch macro {patch_macro}")
        relative = f"libcrypt-upstream/{filename}"
        for identifier in identifiers:
            game_id = identifier.replace("_", "").replace(".", "")
            old = mapping.setdefault(game_id, relative)
            if old != relative:
                raise RuntimeError(f"conflicting LibCrypt patches for {game_id}")

    destination = ROOT / "libcrypt-upstream"
    destination.mkdir(exist_ok=True)
    copied = 0
    for patch in sorted(patches_dir.glob("*.PPF")):
        shutil.copyfile(patch, destination / patch.name)
        copied += 1
    licenses = ROOT / "licenses"
    licenses.mkdir(exist_ok=True)
    shutil.copyfile(license_file, licenses / "libcrypt-patcher-BSD-3-Clause.txt")
    target = ROOT / "data/libcrypt_patches.json"
    target.write_text(
        json.dumps(mapping, ensure_ascii=False, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    print(f"Copied {copied} upstream PPF files; mapped {len(mapping)} game IDs")
    print(f"Left {len(set(skipped))} revision-specific IDs to the revision-aware fallback")


if __name__ == "__main__":
    main()
