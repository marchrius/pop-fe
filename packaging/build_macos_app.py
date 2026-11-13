#!/usr/bin/env python3
"""Assemble a self-contained GTK macOS app bundle for the current architecture."""

import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess

from deno_bundle import ensure_bundle as ensure_deno_bundle
from lcp_bundle import ensure_bundle as ensure_lcp_bundle
from license_bundle import ensure_gpl_2_license
from yt_dlp_bundle import ensure_bundle as ensure_yt_dlp_bundle

ROOT = Path(__file__).resolve().parents[1]


def copy_program(name: str, destination: Path, target_name: str | None = None) -> Path:
    source = shutil.which(name)
    if source is None:
        raise RuntimeError(f"required runtime program is unavailable: {name}")
    target = destination / (target_name or Path(source).name)
    shutil.copy2(source, target)
    target.chmod(0o755)
    return target


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=(ROOT / "VERSION").read_text(encoding="utf-8").strip())
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    machine = platform.machine().lower()
    if machine in ("arm64", "aarch64"):
        deno_target = "macos-aarch64"
        architecture = "aarch64"
    elif machine in ("x86_64", "amd64"):
        deno_target = "macos-x86_64"
        architecture = "x86_64"
    else:
        parser.error(f"unsupported macOS architecture: {machine}")

    binary = ROOT / "target/release/pop-fe-psp"
    encoder = ROOT / "target/atracdenc-build/atracdenc"
    if not binary.is_file() or not encoder.is_file():
        parser.error("build pop-fe-psp and atracdenc before packaging")
    yt_dlp, yt_license, yt_third_party = ensure_yt_dlp_bundle("macos", offline=args.offline)
    deno, deno_license = ensure_deno_bundle(deno_target, offline=args.offline)
    lcp = ensure_lcp_bundle("macos-x86_64", offline=args.offline)
    xdelta_license = ensure_gpl_2_license(offline=args.offline)
    dylibbundler = shutil.which("dylibbundler")
    if dylibbundler is None:
        parser.error("install dylibbundler with Homebrew before packaging")

    output = ROOT / "dist"
    output.mkdir(exist_ok=True)
    app = output / "POP-FE PSP Manager.app"
    if app.exists():
        shutil.rmtree(app)
    contents = app / "Contents"
    macos = contents / "MacOS"
    resources = contents / "Resources"
    frameworks = contents / "Frameworks"
    macos.mkdir(parents=True)
    resources.mkdir()
    frameworks.mkdir()

    for source, name in (
        (binary, "pop-fe-psp"),
        (encoder, "atracdenc"),
        (yt_dlp, "yt-dlp"),
        (deno, "deno"),
        (lcp, "lcp"),
    ):
        target = macos / name
        shutil.copy2(source, target)
        target.chmod(0o755)

    prefix = Path(subprocess.check_output(["brew", "--prefix"], text=True).strip())
    runtime_programs = (
        ("ffmpeg", "ffmpeg"),
        ("pdftoppm", "pdftoppm"),
        ("7zz", "7zz"),
        ("chdman", "chdman"),
        ("xdelta3", "xdelta3"),
    )
    for program, bundled_name in runtime_programs:
        copy_program(program, macos, bundled_name)

    licenses = resources / "licenses"
    licenses.mkdir()
    shutil.copy2(yt_license, licenses / "yt-dlp-LICENSE")
    shutil.copy2(yt_third_party, licenses / "yt-dlp-THIRD_PARTY_LICENSES.txt")
    shutil.copy2(deno_license, licenses / "deno-LICENSE.md")
    shutil.copy2(ROOT / "licenses/libcrypt-patcher-BSD-3-Clause.txt", licenses / "libcrypt-patcher-LICENSE.txt")
    shutil.copy2(xdelta_license, licenses / "xdelta3-GPL-2.0-or-later.txt")
    shutil.copy2(ROOT / "LICENCE-LGPL-2.1.txt", licenses / "pop-fe-LICENSE.txt")

    for package in ("gtk4", "libadwaita", "glib", "gdk-pixbuf", "adwaita-icon-theme", "poppler", "ffmpeg", "openssl@3"):
        package_prefix = Path(subprocess.check_output(["brew", "--prefix", package], text=True).strip())
        if package == "gdk-pixbuf":
            pixbuf_modules = package_prefix / "lib/gdk-pixbuf-2.0"
            if pixbuf_modules.exists():
                shutil.copytree(pixbuf_modules, resources / "lib/gdk-pixbuf-2.0", dirs_exist_ok=True)
        share = package_prefix / "share"
        for relative in (
            "gtk-4.0",
            "glib-2.0/schemas",
            "icons/Adwaita",
            "icons/hicolor",
            "themes/Adwaita",
            "gdk-pixbuf-2.0",
            "poppler",
        ):
            source = share / relative
            if source.exists():
                shutil.copytree(source, resources / "share" / relative, dirs_exist_ok=True)

    app_icon_source = ROOT / "packaging/icons/hicolor/scalable/apps/org.popfe.PSPManager.svg"
    app_icon_dir = resources / "share/icons/hicolor/scalable/apps"
    app_icon_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(app_icon_source, app_icon_dir / app_icon_source.name)
    openssl_prefix = Path(subprocess.check_output(["brew", "--prefix", "openssl@3"], text=True).strip())
    openssl_modules = openssl_prefix / "lib/ossl-modules"
    if not (openssl_modules / "legacy.dylib").is_file():
        raise RuntimeError("OpenSSL legacy provider is missing; encrypted PS3 manuals need it")
    shutil.copytree(openssl_modules, resources / "ossl-modules", dirs_exist_ok=True)
    copy_program("gdk-pixbuf-query-loaders", macos, "gdk-pixbuf-query-loaders")
    icon_png = resources / "app-icon.png"
    subprocess.run(["rsvg-convert", "--width", "1024", "--height", "1024", str(app_icon_source), "--output", str(icon_png)], check=True)
    iconset = resources / "AppIcon.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            pixels = size * scale
            subprocess.run(
                ["sips", "-z", str(pixels), str(pixels), str(icon_png), "--out", str(iconset / f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png")],
                check=True,
                stdout=subprocess.DEVNULL,
            )
    icon_icns = resources / "AppIcon.icns"
    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(icon_icns)], check=True)
    shutil.rmtree(iconset)
    icon_png.unlink()

    plist = f'''<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>POP-FE PSP Manager</string>
<key>CFBundleDisplayName</key><string>POP-FE PSP Manager</string>
<key>CFBundleIdentifier</key><string>org.popfe.PSPManager</string>
<key>CFBundleVersion</key><string>{args.version}</string>
<key>CFBundleShortVersionString</key><string>{args.version}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleExecutable</key><string>launch</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
</dict></plist>\n'''
    (contents / "Info.plist").write_text(plist, encoding="utf-8")
    launcher = macos / "launch"
    launcher.write_text(
        "#!/bin/sh\n"
        'HERE="$(cd "$(dirname "$0")" && pwd)"\n'
        'RESOURCES="$(cd "$HERE/../Resources" && pwd)"\n'
        'export PATH="$HERE:$PATH"\n'
        'export DYLD_LIBRARY_PATH="$HERE/../Frameworks${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"\n'
        'export GTK_DATA_PREFIX="$RESOURCES"\n'
        'export GTK_EXE_PREFIX="$RESOURCES"\n'
        'export XDG_DATA_DIRS="$RESOURCES/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"\n'
        'export GSETTINGS_SCHEMA_DIR="$RESOURCES/share/glib-2.0/schemas"\n'
        'if [ -d "$RESOURCES/ossl-modules" ]; then export OPENSSL_MODULES="$RESOURCES/ossl-modules"; fi\n'
        'if [ -x "$HERE/gdk-pixbuf-query-loaders" ]; then\n'
        '  CACHE="${HOME}/Library/Caches/org.popfe.PSPManager"\n'
        '  mkdir -p "$CACHE"\n'
        '  "$HERE/gdk-pixbuf-query-loaders" "$RESOURCES"/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.dylib > "$CACHE/gdk-pixbuf.loaders"\n'
        '  export GDK_PIXBUF_MODULE_FILE="$CACHE/gdk-pixbuf.loaders"\n'
        'fi\n'
        'exec "$HERE/pop-fe-psp" "$@"\n',
        encoding="utf-8",
    )
    launcher.chmod(0o755)

    bundled_executables = [
        str(path)
        for path in macos.iterdir()
        if path.is_file() and path.name != "launch" and os.access(path, os.X_OK)
    ]
    loader_dir = resources / "lib/gdk-pixbuf-2.0/2.10.0/loaders"
    bundled_executables.extend(str(path) for path in loader_dir.glob("*.dylib"))
    bundled_executables.extend(str(path) for path in (resources / "ossl-modules").glob("*.dylib"))
    command = [dylibbundler, "-od", "-b", "-cd"]
    for executable in bundled_executables:
        command.extend(["-x", executable])
    command.extend(["-d", str(frameworks), "-p", "@executable_path/../Frameworks/"])
    library_paths = {prefix / "lib"}
    library_paths.update(path / "lib" for path in (prefix / "opt").glob("*"))
    for path in sorted(library_paths):
        if path.is_dir():
            command.extend(["-s", str(path)])
    subprocess.run(command, check=True)

    archive = output / f"pop-fe-psp-macos-{architecture}.zip"
    archive.unlink(missing_ok=True)
    subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(archive)], check=True)
    print(archive)


if __name__ == "__main__":
    main()
