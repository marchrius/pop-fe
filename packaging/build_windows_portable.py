#!/usr/bin/env python3
"""Assemble a portable Windows ZIP from a UCRT64 build environment."""

import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile
from zipfile import ZIP_DEFLATED, ZipFile

from deno_bundle import ensure_bundle as ensure_deno_bundle
from lcp_bundle import ensure_bundle as ensure_lcp_bundle
from license_bundle import ensure_apache_2_license
from yt_dlp_bundle import ensure_bundle as ensure_yt_dlp_bundle

ROOT = Path(__file__).resolve().parents[1]


def copy_tree(source: Path, target: Path) -> None:
    if source.is_dir():
        shutil.copytree(source, target, dirs_exist_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/pop-fe-psp-windows-x86_64.zip")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    prefix_value = os.environ.get("MINGW_PREFIX")
    if not prefix_value:
        parser.error("run this script in an MSYS2 UCRT64 shell (MINGW_PREFIX is missing)")
    prefix = Path(prefix_value)
    binary = ROOT / "target/release/pop-fe-psp.exe"
    encoder = ROOT / "target/atracdenc-build/atracdenc.exe"
    if not binary.is_file() or not encoder.is_file():
        parser.error("build pop-fe-psp.exe and atracdenc.exe before packaging")

    yt_dlp, yt_license, yt_third_party = ensure_yt_dlp_bundle("windows-x86_64", offline=args.offline)
    deno, deno_license = ensure_deno_bundle("windows-x86_64", offline=args.offline)
    lcp = ensure_lcp_bundle("windows-x86_64", offline=args.offline)
    xdelta_license = ensure_apache_2_license(offline=args.offline)
    args.output.parent.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="pop-fe-windows-") as temporary:
        stage = Path(temporary) / "POP-FE PSP Manager"
        stage.mkdir()
        for source, name in (
            (binary, "pop-fe-psp.exe"),
            (encoder, "atracdenc.exe"),
            (yt_dlp, "yt-dlp.exe"),
            (deno, "deno.exe"),
            (lcp, "lcp.exe"),
        ):
            shutil.copy2(source, stage / name)

        # Include the utilities used by the existing conversion and manual flows.
        copied_seven_zip = False
        for name in ("ffmpeg.exe", "pdftoppm.exe", "7z.exe", "7zz.exe", "7z.dll", "chdman.exe", "xdelta3.exe"):
            source = shutil.which(name)
            if source:
                target_name = "7z.exe" if name in ("7z.exe", "7zz.exe") else Path(source).name
                if name in ("7z.exe", "7zz.exe") and copied_seven_zip:
                    continue
                shutil.copy2(source, stage / target_name)
                copied_seven_zip |= name in ("7z.exe", "7zz.exe")
        xdelta = shutil.which("xdelta3.exe")
        if xdelta is None:
            raise RuntimeError("xdelta3.exe is required to package automatic compatibility patches")
        for name in ("msys-2.0.dll", "msys-xdelta3-3.dll", "msys-lzma-5.dll"):
            dependency = shutil.which(name)
            if dependency is None:
                raise RuntimeError(f"MSYS2 xdelta3 runtime dependency is missing: {name}")
            shutil.copy2(dependency, stage / name)

        licenses = stage / "licenses"
        licenses.mkdir()
        shutil.copy2(yt_license, licenses / "yt-dlp-LICENSE")
        shutil.copy2(yt_third_party, licenses / "yt-dlp-THIRD_PARTY_LICENSES.txt")
        shutil.copy2(deno_license, licenses / "deno-LICENSE.md")
        shutil.copy2(ROOT / "licenses/libcrypt-patcher-BSD-3-Clause.txt", licenses / "libcrypt-patcher-LICENSE.txt")
        shutil.copy2(ROOT / "licenses/lib-enigma-BSD-3-Clause.txt", licenses / "lib-enigma-LICENSE.txt")
        shutil.copy2(xdelta_license, licenses / "xdelta3-Apache-2.0.txt")
        shutil.copy2(ROOT / "LICENCE-LGPL-2.1.txt", licenses / "pop-fe-LICENSE.txt")
        copy_tree(prefix / "share/licenses", licenses / "msys2-ucrt64")
        copy_tree(prefix.parent / "usr/share/licenses", licenses / "msys2-runtime")

        # MSYS2's UCRT64 packages install the GTK/OpenCV runtime DLLs together.
        runtime_bin = prefix / "bin"
        for source in runtime_bin.glob("*.dll"):
            shutil.copy2(source, stage / source.name)
        query_loaders = runtime_bin / "gdk-pixbuf-query-loaders.exe"
        if not query_loaders.is_file():
            raise RuntimeError("GTK runtime is missing gdk-pixbuf-query-loaders.exe")
        shutil.copy2(query_loaders, stage / query_loaders.name)
        for relative in (
            "share/glib-2.0/schemas",
            "share/gtk-4.0",
            "share/icons/Adwaita",
            "share/icons/hicolor",
            "share/themes/Adwaita",
            "share/gdk-pixbuf-2.0",
            "lib/gdk-pixbuf-2.0",
            "share/icons/hicolor/scalable/apps",
        ):
            copy_tree(prefix / relative, stage / relative)
        openssl_modules = prefix / "lib/ossl-modules"
        if not list(openssl_modules.glob("legacy.*")):
            raise RuntimeError("MSYS2 OpenSSL legacy provider is missing; encrypted PS3 manuals need it")
        copy_tree(openssl_modules, stage / "ossl-modules")
        app_icons = stage / "share/icons/hicolor/scalable/apps"
        app_icons.mkdir(parents=True, exist_ok=True)
        shutil.copy2(
            ROOT / "packaging/icons/hicolor/scalable/apps/org.popfe.PSPManager.svg",
            app_icons / "org.popfe.PSPManager.svg",
        )

        loaders = stage / "lib/gdk-pixbuf-2.0/2.10.0/loaders"
        (stage / "Start POP-FE PSP Manager.cmd").write_text(
            "@echo off\r\n"
            "setlocal\r\n"
            "set APP_DIR=%~dp0\r\n"
            "set PATH=%APP_DIR%;%PATH%\r\n"
            "set GTK_DATA_PREFIX=%APP_DIR%\r\n"
            "set GTK_EXE_PREFIX=%APP_DIR%\r\n"
            "set XDG_DATA_DIRS=%APP_DIR%share\r\n"
            "set GSETTINGS_SCHEMA_DIR=%APP_DIR%share\\glib-2.0\\schemas\r\n"
            "if exist \"%APP_DIR%ossl-modules\" set OPENSSL_MODULES=%APP_DIR%ossl-modules\r\n"
            "set GDK_PIXBUF_MODULE_FILE=%APP_DIR%gdk-pixbuf.loaders\r\n"
            "if exist \"%APP_DIR%gdk-pixbuf-query-loaders.exe\" (\r\n"
            "  if exist \"%APP_DIR%gdk-pixbuf.loaders\" del \"%APP_DIR%gdk-pixbuf.loaders\"\r\n"
            "  for %%F in (\"%APP_DIR%lib\\gdk-pixbuf-2.0\\2.10.0\\loaders\\*.dll\") do if exist \"%%~fF\" \"%APP_DIR%gdk-pixbuf-query-loaders.exe\" \"%%~fF\" >> \"%APP_DIR%gdk-pixbuf.loaders\"\r\n"
            ")\r\n"
            "\"%APP_DIR%pop-fe-psp.exe\"\r\n",
            encoding="utf-8",
        )
        (stage / "README.txt").write_text(
            "POP-FE PSP Manager portable build\n\n"
            "Run Start POP-FE PSP Manager.cmd. This ZIP includes the GTK/OpenCV runtime, "
            "ATRAC3 encoder, yt-dlp, Deno, LibCrypt/Xdelta patchers, FFmpeg, Poppler and 7-Zip.\n"
            "CHD images need chdman.exe from MAME (https://www.mamedev.org/release.html) and its adjacent DLLs. "
            "Place them beside the application or add their folder to PATH.\n",
            encoding="utf-8",
        )
        with ZipFile(args.output, "w", compression=ZIP_DEFLATED, compresslevel=8) as archive:
            for path in sorted(stage.rglob("*")):
                if path.is_file():
                    archive.write(path, path.relative_to(stage.parent))
    print(args.output)


if __name__ == "__main__":
    main()
