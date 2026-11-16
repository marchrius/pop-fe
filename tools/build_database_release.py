#!/usr/bin/env python3
"""Create the versioned archive and integrity manifest for a database-only release."""

import argparse
import hashlib
import json
import os
import re
import zipfile
from pathlib import Path


DATABASE_FILES = (
    "disc_by_md5.json",
    "games.json",
    "libcrypt.json",
    "libcrypt_hash_patches.json",
    "libcrypt_patches.json",
    "libcrypt_revision_signatures.json",
    "ppf_data.json",
    "ppf_fixes.json",
    "psp_configs.json",
    "themes.json",
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default=os.environ.get("GITHUB_SHA"))
    parser.add_argument("--sequence", type=int, default=os.environ.get("GITHUB_RUN_NUMBER"))
    parser.add_argument("--output", type=Path, default=Path("dist/database"))
    args = parser.parse_args()

    if not args.revision or not re.fullmatch(r"[0-9a-fA-F]{40}", args.revision):
        parser.error("--revision or GITHUB_SHA must be a full 40-character commit ID")
    try:
        sequence = int(args.sequence)
    except (TypeError, ValueError):
        parser.error("--sequence or GITHUB_RUN_NUMBER must be a positive integer")
    if sequence < 1:
        parser.error("--sequence or GITHUB_RUN_NUMBER must be a positive integer")

    root = Path(__file__).resolve().parents[1]
    files = {name: (root / "data" / name).read_bytes() for name in DATABASE_FILES}
    archive_name = f"pop-fe-database-{args.revision[:12].lower()}.zip"
    args.output.mkdir(parents=True, exist_ok=True)
    archive_path = args.output / archive_name
    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name in DATABASE_FILES:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, files[name], compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)

    archive_bytes = archive_path.read_bytes()
    manifest = {
        "format_version": 1,
        "schema_version": 1,
        "revision": args.revision.lower(),
        "sequence": sequence,
        "archive": {
            "name": archive_name,
            "sha256": sha256(archive_bytes),
            "size": len(archive_bytes),
        },
        "files": {
            name: {"sha256": sha256(files[name]), "size": len(files[name])}
            for name in sorted(DATABASE_FILES)
        },
    }
    (args.output / "database-manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"Created {archive_path} and {args.output / 'database-manifest.json'}")


if __name__ == "__main__":
    main()
