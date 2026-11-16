#!/usr/bin/env python3
"""Build a Fedora RPM from the native Rust application and runtime tools."""

import argparse
import platform
import re
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

from deno_bundle import ensure_bundle as ensure_deno_bundle
from lcp_bundle import ensure_bundle as ensure_lcp_bundle
from yt_dlp_bundle import ensure_bundle as ensure_yt_dlp_bundle

ROOT = Path(__file__).resolve().parents[1]


def install(source: Path, target: Path, mode: int = 0o644) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    target.chmod(mode)


def build_encoder() -> Path:
    encoder = ROOT / "atracdenc/src/atracdenc"
    if encoder.is_file():
        return encoder
    # Keep Fedora RPM's encoder cache separate from other local packaging
    # builds, whose CMake cache may have been created from a different path.
    build_dir = ROOT / "target/atracdenc-rpm-build"
    subprocess.run(
        [
            "cmake", "-S", str(ROOT / "atracdenc/src"), "-B", str(build_dir),
            "-DCMAKE_BUILD_TYPE=Release",
            "-DCMAKE_POLICY_VERSION_MINIMUM=3.5",
            "-DCMAKE_CXX_FLAGS=-include stdint.h",
        ],
        check=True,
    )
    subprocess.run(["cmake", "--build", str(build_dir), "--parallel"], check=True)
    encoder = build_dir / "atracdenc"
    if not encoder.is_file():
        raise RuntimeError("CMake completed but did not produce the atracdenc executable")
    return encoder


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--version", default=(ROOT / "VERSION").read_text(encoding="utf-8").strip()
    )
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9A-Za-z.+~]+", args.version):
        parser.error("version contains characters that are not valid in an RPM version")
    if not shutil.which("rpmbuild"):
        parser.error("rpmbuild is required; install Fedora's rpm-build package")

    architecture = platform.machine().lower()
    architecture = {"amd64": "x86_64", "arm64": "aarch64"}.get(architecture, architecture)
    if architecture not in ("x86_64", "aarch64"):
        parser.error(f"unsupported RPM architecture: {architecture}")

    cargo = ["cargo", "build", "--release", "--locked", "--manifest-path", str(ROOT / "Cargo.toml")]
    if args.offline:
        cargo.append("--offline")
    subprocess.run(cargo, cwd=ROOT, check=True)
    binary = ROOT / "target/release/pop-fe-psp"
    if not binary.is_file():
        parser.error("Cargo did not produce target/release/pop-fe-psp")
    encoder = build_encoder()
    yt_dlp, yt_license, yt_third_party = ensure_yt_dlp_bundle(architecture, offline=args.offline)
    deno, deno_license = ensure_deno_bundle(architecture, offline=args.offline)
    lcp = ensure_lcp_bundle(architecture, offline=args.offline)

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pop-fe-rpm-") as temporary:
        temporary = Path(temporary)
        payload = temporary / "payload"
        app = payload / "usr/lib/pop-fe-psp"
        install(binary, app / "pop-fe-psp", 0o755)
        install(encoder, app / "atracdenc", 0o755)
        install(yt_dlp, app / "yt-dlp", 0o755)
        install(deno, app / "deno", 0o755)
        if lcp:
            install(lcp, app / "lcp", 0o755)
        install(ROOT / "packaging/pop-fe-psp", payload / "usr/bin/pop-fe-psp", 0o755)
        install(
            ROOT / "packaging/org.popfe.PSPManager.desktop",
            payload / "usr/share/applications/org.popfe.PSPManager.desktop",
        )
        shutil.copytree(ROOT / "packaging/icons/hicolor", payload / "usr/share/icons/hicolor")

        licenses = payload / "usr/share/licenses/pop-fe-psp"
        install(ROOT / "LICENCE-LGPL-2.1.txt", licenses / "LICENSE")
        install(
            ROOT / "licenses/libcrypt-patcher-BSD-3-Clause.txt",
            licenses / "libcrypt-patcher-BSD-3-Clause.txt",
        )
        install(ROOT / "licenses/lib-enigma-BSD-3-Clause.txt", licenses / "lib-enigma-BSD-3-Clause.txt")
        install(yt_license, licenses / "yt-dlp-LICENSE.txt")
        install(yt_third_party, licenses / "yt-dlp-THIRD_PARTY_LICENSES.txt")
        install(deno_license, licenses / "deno-LICENSE.md")

        topdir = temporary / "rpmbuild"
        for directory in ("BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"):
            (topdir / directory).mkdir(parents=True)
        archive = topdir / "SOURCES/pop-fe-psp-payload.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            for path in sorted(payload.rglob("*")):
                tar.add(path, arcname=path.relative_to(payload), recursive=False)

        spec = topdir / "SPECS/pop-fe-psp.spec"
        spec.write_text(
            f"""Name:           pop-fe-psp
%global debug_package %{{nil}}
Version:        {args.version}
Release:        1%{{?dist}}
Summary:        PlayStation disc image converter for PSP
License:        LGPL-2.1-or-later
URL:            https://github.com/marchrius/pop-fe
Source0:        pop-fe-psp-payload.tar.gz
Requires:       gtk4 >= 4.10
Requires:       libadwaita >= 1.5
Requires:       /usr/bin/xdelta3
Requires:       /usr/bin/chdman
Requires:       /usr/bin/pdftoppm
Requires:       /usr/bin/ffmpeg
Requires:       /usr/bin/7z

%description
Native Rust GTK application for converting and extracting PlayStation disc images
to and from PSP EBOOT.PBP files.

%prep
%setup -q -c -T

%build

%install
mkdir -p %{{buildroot}}
tar -xzf %{{SOURCE0}} -C %{{buildroot}}

%files
%license /usr/share/licenses/pop-fe-psp/*
/usr/bin/pop-fe-psp
/usr/lib/pop-fe-psp
/usr/share/applications/org.popfe.PSPManager.desktop
/usr/share/icons/hicolor
""",
            encoding="utf-8",
        )
        subprocess.run(
            ["rpmbuild", "--define", f"_topdir {topdir}", "-bb", str(spec)], check=True
        )
        packages = list((topdir / "RPMS").rglob("*.rpm"))
        if len(packages) != 1:
            raise RuntimeError(f"rpmbuild produced {len(packages)} binary RPM packages")
        destination = output / packages[0].name
        shutil.copy2(packages[0], destination)
    print(destination)


if __name__ == "__main__":
    main()
