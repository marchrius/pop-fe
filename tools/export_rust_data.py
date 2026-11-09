#!/usr/bin/env python3
"""Export the legacy Python literal databases as static Rust-app JSON data."""

import ast
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
    psp_configs = {}
    missing_psp_configs = []
    for game_id, game in source["games"].items():
        relative = game.get("pspconfig")
        if not relative:
            continue
        config_path = ROOT / relative
        if config_path.is_file():
            psp_configs[game_id] = config_path.read_bytes().hex()
        else:
            missing_psp_configs.append((game_id, relative))
    if missing_psp_configs:
        print(
            f"warning: {len(missing_psp_configs)} game records reference missing PSP configs:",
        )
        for game_id, relative in missing_psp_configs:
            print(f"  {game_id}: {relative}")
    write("disc_by_md5", discs)
    write("games", json_value(source["games"]))
    write("psp_configs", psp_configs)
    write("libcrypt", json_value(source["libcrypt"]))
    write("themes", json_value(source["themes"]))
    write("ppf_fixes", json_value(source["ppf_fixes"]))
    ppf_files = {
        str(path.relative_to(ROOT)): path.read_bytes().hex()
        for folder in ("ppf", "libcrypt", "romhacks", "libcrypt-upstream")
        for path in (ROOT / folder).glob("*.PPF" if folder == "libcrypt-upstream" else "*")
        if path.is_file()
    }
    write("ppf_data", ppf_files)
    export_psp_resources()


def export_psp_resources():
    OUT.joinpath("psp").mkdir(parents=True, exist_ok=True)
    source = ast.parse((ROOT / "popstation.py").read_text(encoding="utf-8"))
    names = {"_pstitledata", "_startdatfooter", "_startdatheader", "_datapspbody", "_logo_buffer"}
    for node in ast.walk(source):
        if not isinstance(node, ast.Assign):
            continue
        target = next((item.id for item in node.targets if isinstance(item, ast.Name) and item.id in names), None)
        if target is None:
            continue
        value = node.value
        literal = ast.literal_eval(value.args[0]) if isinstance(value, ast.Call) else ast.literal_eval(value)
        payload = bytes(literal)
        target_path = OUT / "psp" / f"{target.removeprefix('_')}.bin"
        target_path.write_bytes(payload)
        print(f"{target_path.relative_to(ROOT)}: {len(payload):,} bytes")

    frontend = ast.parse((ROOT / "pop-fe.py").read_text(encoding="utf-8"))
    for node in ast.walk(frontend):
        if isinstance(node, ast.Assign) and any(isinstance(item, ast.Name) and item.id == "EMPTY_CONFIG" for item in node.targets):
            target_path = OUT / "psp" / "empty_config.bin"
            payload = bytes(ast.literal_eval(node.value.args[0]))
            target_path.write_bytes(payload)
            print(f"{target_path.relative_to(ROOT)}: {len(payload):,} bytes")
            break


if __name__ == "__main__":
    main()
