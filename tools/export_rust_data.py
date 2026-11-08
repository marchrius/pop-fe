#!/usr/bin/env python3
"""Export the legacy Python literal databases as static Rust-app JSON data."""

import json
import runpy
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "data"


def write(name, value):
    OUT.mkdir(parents=True, exist_ok=True)
    target = OUT / f"{name}.json"
    target.write_text(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"{target.relative_to(ROOT)}: {target.stat().st_size:,} bytes")


def json_value(value):
    if isinstance(value, bytes):
        return value.decode("latin-1")
    if isinstance(value, dict):
        return {str(key): json_value(item) for key, item in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [json_value(item) for item in value]
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    return str(value)


def main():
    source_discs = runpy.run_path(str(ROOT / "db.py"))["disc_by_md5"]
    discs = {
        checksum: {key: record[key] for key in ("id", "title", "exe") if key in record}
        for checksum, record in source_discs.items()
    }
    source = runpy.run_path(str(ROOT / "gamedb.py"))
    write("disc_by_md5", discs)
    write("games", json_value(source["games"]))
    write("libcrypt", json_value(source["libcrypt"]))
    write("themes", json_value(source["themes"]))
    write("ppf_fixes", json_value(source["ppf_fixes"]))


if __name__ == "__main__":
    main()
