#!/usr/bin/env python3
"""Backend shared by the GTK PSP frontend.

The operations here intentionally follow the workflow in pop-fe-psp.py:
process each disc, identify it, load metadata/assets, then call create_psp.
"""

from __future__ import annotations

import argparse
import contextlib
import importlib
import io
import json
import os
import pathlib
import shutil
import sys
import tempfile

from PIL import Image
from gamedb import games, libcrypt


ROOT = pathlib.Path(__file__).resolve().parent
WORK_ROOT = pathlib.Path(os.environ.get("XDG_CACHE_HOME", pathlib.Path.home() / ".cache")) / "org.popfe.PSPManager"


def prepare_runtime() -> None:
    """Keep relative resource paths usable with a read-only installation."""
    runtime = WORK_ROOT / "runtime"
    runtime.mkdir(parents=True, exist_ok=True)
    for name in ("ppf", "pspconfigs", "romhacks", "binmerge", "Cue2cu2", "atracdenc", "psx-undither", "lcp"):
        source, target = ROOT / name, runtime / name
        if source.exists() and not target.exists() and not target.is_symlink():
            target.symlink_to(source, target_is_directory=source.is_dir())
    os.chdir(runtime)


def popfe_module():
    try:
        return importlib.import_module("popfe")
    except ImportError:
        return importlib.import_module("pop-fe")


def analyze(path: str, workdir: pathlib.Path) -> dict:
    popfe = popfe_module()
    shutil.rmtree(workdir, ignore_errors=True)
    workdir.mkdir(parents=True, exist_ok=True)
    temp_files: list[str] = []
    subdir = str(workdir) + "/"
    cue, real_cue, img = popfe.process_disk_file(path, 0, temp_files, subdir=subdir)
    if not cue:
        raise RuntimeError(f"Formato non riconosciuto: {path}")
    disc_id, md5 = popfe.get_disc_id(cue, real_cue, subdir + "metadata.iso", is_psp=True)
    game = popfe.get_game_from_gamelist(disc_id)
    result = {
        "path": path,
        "cue": cue,
        "real_cue": real_cue,
        "img": img,
        "disc_id": disc_id,
        "md5": md5,
        "title": popfe.get_title_from_game(disc_id),
        "manual": popfe.find_local_manual(real_cue),
        "cdda": disc_id in games and "psp-use-cdda" in games[disc_id],
        "assets": {},
    }

    # Same automatic asset sources as the original PSP GUI, cached in workdir.
    try:
        icon = popfe.get_icon0_from_game(disc_id, game, real_cue, subdir + "ICON0.PNG")
        if icon:
            filename = workdir / "ICON0.PNG"
            icon.save(filename)
            result["assets"]["cover"] = str(filename)
    except Exception:
        pass
    for key, loader, filename in (
        ("pic0", popfe.get_pic0_from_game, "PIC0.PNG"),
        ("pic1", popfe.get_pic1_from_game, "PIC1.PNG"),
    ):
        try:
            image = loader(disc_id, game, real_cue)
            if image:
                output = workdir / filename
                image.save(output)
                result["assets"][key] = str(output)
        except Exception:
            pass
    return result


def create(config: dict, workdir: pathlib.Path) -> None:
    popfe = popfe_module()
    shutil.rmtree(workdir, ignore_errors=True)
    workdir.mkdir(parents=True, exist_ok=True)
    temp_files: list[str] = []
    subdir = str(workdir) + "/"
    cue_files: list[str] = []
    real_cue_files: list[str] = []
    img_files: list[str] = []
    real_disc_ids: list[str] = []
    md5_sums: list[str] = []

    for index, original in enumerate(config["files"]):
        print(f"POPFE_PHASE: Analisi disco {index + 1}/{len(config['files'])}", flush=True)
        source = original
        if pathlib.Path(original).suffix.lower() == ".iso":
            cue = workdir / f"ISO{index}.cue"
            cue.write_text(
                f'FILE "{pathlib.Path(original).resolve()}" BINARY\n'
                "  TRACK 01 MODE1/2048\n"
                "    INDEX 01 00:00:00\n",
                encoding="utf-8",
            )
            source = str(cue)
        cue, real_cue, img = popfe.process_disk_file(source, index, temp_files, subdir=subdir)
        if not cue:
            raise RuntimeError(f"Formato non riconosciuto: {original}")
        disc_id, md5 = popfe.get_disc_id(cue, real_cue, subdir + f"ISO{index:02x}01.iso", is_psp=True)
        cue_files.append(cue)
        real_cue_files.append(real_cue)
        img_files.append(img)
        real_disc_ids.append(disc_id)
        md5_sums.append(md5)

    disc_ids = list(config.get("disc_ids") or real_disc_ids)
    print("POPFE_PHASE: Applicazione patch di compatibilità", flush=True)
    cue_files, img_files = popfe.apply_ppf_fixes(real_disc_ids, cue_files, img_files, md5_sums, subdir, tag="psp")
    print("POPFE_PHASE: Preparazione tracce audio", flush=True)
    aea_files, extra_data_tracks = popfe.generate_aea_files(cue_files, img_files, subdir)
    options = config.get("options", {})
    cdda = bool(options.get("cdda") or extra_data_tracks)
    manual = config.get("manual")
    if manual:
        print("POPFE_PHASE: Preparazione manuale", flush=True)
        manual = popfe.create_manual(manual, disc_ids[0], subdir=subdir)

    def image_asset(key: str):
        path = config.get("assets", {}).get(key)
        return Image.open(path) if path else None

    subchannels = [popfe.generate_subchannels(libcrypt[disc_id]["magic_word"]) if disc_id in libcrypt else None for disc_id in real_disc_ids]
    print("POPFE_PHASE: Compressione e scrittura EBOOT.PBP", flush=True)
    popfe.create_psp(
        config["output"], disc_ids, real_disc_ids, config.get("title") or popfe.get_title_from_game(disc_ids[0]),
        image_asset("cover"), image_asset("pic0"), image_asset("pic1"), cue_files, real_cue_files, img_files, [], aea_files,
        subdir=subdir, snd0=config.get("snd0"), no_pstitleimg=bool(options.get("nopstitleimg")),
        watermark=bool(options.get("watermark")), subchannels=subchannels, manual=manual,
        use_cdda=bool(options.get("use_cdda")), logo=image_asset("pic1") if options.get("pic1aslogo") else image_asset("logo"), no_libcrypt=bool(options.get("no_libcrypt")),
        psx_undither=bool(options.get("undither")), force_ntsc=bool(options.get("force_ntsc")), cdda=cdda,
        swap_disc=bool(options.get("swap_disc")),
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("analyze", "create"))
    parser.add_argument("path", nargs="?")
    parser.add_argument("--config")
    parser.add_argument("--workdir")
    args = parser.parse_args()
    prepare_runtime()
    workdir = pathlib.Path(args.workdir) if args.workdir else WORK_ROOT / args.command
    if args.command == "analyze":
        diagnostics = io.StringIO()
        try:
            with contextlib.redirect_stdout(diagnostics), contextlib.redirect_stderr(diagnostics):
                result = analyze(args.path, workdir)
        except Exception as exc:
            print(str(exc), file=sys.stderr)
            print(diagnostics.getvalue(), file=sys.stderr, end="")
            raise SystemExit(1)
        print(json.dumps(result, ensure_ascii=False))
    elif args.command == "create":
        if not args.config:
            raise SystemExit("create richiede --config")
        create(json.loads(pathlib.Path(args.config).read_text(encoding="utf-8")), workdir)


if __name__ == "__main__":
    main()
